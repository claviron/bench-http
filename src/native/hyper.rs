use std::convert::Infallible;
use std::num::NonZeroUsize;

use bench_http::support::{listener, response, runtime, tls};
use bytes::Bytes;
use clap::Parser;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::graceful::GracefulShutdown;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

async fn handle(request: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(response::handle(request).await)
}

/// Serve one accepted stream with the protocol the host speaks.
async fn serve<I>(io: I, http2: bool, http: http1::Builder, watcher: hyper_util::server::graceful::Watcher)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let served = if http2 {
        let connection = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(io), service_fn(handle));
        watcher.watch(connection).await
    } else {
        watcher.watch(http.serve_connection(TokioIo::new(io), service_fn(handle))).await
    };
    if let Err(error) = served {
        runtime::connection_error(&error);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = runtime::Host::parse();
    host.runtime()?.block_on(listen(host.threads))
}

async fn listen(threads: NonZeroUsize) -> Result<(), Box<dyn std::error::Error>> {
    runtime::logging();
    let address = std::env::var("BENCH_ADDRESS")?;
    let listener = listener::bind(&address).await?;
    let http2 = tls::http2_from_env()?;
    let acceptor = tls::acceptor_from_env(http2)?;
    let mut http = http1::Builder::new();
    http.keep_alive(true).header_read_timeout(None).writev(true);
    let graceful = GracefulShutdown::new();
    let stop = runtime::shutdown_signal();
    tokio::pin!(stop);
    tracing::info!(%address, threads, tls = acceptor.is_some(), http2, "Hyper listening");
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                stream.set_nodelay(true)?;
                let http = http.clone();
                let watcher = graceful.watcher();
                let acceptor = acceptor.clone();
                // The handshake runs on the connection's own task, never on
                // the accept loop.
                tokio::spawn(async move {
                    match acceptor {
                        None => serve(stream, http2, http, watcher).await,
                        Some(acceptor) => match acceptor.accept(stream).await {
                            Ok(stream) => serve(stream, http2, http, watcher).await,
                            Err(error) => tracing::debug!(%error, "TLS handshake failed"),
                        },
                    }
                });
            }
        }
    }
    drop(listener);
    graceful.shutdown().await;
    tracing::info!("Hyper shutdown gracefully");
    Ok(())
}
