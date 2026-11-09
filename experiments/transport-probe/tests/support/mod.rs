use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{
    Request, Response,
    body::Incoming,
    http::HeaderMap,
    server::conn::http1,
    service::{Service, service_fn},
};
use hyper_util::rt::TokioIo;
use iroh::{RelayUrl, tls::CaTlsConfig};
use iroh_relay::{
    KeyCache,
    server::{
        Metrics,
        http_server::{BytesBody, Handlers, RelayService, RelayServiceWithNotify},
        streams::MaybeTlsStream,
    },
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Notify, oneshot},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_rustls::TlsAcceptor;

use super::{DEADLINE, Gate};

pub mod http_proxy;
mod lifecycle;

struct Background {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<()>>,
}

impl Background {
    async fn shutdown(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        timeout(DEADLINE, &mut self.task).await???;
        Ok(())
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct TlsProxy {
    pub url: RelayUrl,
    pub ca: CaTlsConfig,
    background: Background,
}

impl TlsProxy {
    pub async fn start(upstream: SocketAddr) -> Result<Self> {
        let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()])?;
        let der = certificate.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der());
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![der.clone()], key.into())?;
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("https://{}", listener.local_addr()?).parse()?;
        let (stop, mut shutdown) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    completed = connections.join_next(), if !connections.is_empty() => {
                        if let Err(error) = completed.expect("nonempty tasks")? {
                            eprintln!("loopback TLS connection ended: {error}");
                        }
                    }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted?;
                        let acceptor = acceptor.clone();
                        connections.spawn(async move {
                            let mut tls = timeout(DEADLINE, acceptor.accept(socket)).await??;
                            let mut upstream = TcpStream::connect(upstream).await?;
                            tokio::io::copy_bidirectional(&mut tls, &mut upstream).await?;
                            Ok::<_, anyhow::Error>(())
                        });
                    }
                }
            }
            connections.shutdown().await;
            Ok(())
        });
        Ok(Self {
            url,
            ca: CaTlsConfig::custom_roots([der]),
            background: Background {
                stop: Some(stop),
                task,
            },
        })
    }

    pub async fn shutdown(self) -> Result<()> {
        self.background.shutdown().await
    }
}

pub struct ScopedRelays {
    pub address: SocketAddr,
    services: [RelayService; 2],
    background: Background,
}

impl ScopedRelays {
    pub async fn start(first: Arc<Gate>, second: Arc<Gate>) -> Result<Self> {
        let services = [first, second].map(|gate| {
            RelayService::new(
                Handlers::default(),
                HeaderMap::new(),
                None,
                KeyCache::new(32),
                gate,
                Arc::new(Metrics::default()),
            )
        });
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let routes = services.clone();
        let (stop, mut shutdown) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown => break,
                    completed = connections.join_next(), if !connections.is_empty() => {
                        if let Err(error) = completed.expect("nonempty tasks")? {
                            eprintln!("loopback relay HTTP connection ended: {error}");
                        }
                    }
                    accepted = listener.accept() => {
                        let (socket, _) = accepted?;
                        let routes = routes.clone();
                        connections.spawn(async move {
                            http1::Builder::new()
                                .serve_connection(
                                    TokioIo::new(MaybeTlsStream::Plain(socket)),
                                    service_fn(move |request| dispatch(request, routes.clone())),
                                )
                                .with_upgrades()
                                .await?;
                            Ok::<_, anyhow::Error>(())
                        });
                    }
                }
            }
            connections.shutdown().await;
            Ok(())
        });
        Ok(Self {
            address,
            services,
            background: Background {
                stop: Some(stop),
                task,
            },
        })
    }

    pub async fn shutdown(self) -> Result<()> {
        for service in &self.services {
            service.shutdown().await;
        }
        self.background.shutdown().await
    }
}

async fn dispatch(
    request: Request<Incoming>,
    services: [RelayService; 2],
) -> Result<Response<BytesBody>, Box<dyn std::error::Error + Send + Sync>> {
    let route = match request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
    {
        Some("Bearer pair-a") => Some(0),
        Some("Bearer pair-b") => Some(1),
        _ => None,
    };
    if let ("/relay", Some(index)) = (request.uri().path(), route) {
        return RelayServiceWithNotify::new(services[index].clone(), Arc::new(Notify::new()))
            .call(request)
            .await;
    }
    let status = match request.uri().path() {
        "/ping" => 200,
        "/relay" => 403,
        _ => 404,
    };
    Ok(Response::builder()
        .status(status)
        .body(Box::new(Full::new(Bytes::new())) as BytesBody)
        .expect("static response"))
}
