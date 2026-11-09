#![forbid(unsafe_code)]

use std::{io, path::PathBuf, process::Stdio, time::Duration};

use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, ChildStdin, Command},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const PREFIX: &str = "sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; [ -x \"$p\" ] || { echo \"herdr-core is not installed here\" >&2; exit 127; }; ";
const SUFFIX: &str = "exec \"$p\" connect'";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("invalid hc session name")]
    InvalidSession,
    #[error("the configured hc executable must be an absolute path")]
    InvalidExecutable,
    #[error("unsupported hc bootstrap or session mismatch; no shell command was executed")]
    UnsupportedBootstrap,
    #[error("hc bridge I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("hc connect exited unsuccessfully: {0}")]
    ChildExit(std::process::ExitStatus),
    #[error("hc connect did not exit within the cleanup deadline")]
    ChildTimeout,
    #[error("hc bridge cancelled")]
    Cancelled,
}

pub fn validate_session(session: &str) -> Result<(), BridgeError> {
    let first = session.bytes().next().ok_or(BridgeError::InvalidSession)?;
    if session.len() > 64
        || !(first.is_ascii_alphanumeric() || first == b'_')
        || !session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    {
        return Err(BridgeError::InvalidSession);
    }
    Ok(())
}

pub fn validate_bootstrap(line: &str, session: &str) -> Result<(), BridgeError> {
    validate_session(session)?;
    if line.len() > 4096 {
        return Err(BridgeError::UnsupportedBootstrap);
    }
    if session == "default" && line == format!("{PREFIX}{SUFFIX}") {
        return Ok(());
    }
    if line == format!("{PREFIX}HC_SESSION={session} {SUFFIX}") {
        return Ok(());
    }
    Err(BridgeError::UnsupportedBootstrap)
}

#[derive(Debug, Clone)]
pub struct HcBridge {
    executable: PathBuf,
    session: String,
}

impl HcBridge {
    pub fn new(executable: PathBuf, session: String) -> Result<Self, BridgeError> {
        validate_session(&session)?;
        if !executable.is_absolute() {
            return Err(BridgeError::InvalidExecutable);
        }
        Ok(Self {
            executable,
            session,
        })
    }

    pub async fn run<R, W>(
        &self,
        input: R,
        mut output: W,
        cancellation: CancellationToken,
    ) -> Result<(), BridgeError>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        if cancellation.is_cancelled() {
            return Err(BridgeError::Cancelled);
        }
        let mut command = Command::new(&self.executable);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command
            .args(["connect", "--session", &self.session])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let child_input = child.stdin.take().expect("piped stdin");
        let mut child_output = child.stdout.take().expect("piped stdout");

        let transfer = {
            let upstream = copy_to_hc(input, child_input);
            let downstream = async move {
                tokio::io::copy(&mut child_output, &mut output).await?;
                output.shutdown().await?;
                Ok::<_, BridgeError>(())
            };
            tokio::pin!(upstream, downstream);
            tokio::select! {
                _ = cancellation.cancelled() => Err(BridgeError::Cancelled),
                result = &mut downstream => result,
                result = &mut upstream => match result {
                    Err(error) => Err(error),
                    Ok(()) => tokio::select! {
                        _ = cancellation.cancelled() => Err(BridgeError::Cancelled),
                        result = &mut downstream => result,
                    },
                },
            }
        };
        let cleanup = reap(&mut child, transfer.is_err()).await;
        transfer?;
        cleanup
    }
}

async fn copy_to_hc<R: AsyncRead + Unpin>(
    mut input: R,
    mut child_input: ChildStdin,
) -> Result<(), BridgeError> {
    let mut buffer = [0; 8192];
    loop {
        let read = input.read(&mut buffer).await?;
        let result = if read == 0 {
            child_input.shutdown().await
        } else {
            child_input.write_all(&buffer[..read]).await
        };
        match result {
            // hc may close stdin before its final stdout response has drained.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(()) if read == 0 => return Ok(()),
            Ok(()) => {}
        }
    }
}

async fn reap(child: &mut Child, force: bool) -> Result<(), BridgeError> {
    if force {
        let _ = child.start_kill();
    }
    match timeout(CLEANUP_TIMEOUT, child.wait()).await {
        Ok(status) => {
            let status = status?;
            if status.success() {
                Ok(())
            } else {
                Err(BridgeError::ChildExit(status))
            }
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = timeout(CLEANUP_TIMEOUT, child.wait()).await;
            Err(BridgeError::ChildTimeout)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_captured_hc_bootstraps() {
        let captures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../experiments/transport-probe/fixtures/hc-0.1.19-wsl2-bootstrap.json"
        ))
        .unwrap();
        for case in captures["cases"].as_array().unwrap() {
            validate_bootstrap(
                case["connector_arguments"][0].as_str().unwrap(),
                case["remote_session"].as_str().unwrap_or("default"),
            )
            .unwrap();
        }
        validate_bootstrap(&format!("{PREFIX}HC_SESSION=default {SUFFIX}"), "default").unwrap();
    }

    #[test]
    fn rejects_changed_commands_and_mismatched_sessions() {
        let valid = format!("{PREFIX}{SUFFIX}");
        for invalid in [
            format!("{valid}; echo injected"),
            format!("{valid}\n"),
            valid.replace(" connect'", " update'"),
            valid.replace("exec \"$p\"", "exec sh"),
            format!("{PREFIX}HC_SESSION=other {SUFFIX}"),
            "hc connect".to_owned(),
        ] {
            assert!(matches!(
                validate_bootstrap(&invalid, "default"),
                Err(BridgeError::UnsupportedBootstrap)
            ));
        }
        assert!(validate_bootstrap(&valid, "named").is_err());
    }

    #[test]
    fn rejects_session_values_that_can_change_arguments_or_paths() {
        for invalid in [
            "", "--help", ".", "..", "a/b", "a\\b", "x;id", "$(id)", "a b", "\u{4e2d}",
        ] {
            assert!(validate_session(invalid).is_err(), "{invalid:?}");
        }
        assert!(validate_session(&"a".repeat(65)).is_err());
    }
}
