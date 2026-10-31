use bench_http::support::{payload::PLAIN, response};
use bytes::Bytes;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full};

#[tokio::test]
async fn shared_json_handler_preserves_utf8_method_path_and_counter() {
    let request = Request::builder().uri("/json?ignored=true")
        .body(Full::new(Bytes::from_static("hello λ".as_bytes()))).unwrap();
    let reply = response::handle(request).await;
    assert_eq!(reply.status(), StatusCode::OK);
    assert_eq!(reply.headers()["content-type"], "application/json");
    let bytes = reply.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["runtime"], "rust");
    assert_eq!(value["engine"], "Rust");
    assert_eq!(value["method"], "GET");
    assert_eq!(value["path"], "/json");
    assert_eq!(value["body"], "hello λ");
    assert!(value["requests"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn plain_drains_a_body_without_decoding_it_and_json_rejects_invalid_utf8() {
    for (path, status) in [("/plain", StatusCode::OK), ("/json", StatusCode::BAD_REQUEST)] {
        let request = Request::builder().uri(path)
            .body(Full::new(Bytes::from_static(&[0xff]))).unwrap();
        let reply = response::handle(request).await;
        assert_eq!(reply.status(), status);
        if status == StatusCode::OK {
            assert_eq!(reply.headers()["content-type"], "text/plain");
            assert_eq!(reply.into_body().collect().await.unwrap().to_bytes().as_ref(), PLAIN);
        }
    }
}
