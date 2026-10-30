# bench-http

Compare direct Mio and Tokio TCP servers, Hyper HTTP/1, Axum's native `Json`
handler and a Claviron Bypass handler over plain HTTP/1.1. All servers default
to a single process, use mimalloc, and serialize the same fields as
the PHP benchmark on every `GET /json` request:

```json
{"runtime":"rust","engine":"Axum","method":"GET","path":"/json","body":"","requests":1}
```

Mio uses `Mio` as its engine, Tokio uses `Tokio`, Hyper uses `Hyper`, and Bypass uses
`Claviron Bypass`. All read the request method, URL path and UTF-8 body, and
increment a process-wide request counter.
Axum uses its native `Request` extractor and `Json` response; Bypass uses
Claviron's request and response body types.

Hyper calls the handler directly through `service_fn`, without a router or
middleware. It uses a concrete `Full<Bytes>` response body, vectored TCP writes
and `TCP_NODELAY`. Empty request bodies skip collection. Tasks and shutdown
watchers are created per connection; requests run inside the connection task.

Tokio uses `TcpListener`/`TcpStream` and `httparse`, without Hyper or a router.
It enables `TCP_NODELAY`, reuses connection buffers for input, JSON and response
headers, and writes headers and body with vectored IO. It supports fragmented
and pipelined HTTP/1.1 requests, UTF-8 bodies framed with `Content-Length`, and
`Connection: close`. Transfer encodings and `Expect` are rejected with HTTP 400;
it does not add a `Date` header. This is a limited benchmark baseline.

Mio shares Tokio's HTTP parser and JSON schema, but handles socket readiness
directly without a Tokio runtime or connection tasks. Each server thread owns
its own `Poll`, `SO_REUSEPORT` listener and connection state. It reuses input
and output buffers, supports partial reads/writes and pipelining, and changes
the socket's registered interest only when read/write backpressure requires it.
The event batch capacity is not a connection limit. Like the Tokio baseline,
it supports HTTP/1.1 with `Content-Length`, rather than general HTTP hosting.

Mio stops accepting on Ctrl+C, SIGTERM or SIGHUP. `Waker` wakes each Poll loop;
idle connections close and admitted requests and pending writes drain without
a shutdown timeout. A client that does not complete an admitted request can
therefore keep shutdown waiting.

## Build

Requires Rust, Python 3 and `wrk`.

```sh
python3 scripts/build.py --native
```

To enable the optional `supervisor` Cargo feature for Bypass:

```sh
python3 scripts/build.py --native --supervisor
```

All binaries use the same release profile: optimization level 3, one codegen
unit, fat LTO, no debug information and unwinding on panic. `--native` optimizes
for the build machine's CPU; omit it for a portable CPU target.

The default build uses the published Claviron dependency. To use a source
checkout, including changes that have not been released:

```sh
python3 scripts/build.py --native --claviron-source /path/to/claviron
```

The script verifies the selected source and prints its commit, or the resolved
registry version. It prints the binary directory and respects `CARGO_TARGET_DIR`.

## Run

Run from the repository root. Start one server at a time on the same address.

Before starting a server or `wrk`, set the soft open-file limit in each shell:

```sh
ulimit -Sn 65536
```

A low descriptor limit can cause `Too many open files` or connection errors
during high-connection runs. The shell's hard limit must permit this value.

```sh
export BENCH_ADDRESS=127.0.0.1:8080
export RUST_LOG=info
./target/release/axum
```

Stop Axum before starting Claviron:

```sh
./target/release/bypass --config-dir config --name bench-http --fleet bhttp
```

The default build runs Bypass standalone and has no `--workers` option. When
built with `--supervisor`, `--workers 0` remains standalone; a positive count
starts supervised worker processes:

```sh
./target/release/bypass --config-dir config --name bench-http --fleet bhttp --workers 4
```

The request counter is local to each worker. `config/supervisor.yaml` keeps the
supervisor's declared defaults.

Stop Claviron before starting Hyper on the same address:

```sh
./target/release/hyper
```

Stop Hyper before starting Tokio on the same address:

```sh
./target/release/tokio
```

Stop Tokio before starting Mio on the same address (Unix):

```sh
./target/release/mio --threads 16
```

`--threads` defaults to the available CPU count. These are server threads
inside one process, independent of `wrk -t`. `--backlog` declares the requested
listen backlog per socket (default 1024); the OS may cap it. Mio reads
`BENCH_ADDRESS` and enables `TCP_NODELAY`, like the other native TCP hosts.

Axum reads `BENCH_ADDRESS` directly and enables `TCP_NODELAY`. Claviron reads
the same address through `config/http.yaml`, runs in production mode
and disables access logging through `config/vhost.yaml`.

## Measure

Check that each server returns HTTP 200 and the JSON fields shown above, with
its own engine name and an increasing request counter:

```sh
curl -i http://127.0.0.1:8080/json
```

Use the same command for each server:

```sh
wrk -t16 -c1024 -d5m --latency -s scripts/summary.lua http://127.0.0.1:8080/json
```

This uses 16 load-generator threads and 1,024 keep-alive connections for five
minutes. `summary.lua` prints `BENCH_HTTP_RESULT` with completed requests, p99
latency and error counters; duration and latency are in microseconds. It does
not validate response bodies. Interpret throughput only after checking the
expected response and confirming that all reported error counters are zero.

## Benchmark

Linux x86_64, 2 × Xeon E5-2650 v2 (16 physical / 32 logical CPUs).
All five used **`wrk -t8 -c1024 -d30m`**, one serving process, release/native
builds, the same wrk executable and allocator policy, without profiling.

| Target | Requests/s | p99, ms | Server CPU, µs/request | Errors |
|---|---:|---:|---:|---:|
| Mio TCP | 645,592.68 | 3.337 | 11.904 | 0 |
| Tokio TCP | 614,926.97 | 2.962 | 12.919 | 0 |
| Hyper | 582,626.08 | 3.506 | 19.210 | 0 |
| Axum | 545,869.55 | 3.997 | 26.497 | 0 |
| Claviron Bypass | 494,709.70 | 5.682 | 33.575 | 0 |

Mio and Tokio are the retained completed runs; Hyper, Axum and Bypass were
then measured sequentially on the same system. Bypass uses Claviron
`7de9d4b`. Server implementations and listener defaults differ; each row is
one observation, rather than a repeated capacity estimate.

[BENCHMARK.md](BENCHMARK.md) contains full conditions, source/binary revisions,
CPU accounting, error counts, the native Deno reference and historical series.
