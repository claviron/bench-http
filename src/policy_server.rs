//! Identical Hyper HTTP/1 transport for the two policy-composition controls.

use std::convert::Infallible;
use std::net::SocketAddr;

use hyper::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper_util::rt::TokioIo;
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::TcpListener;
use tokio::signal;
use tracing_subscriber::EnvFilter;

use crate::policy::Reply;

pub async fn run<M, S>(name: &'static str, make: M) -> Result<(), Box<dyn std::error::Error>>
where
    M: Fn(SocketAddr) -> S,
    S: Service<Request<Incoming>, Response = Reply, Error = Infallible> + Send + 'static,
    S::Future: Send,
{
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let address = std::env::var("BENCH_ADDRESS")?;
    let listener = TcpListener::bind(&address).await?;
    let mut http = http1::Builder::new();
    http.keep_alive(true).header_read_timeout(None).writev(true);
    let graceful = GracefulShutdown::new();
    let stop = shutdown_signal();
    tokio::pin!(stop);

    tracing::info!(server = name, "Listening on address {}", address);
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                stream.set_nodelay(true)?;
                let connection = http.serve_connection(TokioIo::new(stream), make(peer));
                let connection = graceful.watch(connection);
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        // Peer disconnects, including load-generator shutdown, are transport diagnostics.
                        tracing::debug!(%error, "HTTP connection ended");
                    }
                });
            }
        }
    }

    drop(listener);
    graceful.shutdown().await;
    tracing::info!(server = name, "shutdown gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
