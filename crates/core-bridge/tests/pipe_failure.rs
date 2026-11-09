use std::{
    io,
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use herdr_hub_core_bridge::{BridgeError, HcBridge};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

const HELPER: &str = env!("CARGO_BIN_EXE_herdr-hub-test-hc");
const DEADLINE: Duration = Duration::from_secs(15);

#[tokio::test]
async fn closed_child_input_does_not_truncate_its_final_response() {
    let (mut sender, input) = tokio::io::duplex(64);
    let (output, mut receiver) = tokio::io::duplex(4);
    let task = tokio::spawn(async move {
        HcBridge::new(PathBuf::from(HELPER), "fail".to_owned())
            .unwrap()
            .run(input, output, CancellationToken::new())
            .await
    });
    let payload = vec![0x5a; 2_097_152];
    assert!(
        timeout(DEADLINE, sender.write_all(&payload))
            .await
            .unwrap()
            .is_err()
    );
    let mut response = Vec::new();
    timeout(DEADLINE, receiver.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response, b"before-failure");
    assert!(matches!(
        timeout(DEADLINE, task).await.unwrap().unwrap(),
        Err(BridgeError::ChildExit(status)) if status.code() == Some(17)
    ));
}

#[tokio::test]
async fn cancellation_remains_available_while_draining_a_closed_child() {
    let (mut sender, input) = tokio::io::duplex(64);
    let (output, _receiver) = tokio::io::duplex(4);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        HcBridge::new(PathBuf::from(HELPER), "fail".to_owned())
            .unwrap()
            .run(input, output, token)
            .await
    });
    let payload = vec![0x5a; 2_097_152];
    assert!(
        timeout(DEADLINE, sender.write_all(&payload))
            .await
            .unwrap()
            .is_err()
    );
    cancel.cancel();
    assert!(matches!(
        timeout(DEADLINE, task).await.unwrap().unwrap(),
        Err(BridgeError::Cancelled)
    ));
}

struct BrokenInput;

impl AsyncRead for BrokenInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)))
    }
}

#[tokio::test]
async fn broken_input_reader_is_not_mistaken_for_closed_child_stdin() {
    let (output, _receiver) = tokio::io::duplex(64);
    let bridge = HcBridge::new(PathBuf::from(HELPER), "blocked".to_owned()).unwrap();
    let result = timeout(
        DEADLINE,
        bridge.run(BrokenInput, output, CancellationToken::new()),
    )
    .await
    .unwrap();
    assert!(
        matches!(result, Err(BridgeError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe)
    );
}
