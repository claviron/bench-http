use std::sync::Arc;

use claviron::edge::{BypassHandler, BypassService, HostAppRegistry};
use claviron::http::{RouteBody, RouteFuture};
use claviron::{AppState, Server, ServerOptions, registry};
use http::{Method, Request, Response, StatusCode};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

struct JsonHandler;

impl BypassHandler for JsonHandler {
    fn name(&self) -> &'static str {
        "native_json"
    }

    fn call(&self, request: Request<RouteBody>) -> RouteFuture {
        Box::pin(async move {
            let response = if request.method() == Method::GET && request.uri().path() == "/json" {
                bench_http::json_response().map(claviron::http::bytes_body)
            } else {
                let mut response = Response::new(claviron::http::empty_body());
                *response.status_mut() = StatusCode::NOT_FOUND;
                response
            };
            Ok(response)
        })
    }
}

fn register(state: &AppState) -> registry::Result<()> {
    let apps = state
        .registry_ref()
        .resolve::<HostAppRegistry>()
        .ok_or_else(|| registry::Error::msg("host application registry is missing"))?;
    apps.register_bypass("default", |_, _, _| {
        Ok(BypassService::Handler(Arc::new(JsonHandler)))
    });
    Ok(())
}

#[tokio::main]
async fn main() -> claviron::Result<()> {
    let fleet = std::env::var("BENCH_FLEET").unwrap_or_else(|_| "bhttp".into());
    let options = ServerOptions::new()
        .config_dir("config")
        .name(
            "bench-http"
                .parse::<claviron::ServerName>()
                .expect("valid server name"),
        )
        .fleet_name(
            fleet
                .parse::<claviron::FleetName>()
                .expect("valid fleet name"),
        )
        .workers(0)
        .production();
    Server::with_options(options)
        .register(register)
        .serve()
        .await
}
