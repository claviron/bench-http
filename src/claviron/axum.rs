use bench_http::support::{axum, claviron as host};
use claviron::edge::HostAppRegistry;
use claviron::{AppState, registry};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn register(state: &AppState) -> registry::Result<()> {
    let apps = state.registry_ref().resolve::<HostAppRegistry>()
        .ok_or_else(|| registry::Error::msg("host application registry is missing"))?;
    apps.register_router(host::HOST, |_| axum::router());
    Ok(())
}

#[tokio::main]
async fn main() -> claviron::Result<()> {
    host::serve(register).await
}
