use axum::{Router, body::Body, routing::get};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen = std::env::var("BENCH_LISTEN").unwrap_or_else(|_| "127.0.0.1:18081".into());
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    let router = Router::new().route(
        "/json",
        get(async || bench_http::json_response().map(Body::from)),
    );
    println!("AXUM_READY {listen}");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
