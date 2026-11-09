use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::{Parser, Subcommand};
use herdr_hub_core_bridge::{HcBridge, validate_bootstrap};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "herdr-hub-agent",
    version,
    about = "Herdr Hub native connection components"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Local-only development connector; does not establish a remote network connection.
    LocalBridge {
        #[arg(long)]
        hc: PathBuf,
        #[arg(long, default_value = "default")]
        session: String,
        #[arg(last = true)]
        bootstrap: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("herdr-hub-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let Action::LocalBridge {
        hc,
        session,
        bootstrap,
    } = cli.command;
    validate_bootstrap(&bootstrap, &session)?;
    let bridge = HcBridge::new(hc, session)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let cancellation = CancellationToken::new();
        let transfer = bridge.run(
            tokio::io::stdin(),
            tokio::io::stdout(),
            cancellation.clone(),
        );
        tokio::pin!(transfer);
        tokio::select! {
            result = &mut transfer => result,
            _ = interrupted() => {
                cancellation.cancel();
                transfer.await
            },
        }
    });
    // Tokio's blocking stdin read cannot be cancelled when the other end closes first.
    runtime.shutdown_timeout(Duration::from_millis(250));
    result?;
    Ok(())
}

async fn interrupted() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            return;
        }
    }
    ctrl_c().await;
}

async fn ctrl_c() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}
