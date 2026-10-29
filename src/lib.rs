use bytes::Bytes;
use http::{Response, header::CONTENT_TYPE};
use serde::Serialize;

#[derive(Serialize)]
struct Greeting {
    message: &'static str,
}

/// Serialize the same payload on every request in both servers.
pub fn json_response() -> Response<Bytes> {
    let bytes = serde_json::to_vec(&Greeting {
        message: "Hello, world!",
    })
    .expect("the fixed greeting is serializable");
    Response::builder()
        .header(CONTENT_TYPE, "application/json")
        .body(Bytes::from(bytes))
        .expect("the fixed response headers are valid")
}
