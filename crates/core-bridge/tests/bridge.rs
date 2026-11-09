use std::{path::PathBuf, process::Stdio, time::Duration};

use herdr_hub_core_bridge::{BridgeError, HcBridge};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(15);
const HELPER: &str = env!("CARGO_BIN_EXE_herdr-hub-test-hc");
const AGENT: &str = env!("CARGO_BIN_EXE_herdr-hub-agent");

fn bridge(session: &str) -> HcBridge {
    HcBridge::new(PathBuf::from(HELPER), session.to_owned()).unwrap()
}

fn bootstrap(session: &str) -> String {
    format!(
        "sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; [ -x \"$p\" ] || {{ echo \"herdr-core is not installed here\" >&2; exit 127; }}; HC_SESSION={session} exec \"$p\" connect'"
    )
}

#[tokio::test]
async fn binary_copy_preserves_backpressure_and_eof_response() {
    let (mut sender, input) = tokio::io::duplex(128);
    let (output, mut receiver) = tokio::io::duplex(128);
    let task = tokio::spawn(async move {
        bridge("echo")
            .run(input, output, CancellationToken::new())
            .await
    });
    let payload: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();
    let send = async {
        sender.write_all(&payload).await.unwrap();
        sender.shutdown().await.unwrap();
    };
    let receive = async {
        let mut actual = Vec::new();
        receiver.read_to_end(&mut actual).await.unwrap();
        let mut expected = payload.clone();
        expected.extend_from_slice(b"after-eof");
        assert_eq!(actual, expected);
    };
    timeout(DEADLINE, async { tokio::join!(send, receive) })
        .await
        .unwrap();
    timeout(DEADLINE, task).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn output_eof_finishes_even_if_input_is_still_open() {
    let (_sender, input) = tokio::io::duplex(64);
    let (output, mut receiver) = tokio::io::duplex(64);
    let result = timeout(
        DEADLINE,
        bridge("closed").run(input, output, CancellationToken::new()),
    )
    .await
    .unwrap();
    result.unwrap();
    let mut actual = Vec::new();
    receiver.read_to_end(&mut actual).await.unwrap();
    assert_eq!(actual, b"done");
}

#[tokio::test]
async fn cancellation_reaps_a_child_that_will_not_consume_input() {
    let (mut sender, input) = tokio::io::duplex(64);
    let (output, mut receiver) = tokio::io::duplex(64);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move { bridge("blocked").run(input, output, token).await });
    let mut ready = [0; 5];
    timeout(DEADLINE, receiver.read_exact(&mut ready))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&ready, b"ready");
    let payload = vec![0x5a; 2_097_152];
    let write = sender.write_all(&payload);
    tokio::pin!(write);
    assert!(
        timeout(Duration::from_millis(200), &mut write)
            .await
            .is_err()
    );
    cancel.cancel();
    assert!(matches!(
        timeout(DEADLINE, task).await.unwrap().unwrap(),
        Err(BridgeError::Cancelled)
    ));
    assert_eq!(
        timeout(DEADLINE, receiver.read(&mut ready))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn child_failure_is_reported_after_draining_its_output() {
    let (_sender, input) = tokio::io::duplex(64);
    let (output, mut receiver) = tokio::io::duplex(64);
    let result = timeout(
        DEADLINE,
        bridge("fail").run(input, output, CancellationToken::new()),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(BridgeError::ChildExit(status)) if status.code() == Some(17)));
    let mut actual = Vec::new();
    receiver.read_to_end(&mut actual).await.unwrap();
    assert_eq!(actual, b"before-failure");
}

#[tokio::test]
async fn cli_keeps_diagnostics_off_protocol_stdout_and_exits_with_open_stdin() {
    let mut child = Command::new(AGENT)
        .args([
            "local-bridge",
            "--hc",
            HELPER,
            "--session",
            "closed",
            "--",
            &bootstrap("closed"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let _kept_open = child.stdin.take().unwrap();
    let output = timeout(DEADLINE, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"done");
    assert!(String::from_utf8_lossy(&output.stderr).contains("test fixture diagnostic"));
}

#[tokio::test]
async fn cli_rejects_unknown_bootstrap_before_spawning_hc() {
    let output = Command::new(AGENT)
        .args([
            "local-bridge",
            "--hc",
            HELPER,
            "--session",
            "closed",
            "--",
            "echo injected",
        ])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("unsupported hc bootstrap"));
    assert!(!diagnostic.contains("test fixture diagnostic"));
}
