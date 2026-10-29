# bench-http

Compare Axum's native `Json` handler with a Claviron Bypass handler over plain
HTTP/1.1. Both servers run in a single process, use Tokio and mimalloc, and
serialize the same fields as the PHP benchmark on every `GET /json` request:

```json
{"runtime":"rust","engine":"Axum","method":"GET","path":"/json","body":"","requests":1}
```

The Bypass response uses `Claviron Bypass` as its engine. Both read the request
method, URL path and UTF-8 body, and increment a process-wide request counter.
Axum uses its native `Request` extractor and `Json` response; Bypass uses
Claviron's request and response body types.

## Build

Requires Rust, Python 3 and `wrk`.

```sh
python3 scripts/build.py --native
```

Both binaries use the same release profile: optimization level 3, one codegen
unit, fat LTO, no debug information and unwinding on panic. `--native` optimizes
for the build machine's CPU; omit it for a portable CPU target.

The default build uses the published Claviron dependency. To use a source
checkout, including changes that have not been released:

```sh
python3 scripts/build.py --native --claviron-source ../../claviron
```

The script verifies the selected source and prints its commit, or the resolved
registry version. It prints the binary directory and respects `CARGO_TARGET_DIR`.

## Run

Run from the repository root. Start one server at a time on the same address.

```sh
export BENCH_ADDRESS=127.0.0.1:8080
export RUST_LOG=info
./target/release/axum
```

Stop Axum before starting Claviron:

```sh
./target/release/bypass --config-dir config --name bench-http --fleet bhttp
```

Axum reads `BENCH_ADDRESS` directly and enables `TCP_NODELAY`. Claviron reads
the same address through `config/http.yaml`, runs in standalone production mode
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
