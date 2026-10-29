use bytes::Bytes;
use clap::Parser;
use claviron::edge::{BypassHandler, BypassService, HostAppRegistry};
use claviron::http::{RouteBody, RouteFuture};
use claviron::{AppState, FleetName, Server, ServerName, ServerOptions, registry};
use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

static REQUESTS: AtomicU64 = AtomicU64::new(0);

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "config")]
    config_dir: PathBuf,
    #[arg(long, default_value = "bypass-axum")]
    name: ServerName,
    #[arg(long, default_value = "bypass")]
    fleet: FleetName,
    /// Worker processes; 0 runs standalone.
    #[cfg(feature = "supervisor")]
    #[arg(long, default_value_t = 0)]
    workers: u8,
}

impl From<Args> for ServerOptions {
    fn from(args: Args) -> Self {
        let options = Self::new()
            .config_dir(args.config_dir)
            .name(args.name)
            .fleet_name(args.fleet)
            .production();
        #[cfg(feature = "supervisor")]
        let options = options.workers(args.workers);
        options
    }
}

#[derive(serde::Serialize)]
struct Answer<'a> {
    runtime: &'static str,
    engine: &'static str,
    method: &'a str,
    path: &'a str,
    body: &'a str,
    requests: u64,
}

fn json_response(method: &Method, uri: &http::Uri, body: &str) -> Response<Bytes> {
    let bytes = serde_json::to_vec(&Answer {
        runtime: "rust",
        engine: "Claviron Bypass",
        method: method.as_str(),
        path: uri.path(),
        body,
        requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
    })
    .expect("the response is serializable");
    Response::builder()
        .header(CONTENT_TYPE, "application/json")
        .body(Bytes::from(bytes))
        .expect("the fixed response headers are valid")
}

struct JsonHandler;

impl BypassHandler for JsonHandler {
    fn name(&self) -> &'static str {
        "native_json"
    }

    fn call(&self, request: Request<RouteBody>) -> RouteFuture {
        Box::pin(async move {
            let response = if request.method() == Method::GET && request.uri().path() == "/json" {
                let (parts, body) = request.into_parts();
                let body = match body.collect().await {
                    Ok(body) => body.to_bytes(),
                    Err(_) => {
                        let mut response = Response::new(claviron::http::empty_body());
                        *response.status_mut() = StatusCode::BAD_REQUEST;
                        return Ok(response);
                    }
                };
                let body = match std::str::from_utf8(&body) {
                    Ok(body) => body,
                    Err(_) => {
                        let mut response = Response::new(claviron::http::empty_body());
                        *response.status_mut() = StatusCode::BAD_REQUEST;
                        return Ok(response);
                    }
                };
                json_response(&parts.method, &parts.uri, body).map(claviron::http::bytes_body)
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
    Server::with_options(Args::parse().into())
        .register(register)
        .serve()
        .await
}
