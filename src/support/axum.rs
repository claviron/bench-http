//! One router used by native axum::serve and Claviron's dedicated Axum binding.

use axum::{Json, Router, extract::Request, http::StatusCode, response::{IntoResponse, Response}, routing::get};

use super::{payload::Answer, response};

async fn plain(request: Request) -> Result<Response, StatusCode> {
    response::read_body(request.into_body()).await?;
    Ok(response::plain().into_response())
}

async fn json(request: Request) -> Result<Response, StatusCode> {
    let (parts, body) = request.into_parts();
    let body = response::read_body(body).await?;
    let body = std::str::from_utf8(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(Answer::new(parts.method.as_str(), parts.uri.path(), body)).into_response())
}

pub fn router() -> Router {
    Router::new().route("/plain", get(plain)).route("/json", get(json))
}
