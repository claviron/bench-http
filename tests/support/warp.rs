use bytes::Bytes;
use claviron::http::{RustRouter, bytes_body};
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;

#[tokio::test]
async fn warp_and_axum_serve_the_same_workloads() {
    let routers = [
        ("axum", RustRouter::Axum(crate::support::axum::router())),
        ("warp", RustRouter::Warp(super::router())),
    ];
    for (name, router) in routers {
        for (method, uri, body, expected) in [
            ("GET", "/plain", b"".as_slice(), StatusCode::OK),
            ("GET", "/plain", b"\xff", StatusCode::OK),
            ("GET", "/json", b"", StatusCode::OK),
            ("GET", "/json?ignored=1", b"payload", StatusCode::OK),
            ("GET", "/json", b"\xff", StatusCode::BAD_REQUEST),
            ("GET", "/json/extra", b"", StatusCode::NOT_FOUND),
            ("GET", "/missing", b"", StatusCode::NOT_FOUND),
            ("POST", "/json", b"", StatusCode::METHOD_NOT_ALLOWED),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .body(bytes_body(Bytes::from_static(body)))
                .unwrap();
            let response = router.handle(request).await.unwrap();
            assert_eq!(response.status(), expected, "{name}: {method} {uri}");
            if expected != StatusCode::OK {
                continue;
            }
            let (parts, response) = response.into_parts();
            let response = response.collect().await.unwrap().to_bytes();
            if uri == "/plain" {
                assert_eq!(parts.headers[header::CONTENT_TYPE], "text/plain");
                assert_eq!(response, crate::support::payload::PLAIN);
            } else {
                assert_eq!(
                    parts.headers[header::CONTENT_TYPE],
                    "application/json"
                );
                let mut json: serde_json::Value =
                    serde_json::from_slice(&response).unwrap();
                assert!(json["requests"].as_u64().unwrap() > 0);
                json.as_object_mut().unwrap().remove("requests");
                assert_eq!(
                    json,
                    serde_json::json!({
                        "runtime": "rust",
                        "engine": "Rust",
                        "method": method,
                        "path": "/json",
                        "body": std::str::from_utf8(body).unwrap(),
                    })
                );
            }
        }
    }
}
