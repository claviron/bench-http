//! Hyper and Other share this handler; Axum shares its payload and body reader.

use bytes::Bytes;
use http::{HeaderValue, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;

use super::payload::{Answer, Endpoint, PLAIN};

pub fn status(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

pub fn plain() -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from_static(PLAIN)));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    response
}

pub async fn read_body<B: Body<Data = Bytes>>(body: B) -> Result<Bytes, StatusCode> {
    if body.is_end_stream() {
        Ok(Bytes::new())
    } else {
        body.collect().await.map(|body| body.to_bytes()).map_err(|_| StatusCode::BAD_REQUEST)
    }
}

pub async fn handle<B: Body<Data = Bytes>>(request: Request<B>) -> Response<Full<Bytes>> {
    let Some(endpoint) = Endpoint::select(request.method().as_str(), request.uri().path()) else {
        return status(StatusCode::NOT_FOUND);
    };
    handle_selected(request, endpoint).await
}

pub async fn handle_selected<B: Body<Data = Bytes>>(
    request: Request<B>,
    endpoint: Endpoint,
) -> Response<Full<Bytes>> {
    let (parts, body) = request.into_parts();
    let body = match read_body(body).await {
        Ok(body) => body,
        Err(error) => return status(error),
    };
    match endpoint {
        Endpoint::Plain => plain(),
        Endpoint::Json => {
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_) => return status(StatusCode::BAD_REQUEST),
            };
            let bytes = serde_json::to_vec(&Answer::new(parts.method.as_str(), parts.uri.path(), body))
                .expect("the response is serializable");
            let mut response = Response::new(Full::new(Bytes::from(bytes)));
            response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response
        }
    }
}
