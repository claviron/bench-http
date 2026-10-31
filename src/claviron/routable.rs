use std::sync::Arc;

use bench_http::support::{claviron as host, payload::Endpoint, response};
use claviron::edge::pipeline::host::{HostHandler, HostHandlerFuture, HostRequest};
use claviron::edge::{Host, HostConfig, Routable};
use claviron::http::{ServiceResult, boxed_body};
use claviron::{AppState, Component, registry};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct Application;

impl Component<AppState> for Application {
    type Deps = ();

    fn name(&self) -> &'static str {
        "benchmark-routes"
    }
}

impl Routable for Application {
    fn match_host(&self, key: &Host, _: &HostConfig) -> bool {
        key.as_single() == Some(host::HOST)
    }

    fn application(
        self: Arc<Self>, _: &AppState, _: &Host, _: Arc<HostConfig>,
    ) -> registry::Result<Option<Arc<dyn HostHandler>>> {
        Ok(Some(self))
    }
}

impl HostHandler for Application {
    fn call(&self, request: HostRequest) -> HostHandlerFuture {
        let head = request.head();
        let Some(endpoint) = Endpoint::select(head.method.as_str(), head.uri.path()) else {
            return Box::pin(std::future::ready(ServiceResult::Unhandled(request)));
        };
        Box::pin(async move {
            let response = response::handle_selected(request.into_request(), endpoint).await;
            ServiceResult::Handled(response.map(boxed_body))
        })
    }
}

fn register(state: &AppState) -> registry::Result<()> {
    claviron::edge::register_routable(state, Arc::new(Application))
}

#[tokio::main]
async fn main() -> claviron::Result<()> {
    host::serve(register).await
}
