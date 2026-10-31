//! Identical response data for every server; counters are process-local.

use std::sync::atomic::{AtomicU64, Ordering};

pub const PLAIN: &[u8] = b"Hello, World!";
static REQUESTS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub enum Endpoint {
    Plain,
    Json,
}

impl Endpoint {
    pub fn select(method: &str, path: &str) -> Option<Self> {
        match (method, path) {
            ("GET", "/plain") => Some(Self::Plain),
            ("GET", "/json") => Some(Self::Json),
            _ => None,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Plain => "text/plain",
            Self::Json => "application/json",
        }
    }
}

#[derive(serde::Serialize)]
pub struct Answer<'a> {
    pub runtime: &'static str,
    pub engine: &'static str,
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a str,
    pub requests: u64,
}

impl<'a> Answer<'a> {
    pub fn new(method: &'a str, path: &'a str, body: &'a str) -> Self {
        Self {
            runtime: "rust",
            engine: "Rust",
            method,
            path,
            body,
            requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }
}
