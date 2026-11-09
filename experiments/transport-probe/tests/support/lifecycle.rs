use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, endpoint::QuicTransportConfig,
};
use iroh_relay::RelayConfig;
use tokio::{sync::oneshot, time::timeout};

use crate::{ALPN, DEADLINE, Gate, base_builder, direct_endpoint, relay};

async fn interrupted_transfer(local_cancellation: bool) -> Result<()> {
    let client = direct_endpoint().await?;
    let host = direct_endpoint().await?;
    let (release, released) = oneshot::channel();
    let accept = async {
        let connection = host.accept().await.context("host closed")?.await?;
        let (mut send, _receive) = connection.accept_bi().await?;
        send.write_all(b"ready").await?;
        released.await?;
        if !local_cancellation {
            connection.close(1u32.into(), b"simulated peer termination");
        }
        connection.closed().await;
        Ok::<_, anyhow::Error>(())
    };
    let connect = async {
        let connection = client.connect(host.addr(), ALPN).await?;
        let (mut send, mut receive) = connection.open_bi().await?;
        send.write_all(b"x").await?;
        let mut ready = [0; 5];
        receive.read_exact(&mut ready).await?;
        ensure!(&ready == b"ready", "peer did not complete setup");

        let payload = vec![0x5a; 1_048_576];
        let write = send.write_all(&payload);
        tokio::pin!(write);
        ensure!(
            timeout(Duration::from_millis(250), &mut write)
                .await
                .is_err(),
            "write was not flow-controlled"
        );
        let mut byte = [0];
        let read = receive.read_exact(&mut byte);
        tokio::pin!(read);
        ensure!(
            timeout(Duration::from_millis(250), &mut read)
                .await
                .is_err(),
            "read was not pending"
        );
        if local_cancellation {
            connection.close(1u32.into(), b"local cancellation");
        }
        release
            .send(())
            .map_err(|_| anyhow::anyhow!("peer exited early"))?;
        let (write_result, read_result) =
            timeout(DEADLINE, async { tokio::join!(write, read) }).await?;
        ensure!(
            write_result.is_err(),
            "pending write reported success after closure"
        );
        ensure!(
            read_result.is_err(),
            "pending read reported success after closure"
        );
        Ok::<_, anyhow::Error>(())
    };
    let result = timeout(DEADLINE, async { tokio::try_join!(accept, connect) }).await;
    let cleanup = timeout(DEADLINE, async {
        tokio::join!(client.close(), host.close())
    })
    .await
    .context("endpoint cleanup deadline")
    .map(|_| ());
    result
        .context("interrupted transfer deadline")
        .and_then(|result| result.map(|_| ()))
        .and(cleanup)
}

#[tokio::test]
async fn local_cancellation_releases_blocked_read_and_write() -> Result<()> {
    interrupted_transfer(true).await
}

#[tokio::test]
async fn peer_termination_releases_blocked_read_and_write() -> Result<()> {
    interrupted_transfer(false).await
}

async fn short_idle_relay_endpoint(key: SecretKey, url: RelayUrl) -> Result<Endpoint> {
    Ok(base_builder()
        .clear_ip_transports()
        .secret_key(key)
        .relay_mode(RelayMode::Custom(RelayMap::from_iter([RelayConfig::new(
            url, None,
        )])))
        .transport_config(
            QuicTransportConfig::builder()
                .stream_receive_window(16_384u32.into())
                .receive_window(32_768u32.into())
                .send_window(32_768)
                .max_idle_timeout(Some(Duration::from_secs(3).try_into()?))
                .build(),
        )
        .bind()
        .await?)
}

#[tokio::test]
async fn relay_loss_times_out_pending_io_without_closing_the_peer() -> Result<()> {
    let client_key = SecretKey::generate();
    let host_key = SecretKey::generate();
    let gate = Arc::new(Gate::new([client_key.public(), host_key.public()]));
    let (running, url) = relay(gate).await?;
    let mut server = Some(running);
    let client = short_idle_relay_endpoint(client_key, url.clone()).await?;
    let host = short_idle_relay_endpoint(host_key, url.clone()).await?;
    let accept = async {
        let connection = host.accept().await.context("host closed")?.await?;
        let (mut send, _receive) = connection.accept_bi().await?;
        send.write_all(b"ready").await?;
        connection.closed().await;
        Ok::<_, anyhow::Error>(())
    };
    let connect = async {
        timeout(DEADLINE, host.online()).await?;
        let connection = client
            .connect(EndpointAddr::new(host.id()).with_relay_url(url), ALPN)
            .await?;
        let (mut send, mut receive) = connection.open_bi().await?;
        send.write_all(b"x").await?;
        let mut ready = [0; 5];
        receive.read_exact(&mut ready).await?;
        ensure!(&ready == b"ready", "relay path was not established");
        let payload = vec![0x5a; 1_048_576];
        let write = send.write_all(&payload);
        tokio::pin!(write);
        ensure!(
            timeout(Duration::from_millis(250), &mut write)
                .await
                .is_err(),
            "write was not pending"
        );
        server.take().context("missing relay")?.shutdown().await?;
        let mut byte = [0];
        let (written, read) = timeout(DEADLINE, async {
            tokio::join!(write, receive.read_exact(&mut byte))
        })
        .await?;
        ensure!(
            written.is_err() && read.is_err(),
            "lost relay left I/O successful"
        );
        Ok::<_, anyhow::Error>(())
    };
    let result = timeout(DEADLINE, async { tokio::try_join!(accept, connect) }).await;
    let endpoints = timeout(DEADLINE, async {
        tokio::join!(client.close(), host.close())
    })
    .await;
    let relay_cleanup = if let Some(server) = server {
        server.shutdown().await.map_err(anyhow::Error::from)
    } else {
        Ok(())
    };
    result
        .context("relay loss deadline")
        .and_then(|result| result.map(|_| ()))
        .and(endpoints.context("endpoint cleanup deadline").map(|_| ()))
        .and(relay_cleanup)
}
