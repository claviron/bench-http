use std::io::{self, IoSlice, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};

use bench_http::http1::{Answer, parse_head};
use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::signal;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

static REQUESTS: AtomicU64 = AtomicU64::new(0);

async fn write_response(stream: &mut TcpStream, header: &[u8], body: &[u8]) -> io::Result<()> {
    let mut buffers = [IoSlice::new(header), IoSlice::new(body)];
    let mut remaining = &mut buffers[..];
    while !remaining.is_empty() {
        let written = stream.write_vectored(remaining).await?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        IoSlice::advance_slices(&mut remaining, written);
    }
    Ok(())
}

async fn serve(mut stream: TcpStream, mut stop: watch::Receiver<()>) -> io::Result<()> {
    let mut input = BytesMut::with_capacity(1024);
    let mut json = Vec::with_capacity(256);
    let mut header = Vec::with_capacity(128);
    loop {
        let head = match parse_head(&input) {
            Ok(head) => head,
            Err(_) => {
                return stream
                    .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
            }
        };
        let in_flight = head.is_some();
        if let Some(head) = head.filter(|head| input.len() >= head.end) {
            if head.method != "GET" || head.path != "/json" {
                return stream
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
            }
            let body = match std::str::from_utf8(&input[head.header_len..head.end]) {
                Ok(body) => body,
                Err(_) => {
                    return stream
                        .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .await;
                }
            };
            json.clear();
            serde_json::to_writer(
                &mut json,
                &Answer {
                    runtime: "rust",
                    engine: "Tokio",
                    method: head.method,
                    path: head.path,
                    body,
                    requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
                },
            )
            .expect("the response is serializable");
            let close = head.close || stop.has_changed().unwrap_or(true);
            header.clear();
            write!(
                header,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{}\r\n",
                json.len(),
                if close { "connection: close\r\n" } else { "" },
            )?;
            write_response(&mut stream, &header, &json).await?;
            if close {
                return Ok(());
            }
            let end = head.end;
            if end == input.len() {
                input.clear();
            } else {
                input.advance(end);
            }
            continue;
        }

        let read = if in_flight {
            stream.read_buf(&mut input).await?
        } else {
            tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                read = stream.read_buf(&mut input) => read?,
            }
        };
        if read == 0 {
            return if input.is_empty() {
                Ok(())
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
    }
}

fn completed(result: Result<io::Result<()>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error))
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
            ) =>
        {
            tracing::debug!(%error, "HTTP peer disconnected");
        }
        Ok(Err(error)) => tracing::warn!(%error, "HTTP connection failed"),
        Err(error) => tracing::error!(%error, "HTTP connection task failed"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let listener = bench_http::listener::bind(std::env::var("BENCH_ADDRESS")?).await?;
    let (stop, receiver) = watch::channel(());
    let mut connections = JoinSet::new();
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    tracing::info!("Listening on address {}", listener.local_addr()?);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(result) = connections.join_next(), if !connections.is_empty() => completed(result),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                stream.set_nodelay(true)?;
                connections.spawn(serve(stream, receiver.clone()));
            }
        }
    }
    drop(listener);
    stop.send_replace(());
    while let Some(result) = connections.join_next().await {
        completed(result);
    }
    tracing::info!("Tokio shutdown gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
