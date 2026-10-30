use bench_http::{policy, policy_server};
use hyper::service::service_fn;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = policy::Checks::sample();
    policy_server::run("Hyper control", move |peer| {
        service_fn(move |mut request| {
            request.extensions_mut().insert(policy::PeerIp(peer.ip()));
            policy::direct(&prepared, request)
        })
    })
    .await
}
