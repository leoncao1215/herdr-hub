use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, NetReportConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::{Builder, PortmapperConfig, QuicTransportConfig},
    tls::CaTlsConfig,
};
use iroh_relay::{
    RelayConfig,
    server::{Access, AccessControl, ClientRequest, ConnectionId, Server, ServerConfig},
};
use tokio::{
    sync::{broadcast, oneshot},
    time::timeout,
};

mod support;

const ALPN: &[u8] = b"herdr-hub/g1-probe/1";
const DEADLINE: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Allowed(EndpointId),
    Denied(EndpointId),
    Disconnected(EndpointId),
}

#[derive(Debug)]
struct Gate {
    allowed: RwLock<HashSet<EndpointId>>,
    events: broadcast::Sender<Event>,
}

impl Gate {
    fn new(ids: impl IntoIterator<Item = EndpointId>) -> Self {
        Self {
            allowed: RwLock::new(ids.into_iter().collect()),
            events: broadcast::channel(64).0,
        }
    }

    fn revoke(&self, id: EndpointId) {
        self.allowed.write().expect("gate lock").remove(&id);
    }
}

impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = request.endpoint_id();
        if self.allowed.read().expect("gate lock").contains(&id) {
            let _ = self.events.send(Event::Allowed(id));
            Access::Allow
        } else {
            let _ = self.events.send(Event::Denied(id));
            Access::Deny { reason: None }
        }
    }

    fn on_disconnect(&self, id: EndpointId, _connection_id: ConnectionId) {
        let _ = self.events.send(Event::Disconnected(id));
    }
}

async fn event(events: &mut broadcast::Receiver<Event>, expected: Event) -> Result<()> {
    timeout(DEADLINE, async {
        loop {
            if events.recv().await? == expected {
                return Ok(());
            }
        }
    })
    .await
    .context("relay event deadline")?
}

fn base_builder() -> Builder {
    let mut reports = NetReportConfig::default();
    reports.captive_portal_check = false;
    Builder::empty()
        .crypto_provider(iroh_relay::tls::default_provider())
        .net_report_config(reports)
        .portmapper_config(PortmapperConfig::Disabled)
        .transport_config(
            QuicTransportConfig::builder()
                .stream_receive_window(16_384u32.into())
                .receive_window(32_768u32.into())
                .send_window(32_768)
                .build(),
        )
        .alpns(vec![ALPN.to_vec()])
}

async fn direct_endpoint() -> Result<Endpoint> {
    Ok(base_builder()
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")?
        .bind()
        .await?)
}

async fn relay_endpoint(key: SecretKey, url: RelayUrl) -> Result<Endpoint> {
    relay_endpoint_with_ca(key, url, CaTlsConfig::default()).await
}

async fn relay_endpoint_with_ca(
    key: SecretKey,
    url: RelayUrl,
    ca: CaTlsConfig,
) -> Result<Endpoint> {
    relay_endpoint_with_options(key, url, ca, None).await
}

async fn relay_endpoint_with_options(
    key: SecretKey,
    url: RelayUrl,
    ca: CaTlsConfig,
    scope: Option<&str>,
) -> Result<Endpoint> {
    let mut relay = RelayConfig::new(url, None);
    if let Some(scope) = scope {
        relay = relay.with_auth_token(scope);
    }
    let map = RelayMap::from_iter([relay]);
    Ok(base_builder()
        .clear_ip_transports()
        .secret_key(key)
        .relay_mode(RelayMode::Custom(map))
        .ca_tls_config(ca)
        .bind()
        .await?)
}

async fn relay(gate: Arc<Gate>) -> Result<(Server, RelayUrl)> {
    let mut relay_config =
        iroh_relay::server::RelayConfig::new("127.0.0.1:0".parse::<std::net::SocketAddr>()?);
    relay_config.access = gate;
    let mut config = ServerConfig::default();
    config.relay = Some(relay_config);
    config.quic = None;
    let server = Server::spawn(config).await?;
    let url = format!(
        "http://{}",
        server.http_addr().context("relay HTTP address")?
    )
    .parse()?;
    ensure!(server.quic_addr().is_none(), "unexpected QAD listener");
    Ok((server, url))
}

async fn roundtrip(client: &Endpoint, host: &Endpoint, address: EndpointAddr) -> Result<()> {
    let payload: Vec<u8> = (0..1_048_576).map(|i| (i % 251) as u8).collect();
    let request = payload.clone();
    let accept = async {
        let connection = host.accept().await.context("endpoint closed")?.await?;
        ensure!(
            connection.remote_id() == client.id(),
            "wrong client identity"
        );
        let (mut send, mut receive) = connection.accept_bi().await?;
        let received = receive.read_to_end(request.len()).await?;
        ensure!(received == request, "request corrupted");
        // Respond only after EOF, proving that half-close leaves the reverse direction open.
        send.write_all(&received).await?;
        send.write_all(b"after-eof").await?;
        send.finish()?;
        connection.closed().await;
        Ok::<_, anyhow::Error>(())
    };
    let connect = async {
        let connection = client.connect(address, ALPN).await?;
        ensure!(connection.remote_id() == host.id(), "wrong host identity");
        let (mut send, mut receive) = connection.open_bi().await?;
        send.write_all(&payload).await?;
        send.finish()?;
        let reply = receive.read_to_end(payload.len() + 9).await?;
        ensure!(reply.len() == payload.len() + 9, "response length mismatch");
        ensure!(reply[..payload.len()] == payload, "response corrupted");
        ensure!(
            &reply[payload.len()..] == b"after-eof",
            "missing EOF response"
        );
        eprintln!("transfer: 1048576 request bytes; authenticated peers; EOF response received");
        connection.close(0u32.into(), b"probe complete");
        Ok::<_, anyhow::Error>(())
    };
    timeout(DEADLINE, async { tokio::try_join!(accept, connect) })
        .await
        .context("roundtrip deadline")??;
    Ok(())
}

#[tokio::test]
async fn direct_binary_stream_survives_directional_eof() -> Result<()> {
    let client = direct_endpoint().await?;
    let host = direct_endpoint().await?;
    let result = async {
        let address = host.addr();
        ensure!(address.relay_urls().next().is_none(), "unexpected relay");
        ensure!(
            address.ip_addrs().next().is_some()
                && address.ip_addrs().all(|addr| addr.ip().is_loopback()),
            "expected exclusively loopback candidates"
        );
        roundtrip(&client, &host, address).await
    }
    .await;
    tokio::join!(client.close(), host.close());
    result
}

#[tokio::test]
async fn relay_only_binary_stream_survives_directional_eof() -> Result<()> {
    let client_key = SecretKey::generate();
    let host_key = SecretKey::generate();
    let gate = Arc::new(Gate::new([client_key.public(), host_key.public()]));
    let mut events = gate.events.subscribe();
    let (server, url) = relay(gate).await?;
    let client = relay_endpoint(client_key, url.clone()).await?;
    let host = relay_endpoint(host_key, url.clone()).await?;
    let result = async {
        event(&mut events, Event::Allowed(host.id())).await?;
        ensure!(
            host.addr().ip_addrs().next().is_none(),
            "host has IP transport"
        );
        ensure!(
            client.addr().ip_addrs().next().is_none(),
            "client has IP transport"
        );
        roundtrip(
            &client,
            &host,
            EndpointAddr::new(host.id()).with_relay_url(url),
        )
        .await
    }
    .await;
    tokio::join!(client.close(), host.close());
    server.shutdown().await?;
    result
}

#[tokio::test]
async fn relay_rejects_an_unapproved_identity() -> Result<()> {
    let gate = Arc::new(Gate::new([]));
    let mut events = gate.events.subscribe();
    let (server, url) = relay(gate).await?;
    let client = relay_endpoint(SecretKey::generate(), url).await?;
    let result = async {
        event(&mut events, Event::Denied(client.id())).await?;
        ensure!(
            timeout(Duration::from_millis(500), client.online())
                .await
                .is_err(),
            "rejected endpoint became online"
        );
        Ok(())
    }
    .await;
    client.close().await;
    server.shutdown().await?;
    result
}

#[tokio::test]
async fn relay_revocation_disconnects_and_rejects_reconnection() -> Result<()> {
    let key = SecretKey::generate();
    let id = key.public();
    let gate = Arc::new(Gate::new([id]));
    let mut events = gate.events.subscribe();
    let (server, url) = relay(gate.clone()).await?;
    let client = relay_endpoint(key, url).await?;
    let result = async {
        event(&mut events, Event::Allowed(id)).await?;
        timeout(DEADLINE, client.online()).await?;
        gate.revoke(id);
        ensure!(
            server
                .relay_service()
                .context("relay service")?
                .clients()
                .disconnect(id, None),
            "approved connection was not registered"
        );
        event(&mut events, Event::Disconnected(id)).await?;
        event(&mut events, Event::Denied(id)).await?;
        ensure!(
            timeout(Duration::from_millis(500), client.online())
                .await
                .is_err(),
            "revoked endpoint reconnected"
        );
        Ok(())
    }
    .await;
    client.close().await;
    server.shutdown().await?;
    result
}

#[tokio::test]
async fn write_blocks_until_receiver_consumes_data() -> Result<()> {
    let client = direct_endpoint().await?;
    let host = direct_endpoint().await?;
    let (release, wait_for_release) = oneshot::channel();
    let payload = vec![0xa5; 1_048_576];
    let accept = async {
        let connection = host.accept().await.context("endpoint closed")?.await?;
        let (mut send, mut receive) = connection.accept_bi().await?;
        wait_for_release.await?;
        let received = receive.read_to_end(payload.len()).await?;
        ensure!(received == payload, "flow-controlled transfer corrupted");
        send.write_all(b"received").await?;
        send.finish()?;
        connection.closed().await;
        Ok::<_, anyhow::Error>(())
    };
    let connect = async {
        let connection = client.connect(host.addr(), ALPN).await?;
        let (mut send, mut receive) = connection.open_bi().await?;
        {
            let write = send.write_all(&payload);
            tokio::pin!(write);
            ensure!(
                timeout(Duration::from_millis(250), &mut write)
                    .await
                    .is_err(),
                "write completed despite receiver withholding credit"
            );
            release
                .send(())
                .map_err(|_| anyhow::anyhow!("receiver exited"))?;
            write.await?;
        }
        send.finish()?;
        ensure!(
            receive.read_to_end(8).await? == b"received",
            "missing receipt"
        );
        connection.close(0u32.into(), b"flow control complete");
        Ok::<_, anyhow::Error>(())
    };
    let result = timeout(DEADLINE, async { tokio::try_join!(accept, connect) }).await;
    tokio::join!(client.close(), host.close());
    result.context("flow-control deadline")??;
    Ok(())
}

#[tokio::test]
async fn relay_works_behind_tls_termination_without_skipping_certificate_checks() -> Result<()> {
    let client_key = SecretKey::generate();
    let host_key = SecretKey::generate();
    let gate = Arc::new(Gate::new([client_key.public(), host_key.public()]));
    let mut events = gate.events.subscribe();
    let (server, _) = relay(gate).await?;
    let proxy = support::TlsProxy::start(server.http_addr().context("relay address")?).await?;
    let client = relay_endpoint_with_ca(client_key, proxy.url.clone(), proxy.ca.clone()).await?;
    let host = relay_endpoint_with_ca(host_key, proxy.url.clone(), proxy.ca.clone()).await?;
    let result = async {
        event(&mut events, Event::Allowed(host.id())).await?;
        roundtrip(
            &client,
            &host,
            EndpointAddr::new(host.id()).with_relay_url(proxy.url.clone()),
        )
        .await
    }
    .await;
    tokio::join!(client.close(), host.close());
    let proxy_cleanup = proxy.shutdown().await;
    let server_cleanup = server.shutdown().await.map_err(anyhow::Error::from);
    result.and(proxy_cleanup).and(server_cleanup)
}

#[tokio::test]
async fn scoped_relay_routes_do_not_admit_another_pairs_endpoint() -> Result<()> {
    let client_key = SecretKey::generate();
    let host_key = SecretKey::generate();
    let other_key = SecretKey::generate();
    let other_client_key = SecretKey::generate();
    let first_gate = Arc::new(Gate::new([client_key.public(), host_key.public()]));
    let second_gate = Arc::new(Gate::new([other_key.public(), other_client_key.public()]));
    let mut accepted = first_gate.events.subscribe();
    let mut rejected = second_gate.events.subscribe();
    let router = support::ScopedRelays::start(first_gate, second_gate).await?;
    let http_proxy = support::http_proxy::HttpProxy::start(router.address).await?;
    let proxy = support::TlsProxy::start(http_proxy.address).await?;
    let client = relay_endpoint_with_options(
        client_key.clone(),
        proxy.url.clone(),
        proxy.ca.clone(),
        Some("pair-a"),
    )
    .await?;
    let host = relay_endpoint_with_options(
        host_key,
        proxy.url.clone(),
        proxy.ca.clone(),
        Some("pair-a"),
    )
    .await?;
    let intruder = relay_endpoint_with_options(
        client_key,
        proxy.url.clone(),
        proxy.ca.clone(),
        Some("pair-b"),
    )
    .await?;
    let other_host = relay_endpoint_with_options(
        other_key,
        proxy.url.clone(),
        proxy.ca.clone(),
        Some("pair-b"),
    )
    .await?;
    let other_client = relay_endpoint_with_options(
        other_client_key,
        proxy.url.clone(),
        proxy.ca.clone(),
        Some("pair-b"),
    )
    .await?;
    let result = async {
        event(&mut accepted, Event::Allowed(host.id()))
            .await
            .context("pair A host admission")?;
        event(&mut rejected, Event::Denied(intruder.id()))
            .await
            .context("pair B cross-pair rejection")?;
        ensure!(
            timeout(Duration::from_millis(500), intruder.online())
                .await
                .is_err(),
            "cross-pair endpoint became online"
        );
        timeout(DEADLINE, other_host.online()).await?;
        tokio::try_join!(
            roundtrip(
                &client,
                &host,
                EndpointAddr::new(host.id()).with_relay_url(proxy.url.clone()),
            ),
            roundtrip(
                &other_client,
                &other_host,
                EndpointAddr::new(other_host.id()).with_relay_url(proxy.url.clone()),
            ),
        )?;
        let crossed = async {
            tokio::try_join!(
                async {
                    Ok::<_, anyhow::Error>(
                        client
                            .connect(
                                EndpointAddr::new(other_host.id())
                                    .with_relay_url(proxy.url.clone()),
                                ALPN,
                            )
                            .await?,
                    )
                },
                async {
                    Ok::<_, anyhow::Error>(
                        other_host
                            .accept()
                            .await
                            .context("other host closed")?
                            .await?,
                    )
                },
            )
        };
        ensure!(
            timeout(Duration::from_millis(500), crossed).await.is_err(),
            "a connection crossed isolated relay registries"
        );
        Ok(())
    }
    .await;
    tokio::join!(
        client.close(),
        host.close(),
        intruder.close(),
        other_client.close(),
        other_host.close()
    );
    let tls_cleanup = proxy.shutdown().await;
    let http_cleanup = http_proxy.shutdown().await;
    let router_cleanup = router.shutdown().await;
    result
        .and(tls_cleanup)
        .and(http_cleanup)
        .and(router_cleanup)
}
