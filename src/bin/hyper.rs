use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use http::header::{CONTENT_TYPE, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::graceful::GracefulShutdown;
use tokio::signal;
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

static REQUESTS: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Serialize)]
struct Answer<'a> {
    runtime: &'static str,
    engine: &'static str,
    method: &'a str,
    path: &'a str,
    body: &'a str,
    requests: u64,
}

fn empty(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

async fn json(request: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    if request.method() != Method::GET || request.uri().path() != "/json" {
        return Ok(empty(StatusCode::NOT_FOUND));
    }

    let (parts, body) = request.into_parts();
    let body = if body.is_end_stream() {
        Bytes::new()
    } else {
        match body.collect().await {
            Ok(body) => body.to_bytes(),
            Err(_) => return Ok(empty(StatusCode::BAD_REQUEST)),
        }
    };
    let body = match std::str::from_utf8(&body) {
        Ok(body) => body,
        Err(_) => return Ok(empty(StatusCode::BAD_REQUEST)),
    };

    let bytes = serde_json::to_vec(&Answer {
        runtime: "rust",
        engine: "Hyper",
        method: parts.method.as_str(),
        path: parts.uri.path(),
        body,
        requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
    })
    .expect("the response is serializable");
    let mut response = Response::new(Full::new(Bytes::from(bytes)));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(response)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let address = std::env::var("BENCH_ADDRESS")?;
    let listener = bench_http::listener::bind(&address).await?;
    let mut http = http1::Builder::new();
    http.keep_alive(true).header_read_timeout(None).writev(true);
    let graceful = GracefulShutdown::new();
    let stop = shutdown_signal();
    tokio::pin!(stop);

    tracing::info!("Listening on address {}", address);
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                stream.set_nodelay(true)?;
                let connection = http.serve_connection(TokioIo::new(stream), service_fn(json));
                let connection = graceful.watch(connection);
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        tracing::warn!(%error, "HTTP connection failed");
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

    tracing::info!("signal received, shutting down");
}
