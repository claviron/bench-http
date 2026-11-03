//! Warp filters for the same plain and JSON workloads as the other hosts.

use bytes::Bytes;
use claviron::warp::{self, Filter, Reply, reply::Response};
use http::StatusCode;

use super::payload::{Answer, PLAIN};

fn plain(_: Bytes) -> Response {
    warp::reply::with_header(PLAIN, "content-type", "text/plain")
        .into_response()
}

fn json(body: Bytes) -> Response {
    let Ok(body) = std::str::from_utf8(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    warp::reply::json(&Answer::new("GET", "/json", body)).into_response()
}

pub fn router() -> claviron::http::warp::Router {
    let plain = warp::path("plain")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::body::bytes())
        .map(plain);
    let json = warp::path("json")
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::body::bytes())
        .map(json);
    claviron::http::warp::router(plain.or(json).unify())
}

#[cfg(test)]
#[path = "../../tests/support/warp.rs"]
mod tests;
