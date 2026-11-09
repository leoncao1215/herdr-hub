use std::{
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use hyper::{
    Request, Response, StatusCode, body::Incoming, client::conn::http1 as client_http1,
    server::conn::http1 as server_http1, service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::DEADLINE;

type ProxyError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
struct Tasks {
    tracker: TaskTracker,
    cancellation: CancellationToken,
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Tasks {
    fn spawn(&self, work: impl Future<Output = Result<(), ProxyError>> + Send + 'static) {
        let cancelled = self.cancellation.clone();
        let handle = self.tracker.spawn(async move {
            tokio::select! {
                _ = cancelled.cancelled() => {},
                result = work => {
                    if let Err(error) = result {
                        eprintln!("loopback HTTP proxy: {error}");
                    }
                },
            }
        });
        self.handles
            .lock()
            .expect("proxy task handles")
            .push(handle);
    }
}

pub struct HttpProxy {
    pub address: SocketAddr,
    tasks: Tasks,
}

impl HttpProxy {
    pub async fn start(upstream: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let tasks = Tasks {
            tracker: TaskTracker::new(),
            cancellation: CancellationToken::new(),
            handles: Arc::new(Mutex::new(Vec::new())),
        };
        let incoming_tasks = tasks.clone();
        tasks.spawn(async move {
            loop {
                let (socket, _) = listener.accept().await?;
                let requests = incoming_tasks.clone();
                incoming_tasks.spawn(async move {
                    server_http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(socket),
                            service_fn(move |request| forward(request, upstream, requests.clone())),
                        )
                        .with_upgrades()
                        .await?;
                    Ok(())
                });
            }
        });
        Ok(Self { address, tasks })
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.tasks.cancellation.cancel();
        self.tasks.tracker.close();
        timeout(DEADLINE, self.tasks.tracker.wait()).await?;
        let handles: Vec<_> = self
            .tasks
            .handles
            .lock()
            .expect("proxy task handles")
            .drain(..)
            .collect();
        for handle in handles {
            handle.await?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn proxy_shutdown_reports_background_panics() -> Result<()> {
    async fn panic_after_ready(ready: tokio::sync::oneshot::Sender<()>) -> Result<(), ProxyError> {
        let _ = ready.send(());
        panic!("intentional proxy test panic");
    }

    let proxy = HttpProxy::start("127.0.0.1:9".parse()?).await?;
    let (ready, received) = tokio::sync::oneshot::channel();
    proxy.tasks.spawn(panic_after_ready(ready));
    timeout(DEADLINE, received).await??;
    anyhow::ensure!(proxy.shutdown().await.is_err(), "background panic was lost");
    Ok(())
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        self.tasks.cancellation.cancel();
    }
}

async fn forward(
    mut request: Request<Incoming>,
    upstream: SocketAddr,
    tasks: Tasks,
) -> Result<Response<Incoming>, ProxyError> {
    let downstream_upgrade = hyper::upgrade::on(&mut request);
    request
        .headers_mut()
        .insert(hyper::header::HOST, upstream.to_string().parse()?);
    let socket = TcpStream::connect(upstream).await?;
    let (mut sender, connection) = client_http1::handshake(TokioIo::new(socket)).await?;
    tasks.spawn(async move {
        connection.with_upgrades().await?;
        Ok(())
    });
    let mut response = sender.send_request(request).await?;
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tasks.spawn(async move {
            let (downstream, upstream) = tokio::try_join!(downstream_upgrade, upstream_upgrade)?;
            tokio::io::copy_bidirectional(
                &mut TokioIo::new(downstream),
                &mut TokioIo::new(upstream),
            )
            .await?;
            Ok(())
        });
    }
    Ok(response)
}
