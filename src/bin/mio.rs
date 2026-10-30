use std::collections::HashMap;
use std::io::{self, IoSlice, Read, Write};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;

use bench_http::http1::{Answer, parse_head};
use clap::Parser;
use mio::net::{TcpListener, TcpStream};
use mio::{Events, Interest, Poll, Registry, Token, Waker};
use socket2::{Domain, Protocol, Socket, Type};
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

static REQUESTS: AtomicU64 = AtomicU64::new(0);
const LISTENER: Token = Token(0);
const STOP: Token = Token(1);

#[derive(Parser)]
#[command(about = "Serve HTTP/1.1 JSON directly from one Mio Poll loop per thread")]
struct Options {
    /// Number of independent Poll loops in this process.
    #[arg(long, default_value_t = thread::available_parallelism().unwrap_or(NonZeroUsize::MIN))]
    threads: NonZeroUsize,
    /// Requested listen backlog for each socket; the OS may cap it.
    #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(i32).range(1..))]
    backlog: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Read,
    Write,
    Closed,
}

impl Waiting {
    fn interest(self) -> Interest {
        match self {
            Self::Read => Interest::READABLE,
            Self::Write => Interest::WRITABLE,
            Self::Closed => unreachable!("closed sockets are deregistered"),
        }
    }
}

struct Connection {
    socket: TcpStream,
    input: Vec<u8>,
    used: usize,
    json: Vec<u8>,
    header: Vec<u8>,
    written: usize,
    consumed: usize,
    writing: bool,
    close: bool,
    waiting: Waiting,
}

impl Connection {
    fn new(socket: TcpStream) -> Self {
        Self {
            socket,
            input: vec![0; 1024],
            used: 0,
            json: Vec::with_capacity(256),
            header: Vec::with_capacity(128),
            written: 0,
            consumed: 0,
            writing: false,
            close: false,
            waiting: Waiting::Read,
        }
    }

    fn reject(&mut self, response: &[u8]) {
        self.header.clear();
        self.header.extend_from_slice(response);
        self.json.clear();
        self.written = 0;
        self.writing = true;
        self.close = true;
    }

    fn flush(&mut self) -> io::Result<bool> {
        let mut buffers = [
            IoSlice::new(&self.header[self.written.min(self.header.len())..]),
            IoSlice::new(&self.json[self.written.saturating_sub(self.header.len())..]),
        ];
        let mut remaining = &mut buffers[..];
        while !remaining.is_empty() {
            match self.socket.write_vectored(remaining) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => {
                    self.written += written;
                    IoSlice::advance_slices(&mut remaining, written);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }

    fn drive(&mut self, stop: &AtomicBool) -> io::Result<Waiting> {
        loop {
            if self.writing {
                if !self.flush()? {
                    return Ok(Waiting::Write);
                }
                if self.close || stop.load(Ordering::Relaxed) {
                    return Ok(Waiting::Closed);
                }
                if self.consumed == self.used {
                    self.used = 0;
                } else {
                    self.input.copy_within(self.consumed..self.used, 0);
                    self.used -= self.consumed;
                }
                self.writing = false;
            }

            let head = match parse_head(&self.input[..self.used]) {
                Ok(head) => head,
                Err(_) => {
                    self.reject(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                    continue;
                }
            };
            if let Some(head) = head {
                if self.used >= head.end {
                    if head.method != "GET" || head.path != "/json" {
                        self.reject(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                        continue;
                    }
                    let body = match std::str::from_utf8(&self.input[head.header_len..head.end]) {
                        Ok(body) => body,
                        Err(_) => {
                            self.reject(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                            continue;
                        }
                    };
                    self.json.clear();
                    serde_json::to_writer(
                        &mut self.json,
                        &Answer {
                            runtime: "rust",
                            engine: "Mio",
                            method: head.method,
                            path: head.path,
                            body,
                            requests: REQUESTS.fetch_add(1, Ordering::Relaxed) + 1,
                        },
                    )
                    .expect("the response is serializable");
                    self.close = head.close || stop.load(Ordering::Relaxed);
                    self.consumed = head.end;
                    self.header.clear();
                    write!(
                        self.header,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{}\r\n",
                        self.json.len(),
                        if self.close {
                            "connection: close\r\n"
                        } else {
                            ""
                        },
                    )?;
                    self.written = 0;
                    self.writing = true;
                    continue;
                }
            } else if stop.load(Ordering::Relaxed) {
                return Ok(Waiting::Closed);
            }

            if self.used == self.input.len() {
                let length = self.input.len().checked_mul(2).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::OutOfMemory, "request buffer is too large")
                })?;
                self.input.resize(length, 0);
            }
            match self.socket.read(&mut self.input[self.used..]) {
                Ok(0) if self.used == 0 => return Ok(Waiting::Closed),
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(read) => self.used += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(Waiting::Read);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn ready(&mut self, registry: &Registry, token: Token, stop: &AtomicBool) -> bool {
        let result = self.drive(stop).and_then(|waiting| {
            if waiting != Waiting::Closed && waiting != self.waiting {
                registry.reregister(&mut self.socket, token, waiting.interest())?;
                self.waiting = waiting;
            }
            Ok(waiting)
        });
        match result {
            Ok(Waiting::Read | Waiting::Write) => return true,
            Ok(Waiting::Closed) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                ) =>
            {
                tracing::debug!(%error, "HTTP peer disconnected");
            }
            Err(error) => tracing::warn!(%error, "HTTP connection failed"),
        }
        if let Err(error) = registry.deregister(&mut self.socket) {
            tracing::debug!(%error, "HTTP socket deregistration failed");
        }
        false
    }
}

struct Worker {
    poll: Poll,
    listener: Option<TcpListener>,
    connections: HashMap<Token, Connection>,
    next_token: usize,
}

impl Worker {
    fn new(mut listener: TcpListener) -> io::Result<(Self, Waker)> {
        let poll = Poll::new()?;
        poll.registry()
            .register(&mut listener, LISTENER, Interest::READABLE)?;
        let waker = Waker::new(poll.registry(), STOP)?;
        Ok((
            Self {
                poll,
                listener: Some(listener),
                connections: HashMap::new(),
                next_token: STOP.0 + 1,
            },
            waker,
        ))
    }

    fn accept(&mut self, stop: &AtomicBool) -> io::Result<()> {
        let Some(listener) = &mut self.listener else {
            return Ok(());
        };
        while !stop.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            stream.set_nodelay(true)?;
            let token = Token(self.next_token);
            self.next_token = self
                .next_token
                .checked_add(1)
                .ok_or_else(|| io::Error::other("connection tokens exhausted"))?;
            self.poll
                .registry()
                .register(&mut stream, token, Interest::READABLE)?;
            self.connections.insert(token, Connection::new(stream));
        }
        Ok(())
    }

    fn run(mut self, stop: &AtomicBool) -> io::Result<()> {
        // This is an event batch size, not a connection or request limit.
        let mut events = Events::with_capacity(1024);
        loop {
            if stop.load(Ordering::Relaxed) && self.listener.is_some() {
                let mut listener = self.listener.take().expect("the listener is present");
                self.poll.registry().deregister(&mut listener)?;
                drop(listener);
                self.connections.retain(|token, connection| {
                    connection.ready(self.poll.registry(), *token, stop)
                });
            }
            if self.listener.is_none() && self.connections.is_empty() {
                return Ok(());
            }
            match self.poll.poll(&mut events, None) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            for event in &events {
                match event.token() {
                    LISTENER => self.accept(stop)?,
                    STOP => {}
                    token => {
                        if let Some(connection) = self.connections.get_mut(&token)
                            && !connection.ready(self.poll.registry(), token, stop)
                        {
                            self.connections.remove(&token);
                        }
                    }
                }
            }
        }
    }
}

fn bind(address: SocketAddr, backlog: i32) -> io::Result<TcpListener> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&address.into())?;
    socket.listen(backlog)?;
    Ok(TcpListener::from_std(socket.into()))
}

fn stop_workers(stop: &AtomicBool, wakers: &[Waker]) {
    stop.store(true, Ordering::Relaxed);
    for waker in wakers {
        // A worker that has already exited no longer needs waking.
        let _ = waker.wake();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .without_time()
        .init();

    let mut address: SocketAddr = std::env::var("BENCH_ADDRESS")?.parse()?;
    let mut workers = Vec::with_capacity(options.threads.get());
    let mut wakers = Vec::with_capacity(options.threads.get());
    for _ in 0..options.threads.get() {
        let listener = bind(address, options.backlog)?;
        address = listener.local_addr()?;
        let (worker, waker) = Worker::new(listener)?;
        workers.push(worker);
        wakers.push(waker);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let wakers = Arc::new(wakers);
    ctrlc::set_handler({
        let stop = Arc::clone(&stop);
        let wakers = Arc::clone(&wakers);
        move || stop_workers(&stop, &wakers)
    })?;

    let mut threads = Vec::with_capacity(workers.len());
    let mut failure = None;
    for (index, worker) in workers.into_iter().enumerate() {
        let worker_stop = Arc::clone(&stop);
        let worker_wakers = Arc::clone(&wakers);
        let started = thread::Builder::new()
            .name(format!("mio-{index}"))
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| worker.run(&worker_stop)))
                    .unwrap_or_else(|_| Err(io::Error::other("Mio poll thread panicked")));
                if result.is_err() {
                    stop_workers(&worker_stop, &worker_wakers);
                }
                result
            });
        match started {
            Ok(thread) => threads.push(thread),
            Err(error) => {
                failure = Some(error);
                stop_workers(&stop, &wakers);
                break;
            }
        }
    }
    if failure.is_none() {
        tracing::info!(%address, threads = options.threads.get(), backlog = options.backlog, "Listening");
    }
    for thread in threads {
        let result = thread
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("Mio poll thread panicked")));
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    tracing::info!("Mio shutdown gracefully");
    Ok(())
}
