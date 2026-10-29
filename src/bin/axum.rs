use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::serve::ListenerExt;
use axum::{Json, Router, routing::get};
use http_body_util::BodyExt;

use tokio::net::TcpListener;
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

async fn json(request: Request) -> Result<Response, StatusCode> {
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_bytes();
    let body = std::str::from_utf8(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(Answer {
        runtime: "rust",
        engine: "Axum",
        method: parts.method.as_str(),
        path: parts.uri.path(),
        body,
        requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
    })
    .into_response())
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

    let listener = TcpListener::bind(&address).await?.tap_io(|stream| {
        if let Err(error) = stream.set_nodelay(true) {
            tracing::error!("TCP_NODELAY setup failed: {error}");
        }
    });

    let app = Router::new().route("/json", get(json));

    tracing::info!("Listening on address {}", address);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("🏁 axum shutdown gracefully");
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
