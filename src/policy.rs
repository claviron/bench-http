//! A fixed policy workload shared by the direct and eight-layer Tower hosts.
//! These deliberately small fixtures are not Claviron's production validators.

use std::convert::Infallible;
use std::future::{Future, Ready, ready};
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::SystemTime;

use bytes::Bytes;
use claviron::http::ext::{RealIpExt, ReceivedAt};
use futures_util::future::Either;
use http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_LENGTH, CONTENT_TYPE, HOST, ORIGIN, VARY};
use http::uri::Authority;
use http::{HeaderValue, Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use pin_project_lite::pin_project;
use tower::{Layer, Service, ServiceBuilder};

pub type Reply = Response<Full<Bytes>>;
type Answer = Result<Reply, Infallible>;

static REQUESTS: AtomicU64 = AtomicU64::new(0);

/// Physical peer supplied by the connection, independently of forwarding headers.
#[derive(Clone, Copy)]
pub struct PeerIp(pub IpAddr);

#[derive(Clone)]
struct HostAuthority(Authority);

#[derive(Clone)]
struct HostContext {
    authority: Authority,
    peer: IpAddr,
}

/// Immutable fixture settings, constructed before the listener starts.
#[derive(Clone, Copy)]
pub struct Checks {
    authority: AuthorityCheck,
    deny: DenyPath,
    proxy: Proxy,
    context: PublishContext,
    acl: Acl,
    length: ContentLength,
    received: ReceiveTime,
    origin: Origin,
}

impl Checks {
    pub fn sample() -> Self {
        Self {
            authority: AuthorityCheck,
            deny: DenyPath { prefix: "/denied" },
            proxy: Proxy,
            context: PublishContext,
            acl: Acl,
            length: ContentLength,
            received: ReceiveTime,
            origin: Origin {
                allowed: "https://client.example",
            },
        }
    }
}

fn empty(status: StatusCode) -> Reply {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

trait Finish {
    fn apply(&self, response: &mut Reply);
}

impl Finish for () {
    fn apply(&self, _: &mut Reply) {}
}

trait Check: Clone {
    type After: Finish;

    fn before<B>(&self, request: &mut Request<B>) -> Result<Self::After, StatusCode>;
}

#[derive(Clone, Copy)]
struct AuthorityCheck;

impl Check for AuthorityCheck {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let mut hosts = request.headers().get_all(HOST).iter();
        let host = hosts.next().ok_or(StatusCode::BAD_REQUEST)?;
        if hosts.next().is_some() {
            return Err(StatusCode::BAD_REQUEST);
        }
        let authority =
            Authority::try_from(host.as_bytes()).map_err(|_| StatusCode::BAD_REQUEST)?;
        if authority.host().is_empty()
            || request
                .uri()
                .authority()
                .is_some_and(|uri| uri != &authority)
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        request.extensions_mut().insert(HostAuthority(authority));
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct DenyPath {
    prefix: &'static str,
}

impl Check for DenyPath {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let path = request.uri().path();
        if path == self.prefix
            || path
                .strip_prefix(self.prefix)
                .is_some_and(|tail| tail.starts_with('/'))
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Proxy;

impl Check for Proxy {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let peer = request
            .extensions()
            .get::<PeerIp>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
            .0;
        let mut real_ip = peer;
        // This fixture trusts only loopback peers and accepts one forwarded IP.
        if peer.is_loopback() {
            let mut forwarded = request.headers().get_all("x-forwarded-for").iter();
            if let Some(value) = forwarded.next() {
                if forwarded.next().is_some() {
                    return Err(StatusCode::BAD_REQUEST);
                }
                real_ip = value
                    .to_str()
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .trim()
                    .parse()
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
            }
        }
        request.extensions_mut().insert(RealIpExt(real_ip));
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct PublishContext;

impl Check for PublishContext {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let authority = request
            .extensions()
            .get::<HostAuthority>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
            .0
            .clone();
        let peer = request
            .extensions()
            .get::<PeerIp>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
            .0;
        request
            .extensions_mut()
            .insert(HostContext { authority, peer });
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Acl;

impl Check for Acl {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let ip = request
            .extensions()
            .get::<RealIpExt>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
            .0;
        // Loopback and documentation networks are the fixed allowed test inputs.
        let allowed = ip.is_loopback()
            || match ip {
                IpAddr::V4(ip) => ip.octets()[..3] == [192, 0, 2],
                IpAddr::V6(ip) => ip.segments()[..2] == [0x2001, 0x0db8],
            };
        if allowed {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

#[derive(Clone, Copy)]
struct ContentLength;

impl Check for ContentLength {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        let mut lengths = request.headers().get_all(CONTENT_LENGTH).iter();
        if let Some(value) = lengths.next() {
            if lengths.next().is_some() {
                return Err(StatusCode::BAD_REQUEST);
            }
            let value = value.as_bytes();
            if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(StatusCode::BAD_REQUEST);
            }
            std::str::from_utf8(value)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .parse::<u64>()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ReceiveTime;

impl Check for ReceiveTime {
    type After = ();

    fn before<B>(&self, request: &mut Request<B>) -> Result<(), StatusCode> {
        request
            .extensions_mut()
            .insert(ReceivedAt(SystemTime::now()));
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Origin {
    allowed: &'static str,
}

struct OriginReply(Option<HeaderValue>);

impl Finish for OriginReply {
    fn apply(&self, response: &mut Reply) {
        if let Some(origin) = &self.0 {
            response
                .headers_mut()
                .insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
            response
                .headers_mut()
                .append(VARY, HeaderValue::from_static("Origin"));
        }
    }
}

impl Check for Origin {
    type After = OriginReply;

    fn before<B>(&self, request: &mut Request<B>) -> Result<OriginReply, StatusCode> {
        let mut origins = request.headers().get_all(ORIGIN).iter();
        let origin = match origins.next() {
            Some(value) => {
                if origins.next().is_some() {
                    return Err(StatusCode::BAD_REQUEST);
                }
                if value.as_bytes() != self.allowed.as_bytes() {
                    return Err(StatusCode::FORBIDDEN);
                }
                Some(value.clone())
            }
            None => None,
        };
        Ok(OriginReply(origin))
    }
}

#[derive(Clone, Copy)]
struct CheckLayer<C>(C);

#[derive(Clone)]
struct Checked<C, S> {
    check: C,
    inner: S,
}

impl<C: Clone, S> Layer<S> for CheckLayer<C> {
    type Service = Checked<C, S>;

    fn layer(&self, inner: S) -> Self::Service {
        Checked {
            check: self.0.clone(),
            inner,
        }
    }
}

pin_project! {
    struct ReplyFuture<F, A> {
        #[pin]
        inner: F,
        after: A,
    }
}

impl<F: Future<Output = Answer>, A: Finish> Future for ReplyFuture<F, A> {
    type Output = Answer;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Answer> {
        let this = self.project();
        this.inner.poll(cx).map(|result| {
            result.map(|mut response| {
                this.after.apply(&mut response);
                response
            })
        })
    }
}

impl<B, C: Check, S> Service<Request<B>> for Checked<C, S>
where
    S: Service<Request<B>, Response = Reply, Error = Infallible>,
{
    type Response = Reply;
    type Error = Infallible;
    type Future = Either<Ready<Answer>, ReplyFuture<S::Future, C::After>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        match self.check.before(&mut request) {
            Ok(after) => Either::Right(ReplyFuture {
                inner: self.inner.call(request),
                after,
            }),
            Err(status) => Either::Left(ready(Ok(empty(status)))),
        }
    }
}

/// Eight concrete layers, built once; no BoxService or erased response future.
pub fn tower<B>(
    checks: Checks,
) -> impl Service<Request<B>, Response = Reply, Error = Infallible, Future: Send> + Clone + Send + Sync
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Send,
{
    ServiceBuilder::new()
        .layer(CheckLayer(checks.authority))
        .layer(CheckLayer(checks.deny))
        .layer(CheckLayer(checks.proxy))
        .layer(CheckLayer(checks.context))
        .layer(CheckLayer(checks.acl))
        .layer(CheckLayer(checks.length))
        .layer(CheckLayer(checks.received))
        .layer(CheckLayer(checks.origin))
        .service(tower::service_fn(json::<B>))
}

fn direct_checks<B>(checks: &Checks, request: &mut Request<B>) -> Result<OriginReply, StatusCode> {
    checks.authority.before(request)?;
    checks.deny.before(request)?;
    checks.proxy.before(request)?;
    checks.context.before(request)?;
    checks.acl.before(request)?;
    checks.length.before(request)?;
    checks.received.before(request)?;
    checks.origin.before(request)
}

/// The identical checks and response work, without Tower service composition.
pub fn direct<B>(
    checks: &Checks,
    mut request: Request<B>,
) -> impl Future<Output = Answer> + Send + use<B>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Send,
{
    match direct_checks(checks, &mut request) {
        Ok(after) => Either::Right(ReplyFuture {
            inner: json(request),
            after,
        }),
        Err(status) => Either::Left(ready(Ok(empty(status)))),
    }
}

async fn json<B>(request: Request<B>) -> Answer
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Send,
{
    if request.method() != Method::GET || request.uri().path() != "/json" {
        return Ok(empty(StatusCode::NOT_FOUND));
    }
    let Some(context) = request.extensions().get::<HostContext>() else {
        return Ok(empty(StatusCode::INTERNAL_SERVER_ERROR));
    };
    if context.authority.host().is_empty()
        || request
            .extensions()
            .get::<PeerIp>()
            .is_none_or(|peer| peer.0 != context.peer)
        || request.extensions().get::<ReceivedAt>().is_none()
    {
        return Ok(empty(StatusCode::INTERNAL_SERVER_ERROR));
    }

    let (parts, body) = request.into_parts();
    let body = if body.is_end_stream() {
        Bytes::new()
    } else {
        match body.collect().await {
            Ok(body) => body.to_bytes(),
            Err(_) => return Ok(empty(StatusCode::BAD_REQUEST)),
        }
    };
    let body = match std::str::from_utf8(&body) {
        Ok(body) => body,
        Err(_) => return Ok(empty(StatusCode::BAD_REQUEST)),
    };
    let bytes = serde_json::to_vec(&crate::http1::Answer {
        runtime: "rust",
        engine: "Hyper",
        method: parts.method.as_str(),
        path: parts.uri.path(),
        body,
        requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
    })
    .expect("the response is serializable");
    let mut response = Response::new(Full::new(Bytes::from(bytes)));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(response)
}
