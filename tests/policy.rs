use std::convert::Infallible;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bench_http::policy::{self, Checks, PeerIp, Reply};
use bytes::Bytes;
use futures_util::FutureExt;
use futures_util::future::Either;
use futures_util::task::noop_waker_ref;
use http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_LENGTH, HOST, ORIGIN};
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame};
use tower::ServiceExt;

fn request<B>(body: B) -> Request<B> {
    Request::builder()
        .uri("/json")
        .header(HOST, "benchmark.test")
        .extension(PeerIp(IpAddr::from([127, 0, 0, 1])))
        .body(body)
        .unwrap()
}

fn answer<B>(tower: bool, request: Request<B>) -> Reply
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Send,
{
    let checks = Checks::sample();
    let result = if tower {
        policy::tower(checks).oneshot(request).now_or_never()
    } else {
        policy::direct(&checks, request).now_or_never()
    };
    result.expect("fixture body is immediately ready").unwrap()
}

#[test]
fn direct_and_tower_preserve_json_body_and_response_headers() {
    let mut outputs = Vec::new();
    for tower in [false, true] {
        let mut request = request(Full::new(Bytes::from_static(b"request body")));
        request
            .headers_mut()
            .insert("x-forwarded-for", "192.0.2.10".parse().unwrap());
        request
            .headers_mut()
            .insert(CONTENT_LENGTH, "12".parse().unwrap());
        request
            .headers_mut()
            .insert(ORIGIN, "https://client.example".parse().unwrap());
        let response = answer(tower, request);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://client.example"
        );
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .now_or_never()
            .unwrap()
            .unwrap()
            .to_bytes();
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["runtime"], "rust");
        assert_eq!(json["engine"], "Hyper");
        assert_eq!(json["body"], "request body");
        assert!(json["requests"].as_u64().unwrap() > 0);
        json.as_object_mut().unwrap().remove("requests");
        outputs.push((headers, json));
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn acl_uses_forwarded_identity_only_for_a_trusted_peer() {
    for tower in [false, true] {
        // The physical peer is allowed, but the forwarded client is denied.
        let mut forwarded = request(NeverRead);
        forwarded
            .headers_mut()
            .insert("x-forwarded-for", "203.0.113.10".parse().unwrap());
        assert_eq!(answer(tower, forwarded).status(), StatusCode::FORBIDDEN);

        // An untrusted peer cannot borrow an allowed identity from its header.
        let mut spoofed = request(NeverRead);
        spoofed
            .extensions_mut()
            .insert(PeerIp(IpAddr::from([203, 0, 113, 10])));
        spoofed
            .headers_mut()
            .insert("x-forwarded-for", "192.0.2.10".parse().unwrap());
        assert_eq!(answer(tower, spoofed).status(), StatusCode::FORBIDDEN);

        // Malformed forwarding is ignored for an untrusted, permitted peer.
        let mut ignored = request(Full::new(Bytes::new()));
        ignored
            .extensions_mut()
            .insert(PeerIp(IpAddr::from([192, 0, 2, 10])));
        ignored
            .headers_mut()
            .insert("x-forwarded-for", "invalid".parse().unwrap());
        assert_eq!(answer(tower, ignored).status(), StatusCode::OK);
    }
}

#[test]
fn rejected_heads_do_not_poll_the_body() {
    for tower in [false, true] {
        let mut missing_host = request(NeverRead);
        missing_host.headers_mut().remove(HOST);
        assert_eq!(
            answer(tower, missing_host).status(),
            StatusCode::BAD_REQUEST
        );

        let mut denied = request(NeverRead);
        *denied.uri_mut() = "/denied/private".parse().unwrap();
        assert_eq!(answer(tower, denied).status(), StatusCode::FORBIDDEN);

        let mut malformed = request(NeverRead);
        malformed
            .headers_mut()
            .insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        assert_eq!(answer(tower, malformed).status(), StatusCode::BAD_REQUEST);

        let mut duplicate = request(NeverRead);
        duplicate
            .headers_mut()
            .append("x-forwarded-for", "192.0.2.10".parse().unwrap());
        duplicate
            .headers_mut()
            .append("x-forwarded-for", "192.0.2.11".parse().unwrap());
        assert_eq!(answer(tower, duplicate).status(), StatusCode::BAD_REQUEST);

        let mut length = request(NeverRead);
        length
            .headers_mut()
            .insert(CONTENT_LENGTH, "+1".parse().unwrap());
        assert_eq!(answer(tower, length).status(), StatusCode::BAD_REQUEST);

        let mut origin = request(NeverRead);
        origin
            .headers_mut()
            .insert(ORIGIN, "https://other.example".parse().unwrap());
        assert_eq!(answer(tower, origin).status(), StatusCode::FORBIDDEN);
    }
}

#[test]
fn origin_response_work_also_wraps_an_application_404() {
    for tower in [false, true] {
        let mut request = request(NeverRead);
        *request.uri_mut() = "/missing".parse().unwrap();
        request
            .headers_mut()
            .insert(ORIGIN, "https://client.example".parse().unwrap());
        let response = answer(tower, request);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://client.example"
        );
    }
}

#[test]
fn dropping_a_pending_reply_releases_the_request_body() {
    for tower in [false, true] {
        let dropped = Arc::new(AtomicBool::new(false));
        let request = request(PendingBody(dropped.clone()));
        let checks = Checks::sample();
        let future = if tower {
            Either::Left(policy::tower(checks).oneshot(request))
        } else {
            Either::Right(policy::direct(&checks, request))
        };
        {
            let mut future = std::pin::pin!(future);
            let mut context = Context::from_waker(noop_waker_ref());
            assert!(future.as_mut().poll(&mut context).is_pending());
        }
        assert!(dropped.load(Ordering::Relaxed));
    }
}

struct NeverRead;

impl Body for NeverRead {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        panic!("a rejected request body must remain unpolled")
    }
}

struct PendingBody(Arc<AtomicBool>);

impl Body for PendingBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Pending
    }
}

impl Drop for PendingBody {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
