# bench-http

Compare native HTTP servers with Claviron's registered host paths. All targets
serve the same plain and JSON workloads with mimalloc in one process by default.

## Targets

| Source | Binary | Execution |
| --- | --- | --- |
| `src/native/tokio.rs` | `tokio` | Tokio TCP tasks and a small HTTP/1 parser. |
| `src/native/mio.rs` | `mio` | One Mio Poll loop and reuse-port listener per thread. |
| `src/native/hyper.rs` | `hyper` | Native Hyper HTTP/1 service. |
| `src/native/axum.rs` | `axum` | Native `axum::serve` and Axum router. |
| `src/claviron/other.rs` | `claviron-other` | Registered terminal `OtherHandler`. |
| `src/claviron/axum.rs` | `claviron-axum` | Registered dedicated Axum router, identical to native Axum. |
| `src/claviron/routable.rs` | `claviron-routable` | Provider-registered `Routable` contribution in the configured host chain. |

`Other` and dedicated Axum own their replies. A `Routable` contribution may return
`Unhandled` with the unread request body so the configured chain can continue.
Claviron performs host admission and response policy around each registered owner.

`src/support/` holds the shared payload, HTTP handlers, Axum router, TCP parser,
listener settings, logging/shutdown and Claviron CLI setup.

## Workloads

- `GET /plain`: `200 OK`, `text/plain`, static `Hello, World!` body. No JSON
  serialization or request-counter increment; use this to expose host overhead.
- `GET /json`: read the UTF-8 request body, increment a process-local counter and
  serialize the same six fields on every target:

```json
{"runtime":"rust","engine":"Rust","method":"GET","path":"/json","body":"","requests":1}
```

Plain requests drain any supplied body without decoding it. Empty bodies skip
collection in the Hyper/Axum/Claviron handlers. Axum uses its own request extractor
and `Json` response. Hyper and Other share the same handler; Routable delegates
to that handler after claiming the path.

Mio/Tokio reuse connection buffers and vectored writes. Their limited parser
supports HTTP/1.1 Content-Length bodies, pipelining and connection close;
Transfer-Encoding and Expect are rejected, and Date is omitted. They are lower
boundary references. Server headers can also differ between native and Claviron.

## Build

Native targets do not select the optional Claviron dependency:

```sh
python3 scripts/build.py --native --target native
```

The Claviron entries use the current HostPipeline/Other API. Until a published
release contains it, select a matching source checkout explicitly:

```sh
python3 scripts/build.py --native --claviron-source /path/to/claviron
```

The default target is `all`. `--target claviron` builds only the three Claviron
entries; a binary name selects one target. `--supervisor` exposes `--workers` on
Claviron entries. Source overrides are generated under ignored `.cache/`; the
manifest keeps a registry dependency. The script verifies the selected checkout
and prints its Git revision, or the resolved registry version.

Every target uses the same release profile: opt-level 3, one codegen unit, fat
LTO, no debug information and panic unwinding. `--native` selects the build CPU;
omit it for a portable CPU target. `CARGO_TARGET_DIR` is respected.

## Run

Run from the repository root, one server at a time. In both the server and wrk
shells, raise the soft descriptor limit within the permitted hard limit:

```sh
ulimit -Sn 65536
export BENCH_ADDRESS=127.0.0.1:8080
export BENCH_BACKLOG=2048
export RUST_LOG=info
./target/release/hyper
```

Stop that server before selecting another binary. Claviron reads the same address
and backlog through `config/http.yaml`, runs in production mode, and disables
access logging through `config/vhost.yaml`:

```sh
./target/release/claviron-other --config-dir config --name bench-http --fleet bhttp
```

With `--supervisor` selected at build time, `--workers 0` is standalone and a
positive count starts supervised worker processes. The counter remains local
to each process. Mio's `--threads N` selects Poll threads inside one process.

All targets request the same declared TCP backlog and enable TCP_NODELAY. Native
listeners default to 2048 when BENCH_BACKLOG is absent; Claviron requires the
exported value. Mio's `--backlog` overrides its environment value. The OS may cap
the queue: verify the effective backlog, especially when connections are opened
in a burst. Backlog is not an active-request or keep-alive connection limit.
Shutdown drains owned work without a benchmark-specific termination timeout.

## Measure

Preflight the expected status, content type and body before applying load:

```sh
curl -i http://127.0.0.1:8080/plain
curl -i http://127.0.0.1:8080/json
wrk -t16 -c1024 -d5m --latency -s scripts/summary.lua http://127.0.0.1:8080/plain
```

Repeat separately for `/json`, using the same build flags, host, backlog,
thread/process counts and load parameters. Compare Hyper with `claviron-other`
or `claviron-routable`, and native Axum with `claviron-axum`.

Report requests/s, p99, errors and serving-process CPU/request. CPU/request is
the user+system CPU-time delta across all serving threads/processes divided by
completed requests; exclude the generator and preflight. The retained minimal-path
criterion is at most 10% extra server CPU/request against a matched native control;
it is a project budget, not a claim that the current implementation meets it.

`summary.lua` reports completed requests, wrk's corrected p99 and error counters.
It does not validate every body. Only compare runs after functional preflight
and with zero reported HTTP/socket/timeout errors. For Claviron's own isolated
microbenchmarks, use the library repository's `benches/` suite.

[BENCHMARK.md](BENCHMARK.md) retains historical measurements and their original
sources/settings. Those predate this layout and the uniform engine field;
they are not results for the current code.
