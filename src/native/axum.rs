use std::num::NonZeroUsize;

use axum::serve::ListenerExt;
use bench_http::support::{axum as app, listener, runtime};
use clap::Parser;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = runtime::Host::parse();
    host.runtime()?.block_on(listen(host.threads))
}

async fn listen(threads: NonZeroUsize) -> Result<(), Box<dyn std::error::Error>> {
    runtime::logging();
    let address = std::env::var("BENCH_ADDRESS")?;
    let listener = listener::bind(&address).await?.tap_io(|stream| {
        if let Err(error) = stream.set_nodelay(true) {
            tracing::error!(%error, "TCP_NODELAY setup failed");
        }
    });
    tracing::info!(%address, threads, "Axum listening");
    axum::serve(listener, app::router())
        .with_graceful_shutdown(runtime::shutdown_signal()).await?;
    tracing::info!("Axum shutdown gracefully");
    Ok(())
}
