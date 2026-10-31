use std::convert::Infallible;

use bench_http::support::{listener, response, runtime};
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::graceful::GracefulShutdown;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

async fn handle(request: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(response::handle(request).await)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    runtime::logging();
    let address = std::env::var("BENCH_ADDRESS")?;
    let listener = listener::bind(&address).await?;
    let mut http = http1::Builder::new();
    http.keep_alive(true).header_read_timeout(None).writev(true);
    let graceful = GracefulShutdown::new();
    let stop = runtime::shutdown_signal();
    tokio::pin!(stop);
    tracing::info!(%address, "Hyper listening");
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                stream.set_nodelay(true)?;
                let connection = http.serve_connection(TokioIo::new(stream), service_fn(handle));
                let connection = graceful.watch(connection);
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        runtime::connection_error(&error);
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
