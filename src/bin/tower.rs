use bench_http::{policy, policy_server};
use hyper::body::Incoming;
use hyper::service::{Service, service_fn};
use hyper_util::service::TowerToHyperService;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prepared = policy::tower::<Incoming>(policy::Checks::sample());
    policy_server::run("Tower", move |peer| {
        // Clone the already-built stack for a connection; the stock bridge
        // also clones it per request. Neither clone erases the service/future.
        let service = TowerToHyperService::new(prepared.clone());
        service_fn(move |mut request| {
            request.extensions_mut().insert(policy::PeerIp(peer.ip()));
            service.call(request)
        })
    })
    .await
}
