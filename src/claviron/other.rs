use bench_http::support::{claviron as host, response};
use claviron::edge::{HostAppRegistry, OtherHandler, OtherService};
use claviron::http::{RouteBody, RouteFuture, boxed_body};
use claviron::{AppState, registry};
use http::Request;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct Handler;

impl OtherHandler for Handler {
    fn call(&self, request: Request<RouteBody>) -> RouteFuture {
        Box::pin(async move { Ok(response::handle(request).await.map(boxed_body)) })
    }
}

fn register(state: &AppState) -> registry::Result<()> {
    let apps = state.registry_ref().resolve::<HostAppRegistry>()
        .ok_or_else(|| registry::Error::msg("host application registry is missing"))?;
    apps.register_other(host::HOST, |_, _, _| Ok(OtherService::new(Handler)));
    Ok(())
}

#[tokio::main]
async fn main() -> claviron::Result<()> {
    host::serve(register).await
}
