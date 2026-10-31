//! Startup-only listen backlog configuration for the native benchmark hosts.

use std::env::VarError;
use std::io;

use tokio::net::{TcpListener, ToSocketAddrs};

/// Parse a requested backlog accepted by the socket API.
pub fn parse_backlog(value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .ok()
        .filter(|backlog| *backlog > 0)
        .ok_or_else(|| "listen backlog must be an integer from 1 to 2147483647".to_owned())
}

/// Read the startup setting; the OS may cap the requested queue length.
pub fn backlog_from_env() -> io::Result<i32> {
    match std::env::var("BENCH_BACKLOG") {
        Ok(value) => parse_backlog(&value).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("BENCH_BACKLOG: {error}"),
            )
        }),
        Err(VarError::NotPresent) => Ok(2048),
        Err(VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "BENCH_BACKLOG must be valid Unicode",
        )),
    }
}

/// Apply the requested backlog before publishing or accepting on the listener.
pub async fn bind(address: impl ToSocketAddrs) -> io::Result<TcpListener> {
    let backlog = backlog_from_env()?;
    let listener = TcpListener::bind(address).await?;
    socket2::SockRef::from(&listener).listen(backlog)?;
    Ok(listener)
}
