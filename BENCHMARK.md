# HTTP/1.1 JSON benchmarks

Recorded October 29–30, 2026. This report collects existing measurements; it
does not combine different machines or load settings into one ranking.

## Workload and accounting

Every Rust target handles `GET /json`, reads the method, path and UTF-8 body,
increments a process-wide counter and serializes six JSON fields per request:

```json
{"runtime":"rust","engine":"Mio","method":"GET","path":"/json","body":"","requests":1}
```

The engine field identifies the target. These are dynamic responses, not cached
JSON bytes. The loaded GET has an empty body. HTTP/1.1 keep-alive is enabled.
Each measured Rust server runs as one process. Bypass runs standalone, with no
supervised child workers.

Mio and Tokio use a limited HTTP/1.1 implementation with `httparse`, reusable
connection buffers, vectored writes and `TCP_NODELAY`. They accept
Content-Length bodies, reject transfer encodings and Expect, and omit Date.
Hyper is a direct `service_fn`; Axum uses its request extractor and native
`Json` response. Bypass includes Claviron's edge admission, context, policy and
body adaptation. Response header and engine-string sizes differ.

Server CPU/request is the serving process's user + system CPU delta, across
all its threads, divided by completed wrk requests. It excludes wrk and is not
the complete host networking cost. Generator CPU is measured separately.
Process CPU, perf task-clock and sampled-clock attribution use different
accounting and are not interchangeable.

The Lua callback reports totals and wrk's corrected latency histogram only
after the load. It does not check every response body. Outside the timed load,
the recorded Linux series check HTTP status, JSON fields, counters, UTF-8 body
handling and connection reuse. Independent server-counter deltas cross-check
completed client requests; the residual can include in-flight requests at exit.

## Measurement sets

| Set | System | wrk settings | Targets | Instrumentation |
|---|---|---|---|---|
| A | Linux x86_64; 2 × Xeon E5-2650 v2, 16 physical / 32 logical CPUs | `-t8 -c1024 -d30m` | Mio, Tokio; native Deno reference | No profiler |
| B | Linux x86_64; 40 logical CPUs | `-t4 -c64 -d5m` | Tokio, Hyper, Axum, Bypass | Boundary resource counters |
| C | Same 40-CPU system as B | `-t16 -c2048 -d30m` | Hyper, original Bypass | Resource counters, no profiler |
| D | Same 40-CPU system as B | `-t16 -c2048 -d5m` | Saved Hyper, successive Bypass revisions | Target-only perf record + stat |
| E | macOS; hardware details not retained with the output | `-t4 -c1024 -d5m` | Tokio, Hyper, Axum, Bypass | Terminal output only |
| F | Same 32-CPU system as A | `-t8 -c1024 -d30m` | Mio/Tokio retained, Hyper/Axum/Bypass added | No profiler |

Each set used sequential loads with the generator on the server's machine.
Tests used the system's existing scheduling and CPU affinity. There is one
observation per target/revision in each set, without a confidence interval.
Similar throughput does not establish equal CPU cost or a server capacity
ceiling. Set F completes the same-host/load five-target table using retained
Mio/Tokio results and three additional runs.

## F — five Rust targets, same system and load

This completes the five-target comparison on the 32-logical-CPU system from A.
The existing Mio and Tokio results are reused unchanged; **only Hyper, Axum
and Bypass were added**. All five runs used 8 generator threads, 1,024 requested
keep-alive connections and 30 minutes on the same physical machine, with one
server and one load generator active at a time. They were sequential, not
contemporaneous or repeated controls.

```sh
wrk -t8 -c1024 -d30m --latency \
  -s scripts/summary.lua http://127.0.0.1:18087/json
```

| Target | Requests/s | p99, ms | Server CPU, µs/request | Errors |
|---|---:|---:|---:|---:|
| Mio TCP | 645,592.68 | 3.337 | 11.904 | 0 |
| Tokio TCP | 614,926.97 | 2.962 | 12.919 | 0 |
| Hyper | 582,626.08 | 3.506 | 19.210 | 0 |
| Axum | 545,869.55 | 3.997 | 26.497 | 0 |
| Claviron Bypass | 494,709.70 | 5.682 | 33.575 | 0 |

The two raw implementations retain their narrower HTTP support. Hyper uses
a direct service, Axum its own serve/router/Json, and Bypass Claviron's full
edge admission/context/body path. Native engine strings and HTTP response
headers differ. Matching the host and load does not isolate those individual
costs or remove the co-located generator's CPU/network demand.

### Completed work and latency

| Target | Requests | Duration, s | Mean, ms | p50, ms | p90, ms | Maximum, ms |
|---|---:|---:|---:|---:|---:|---:|
| Mio TCP | 1,162,124,055 | 1800.088644 | 0.860 | 0.769 | 0.940 | 26.920 |
| Tokio TCP | 1,106,918,518 | 1800.081274 | 1.030 | 0.846 | 1.670 | 33.520 |
| Hyper | 1,048,784,500 | 1800.098801 | 1.230 | 1.050 | 1.950 | 202.280 |
| Axum | 982,613,325 | 1800.088164 | 1.410 | 1.290 | 2.160 | 206.430 |
| Claviron Bypass | 890,523,052 | 1800.092142 | 1.730 | 1.630 | 2.680 | 51.260 |

### Process CPU and validation

| Target | Server user CPU, s | Server system CPU, s | Generator CPU, s | OS threads, before/after | Counter residual | Connect/read/write/status/timeout |
|---|---:|---:|---:|---|---:|---|
| Mio TCP | 3,232.85 | 10,600.53 | 10,424.462463 | 34/34 | 518 | 0/0/0/0/0 |
| Tokio TCP | 4,316.91 | 9,983.74 | 10,469.920596 | 33/33 | 477 | 0/0/0/0/0 |
| Hyper | 9,034.57 | 11,112.45 | 10,528.360115 | 33/33 | 691 | 0/0/0/0/0 |
| Axum | 14,198.56 | 11,837.90 | 10,531.624568 | 33/33 | 514 | 0/0/0/0/0 |
| Claviron Bypass | 18,028.56 | 11,870.94 | 10,320.113846 | 34/33 | 594 | 0/0/0/0/0 |

Each preflight issued the same 1,024 concurrent JSON requests, two reused
keep-alive GETs, a UTF-8-body GET and a final counter-reading GET (counter
1,028). All preflight sockets closed before timing; no idle control sockets
or HTTP probes were retained. Postflight counter deltas exclude the reading
GET and reconcile with client completions within 1,024 in-flight requests.
Server birth identities were stable, with no tracer or child serving process.
All owned servers received SIGINT after load and exited 0 after drain.

Mio uses 32 independent Poll loops with SO_REUSEPORT listeners. The other
four targets use the default 32 Tokio runtime workers without a thread-count
override. Total OS thread counts include main and runtime/helper threads.
Listener defaults and HTTP framing remain those of each existing target.

Hyper, Axum and Bypass use fresh private network/PID/IPC/mount namespaces,
private SHM/tmp and the same loopback port, as did Mio and Tokio. Kernel TCP
observations verify connection counts without sending HTTP requests. Early
startup snapshots showed 849 established connections for Hyper and 809 for
Axum; subsequent observations showed 1,024 for both. Bypass observations
showed 1,024. These startup snapshots are retained, rather than assuming all
requested connections were established immediately. Builds finished before load. No profiling, CPU pinning, OS tuning, production service
operation, source edit or version bump was performed for these measurements.

RSS, informational only: Mio TCP 3.62 → 7.25 MiB; Tokio TCP 6.26 → 9.37 MiB; Hyper 15.19 → 28.89 MiB; Axum 10.48 → 34.34 MiB; Claviron Bypass 24.50 → 28.54 MiB.

### Build provenance

All Rust builds use opt-level 3, one codegen unit, fat LTO, debug=0,
panic=unwind and `-C target-cpu=native` through `scripts/build.py`.
Compiler: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`.
Natural MiMalloc release policy, `MIMALLOC_ALLOW_THP=0`, purge overrides unset;
warn logging plus startup/shutdown readiness records, access log off.

Hyper/Axum/Bypass benchmark revision:
`047e61ed216d4b054add1d94dad995d372b2598b`.
Its Rust sources/configuration match Mio's snapshot; the intervening commit
only documented results. Claviron main was pulled before building and pinned
at `7de9d4b9c3a567fa972da543eea77740e2cae7d9` for Bypass.
Orbitive `bc0ebd2f0c5a2c9d953ac4f727d5dbcc50b8246a` and
Continuo `d9c507ae7560c279f9eae6aa085cbf1e1b948657` were pinned locally.
No Claviron request dispatch runs inside the raw, Hyper or Axum targets.
The benchmark supervisor feature is off; Bypass runs standalone even though
Claviron's edge feature transitively compiles internal supervisor code.

The additional three builds and Mio resolve the same Cargo.lock, SHA-256
`ed7826892338e99b6494cec51075b56642249db78543d26889859e7bcb20d6e8`.

Packages: axum 0.8.9, httparse 1.10.1, hyper 1.12.0, hyper-util 0.1.21, libmimalloc-sys 0.1.49, mimalloc 0.1.52, mio 1.2.4, serde_json 1.0.154, tokio 1.54.1.

Shared wrk 4.2.0 SHA-256:
`850cfa3049d6b912a5b75f83cce2a8a1377ec9eefddca16bbd101cf079cb2ee8`.

| Target | Benchmark revision | Executable SHA-256 |
|---|---|---|
| Mio TCP | `3dc5960df9f9f9950bac3fc114e1e13cdf9052f1` | `824df271c925feb8f23408dc2dc481c20785e794719b8ee59f080ecd6024462e` |
| Tokio TCP | `5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7` | `e050720aff40c61cb94ea96629eb65bc13c464f0659e4584d33cf067f3aab06c` |
| Hyper | `047e61ed216d4b054add1d94dad995d372b2598b` | `bda783050da0bfb09abb9f99a661b43784e534610d1c501db5f472f2767a02da` |
| Axum | `047e61ed216d4b054add1d94dad995d372b2598b` | `253a7943e7d2547cb18d4e97eeb0ad0137d27f2ebd7491fd1b5fc501b0dd6c71` |
| Claviron Bypass | `047e61ed216d4b054add1d94dad995d372b2598b` | `33186ea1d405491f660f9160db7a309103ef3749462b78ca533a44c0fe9507b9` |

### Measurement windows and primary reports

| Target | UTC start | UTC end | Evidence |
|---|---|---|---|
| Mio TCP | `2026-10-30T03:06:15.429311+00:00` | `2026-10-30T03:36:15.560899+00:00` | [Report](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-895) |
| Tokio TCP | `2026-10-30T02:25:55.684047+00:00` | `2026-10-30T02:55:55.796460+00:00` | [Report](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-886) |
| Hyper | `2026-10-30T04:24:25.363969+00:00` | `2026-10-30T04:54:25.494373+00:00` | [Report](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-903) |
| Axum | `2026-10-30T04:54:26.255065+00:00` | `2026-10-30T05:24:26.378608+00:00` | [Report](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-905) |
| Claviron Bypass | `2026-10-30T05:24:27.180921+00:00` | `2026-10-30T05:54:27.309261+00:00` | [Report](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-907) |

### Raw output for the three added runs

#### Hyper

```text
Running 30m test @ http://127.0.0.1:18087/json
  8 threads and 1024 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency     1.23ms  657.06us 202.28ms   80.47%
    Req/Sec    73.20k     5.38k   98.42k    67.17%
  Latency Distribution
     50%    1.05ms
     75%    1.60ms
     90%    1.95ms
     99%    3.51ms
  1048784500 requests in 30.00m, 199.20GB read
Requests/sec: 582626.08
Transfer/sec:    113.32MB

BENCH_HTTP_RESULT {"duration_us":1800098801,"requests":1048784500,"p99_us":3506,"errors":{"connect":0,"read":0,"write":0,"status":0,"timeout":0}}
```

#### Axum

```text
Running 30m test @ http://127.0.0.1:18087/json
  8 threads and 1024 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency     1.41ms  760.81us 206.43ms   82.24%
    Req/Sec    68.58k     4.74k   96.60k    73.39%
  Latency Distribution
     50%    1.29ms
     75%    1.78ms
     90%    2.16ms
     99%    4.00ms
  982613325 requests in 30.00m, 185.67GB read
Requests/sec: 545869.55
Transfer/sec:    105.62MB

BENCH_HTTP_RESULT {"duration_us":1800088164,"requests":982613325,"p99_us":3997,"errors":{"connect":0,"read":0,"write":0,"status":0,"timeout":0}}
```

#### Claviron Bypass

```text
Running 30m test @ http://127.0.0.1:18087/json
  8 threads and 1024 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency     1.73ms    1.03ms  51.26ms   83.75%
    Req/Sec    62.16k     4.07k   91.33k    76.48%
  Latency Distribution
     50%    1.63ms
     75%    2.01ms
     90%    2.68ms
     99%    5.68ms
  890523052 requests in 30.00m, 193.14GB read
Requests/sec: 494709.70
Transfer/sec:    109.87MB

BENCH_HTTP_RESULT {"duration_us":1800092142,"requests":890523052,"p99_us":5682,"errors":{"connect":0,"read":0,"write":0,"status":0,"timeout":0}}
```

The earlier Mio raw output and native Deno reference remain in A. Historical
40-CPU and macOS sets B–E remain separate and are not used in this table.

## A — Mio, Tokio and native Deno, 30 minutes

All three 30-minute runs completed. Both Rust targets use `/json`.
Native Deno uses `/deno` and JS
JSON.stringify with the same field names and request/body/counter work, but
different runtime, engine and path strings. This compares complete servers,
not identical byte responses or an isolated executor.

```sh
wrk -t8 -c1024 -d30m --latency \
  -s scripts/summary.lua http://127.0.0.1:18087/json
```

| Target | Completed requests | Requests/s | p99, ms | Server CPU, µs/request | Errors |
|---|---:|---:|---:|---:|---:|
| Mio TCP | 1,162,124,055 | 645,592.68 | 3.337 | 11.904 | 0 |
| Tokio TCP | 1,106,918,518 | 614,926.97 | 2.962 | 12.919 | 0 |
| Native Deno CLI | 802,379,215 | 445,751.55 | 3.395 | 31.271 | 0 |

Tokio completed 37.95% more requests/s than native Deno in this observation.
Mio completed 4.99% more requests/s than Tokio with 7.86% less process
CPU/request, while its p99 was 12.66% higher. Mio completed 44.83% more
requests/s than native Deno. These are single sequential observations.
The configurations differ:

| Target | Server topology | Initial total OS threads | Idle counter-control connections |
|---|---|---:|---:|
| Mio | 32 independent Poll loops and SO_REUSEPORT listeners, one process | 34 | 0 |
| Tokio | Default 32-thread multithread runtime, one process | 33 | 0 |
| Native Deno CLI | 16 serving isolates with current-thread Tokio runtimes, one process | 21 | 16 |

All three use 1,024 active wrk connections. The Deno control connections are
idle during timing and allow per-isolate counter readback afterward. Mio's
1,024-event batch is not a connection limit. The Rust global counter needs no
retained control connection. No HTTP probes run during the load.

The Rust builds use optimization level 3, one codegen unit, fat LTO, debug=0,
panic=unwind and `-C target-cpu=native`, built through `scripts/build.py`.
Rustc: `1.99.1 (58cf8f9c3 2026-10-13)`. Mimalloc uses its natural release
policy with `MIMALLOC_ALLOW_THP=0` and purge overrides unset. Logging is warn
plus startup/shutdown; there is no access log. Build work finished before
load. Runs use private network/PID/IPC/mount namespaces with private tmp/SHM.

| Artifact | Revision or SHA-256 |
|---|---|
| Mio benchmark source | `3dc5960df9f9f9950bac3fc114e1e13cdf9052f1` |
| Tokio benchmark source | `5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7` |
| Mio executable | `824df271c925feb8f23408dc2dc481c20785e794719b8ee59f080ecd6024462e` |
| Tokio executable | `e050720aff40c61cb94ea96629eb65bc13c464f0659e4584d33cf067f3aab06c` |
| Shared wrk 4.2.0 executable | `850cfa3049d6b912a5b75f83cce2a8a1377ec9eefddca16bbd101cf079cb2ee8` |

The Mio commit extracts Tokio's parser/schema into a shared module. Resolved
Rust packages include Tokio 1.54.1, Mio 1.2.4, httparse 1.10.1, serde_json
1.0.154, mimalloc 0.1.52 and libmimalloc-sys 0.1.49; Mio also uses socket2
0.6.5 and ctrlc 3.5.2. Claviron was compiled as a workspace dependency but is
not part of either raw server's request path.

Native Deno is the official 2.9.8 release, V8 15.0.245.2-rusty, with
`DENO_JOBS=16 deno serve --parallel`. An untimed source-file audit observed
startup reads and no source/cache path operations during requests. The
measured process was fresh and untraced. Deno's current-thread Tokio runtimes
and custom HTTP/1 path differ from this raw Rust implementation.

Tokio mean / p50 / p90 / maximum latency: 1.030 / 0.846 / 1.670 / 33.520 ms.
Native Deno: 2.260 / 2.240 / 3.020 / 35.060 ms. Tokio server CPU was
14,300.65 s and generator CPU 10,469.92 s over 1,800.081274 s.
Tokio's handler-counter residual was 477 requests; Deno's was 683, both
within 1,024 in-flight requests. Both servers exited cleanly after the load.

Mio mean / p50 / p90 / maximum latency: 0.860 / 0.769 / 0.940 / 26.920 ms.
Server user + system CPU was 3,232.85 + 10,600.53 = 13,833.38 s; generator
CPU was 10,424.462463 s over 1,800.088644 s. The measured UTC window was
`2026-10-30T03:06:15.429311+00:00` to
`2026-10-30T03:36:15.560899+00:00`. Server birth identity and thread count
were unchanged; there were no child processes or tracer. The independent
counter delta was 1,162,124,573, a residual of 518 over wrk completions,
within 1,024 possible in-flight requests. Generator and server exited 0;
the server logged graceful shutdown after SIGINT.

Mio RSS, informational only: 3.62 → 7.25 MiB.

Sources: [Tokio result](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-886),
[Mio result](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-895),
[native Deno result](https://git.claviron.dev/claviron/bench-deno/issues/1#issuecomment-876),
[Deno preparation](https://git.claviron.dev/claviron/bench-deno/issues/1#issuecomment-871).

### Raw Mio output

```text
Running 30m test @ http://127.0.0.1:18087/json
  8 threads and 1024 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency     0.86ms  507.16us  26.92ms   95.10%
    Req/Sec    81.11k     4.74k  103.77k    75.16%
  Latency Distribution
     50%  769.00us
     75%  840.00us
     90%    0.94ms
     99%    3.34ms
  1162124055 requests in 30.00m, 178.63GB read
Requests/sec: 645592.68
Transfer/sec:    101.62MB

BENCH_HTTP_RESULT {"duration_us":1800088644,"requests":1162124055,"p99_us":3337,"errors":{"connect":0,"read":0,"write":0,"status":0,"timeout":0}}
```

### Native Deno versus raw Tokio: source findings

Reviewed the measured Deno 2.9.8 tag (`fe7ed012d92c642fd8e5607ac5a45d517538b5c5`)
and the retained raw Tokio benchmark. No new Deno run or profiling was started.

- **Different runtime layout.** `deno serve --parallel` creates a current-thread
  Tokio runtime for each serving worker. The recorded native run had 16 serving
  isolates. The raw Tokio target uses one multithread runtime with 32 workers;
  connection tasks may be scheduled across them. Similar thread counts do not
  describe the same I/O/task layout.
- **A shared listener.** Declarative CLI serving selects the internal
  load-balanced mode. In this tag it binds one listener and clones its FD into
  the worker runtimes. The address-map mutex is used during binding/drop, while
  accept directly awaits the worker's Tokio listener. This is distinct from
  Mio's independent SO_REUSEPORT listeners; the source does not justify calling
  native CLI parallelism a SO_REUSEPORT topology.
- **Dynamic JSON can use a flat-response shortcut.** `Response.json(data)` still
  serializes data on every call. With default initialization it retains the
  resulting string and content type in private lazy/fast fields. "Static body"
  here means a complete body already in memory, not a cached constant response.
  The native H1 writer has a status-200/content-type-only path that builds the
  complete small response in reused connection scratch space and writes it
  together. The raw Tokio target instead reuses separate JSON/header buffers
  and performs vectored writes. Neither implementation needs a body stream for
  this JSON shape.
- **The HTTP/1 driver is specialized.** The CLI's plain HTTP/1 branch uses
  `deno_http_h1`; Hyper serves the HTTP/2 branch. The custom framing still uses
  httparse, including reusable uninitialized header scratch. Small status-200
  content-type-only responses use a compact writer instead of the general
  response-header/stream path. V8 one-byte strings are copied into owned bytes,
  validated as ASCII, and returned directly when ASCII; Latin-1 and wider text
  retain UTF-8 conversion. JSON bytes are then copied into reused wire scratch.
  This is a small-buffer/copy design, not a zero-copy or cached-JSON result.
- **Promise completion can remain in the current poll.** Native dispatch invokes
  the JS callback and performs a V8 microtask checkpoint. If the handler and its
  response ops finish then, the response-ready check immediately returns Ready
  and the H1 connection can write without another Tokio scheduling turn. A
  genuinely pending operation still waits normally. The raw Tokio handler also
  constructs its JSON within the connection task: neither target spawns a new
  task per request. This is not evidence of a replacement Tokio executor.
- **Cancellation polling also differs.** On an empty input buffer, the raw
  Tokio target's biased select polls `stop.changed()` before socket reading.
  In Tokio 1.54.1 this uses watch's eight-shard BigNotify; a pending Notify
  waiter takes a mutex on registration and removal. Thus the stop branch can
  register a waiter even when the subsequent read is immediately ready.
  Deno's thread-local Cancelable checks cancellation, polls the underlying I/O,
  and only links a cancellation waiter if that I/O returns Pending. This is a
  source-confirmed difference in this particular raw benchmark's graceful-stop
  composition, not proof of mutex contention or a measured throughput gain.
- **Counter and HTTP semantics differ.** Native JS counters are per isolate;
  raw Tokio updates a process-wide relaxed atomic. Native GET Request.text()
  resolves to an empty string; the raw server supports Content-Length GET
  bodies. The measured load has empty GET bodies, so this does not establish a
  skipped benchmark payload. The CLI wrapper accepts request arguments, so its
  zero-argument/no-Request optimization does not apply to this handler.

These are source-confirmed design differences and eligible fast paths, not
measured attribution of their individual CPU costs or proof that every branch
was taken in the historical load. No call-count/off-CPU profile was collected
for this investigation. The completed same-host t8/c1024/30m observations remain
Tokio **614,926.97 requests/s, 12.919 µs CPU/request** and native Deno
**445,751.55 requests/s, 31.271 µs CPU/request**. Thus the recorded high absolute
Deno throughput does not exceed this raw Tokio result.

Sources: [worker/runtime layout](https://github.com/denoland/deno/blob/v2.9.8/cli/tools/serve.rs#L90),
[current-thread builder](https://github.com/denoland/deno/blob/v2.9.8/runtime/tokio_util.rs#L25),
[CLI listener selection](https://github.com/denoland/deno/blob/v2.9.8/ext/http/00_serve.ts#L1655),
[shared listener](https://github.com/denoland/deno/blob/v2.9.8/ext/net/tcp.rs#L141),
[JSON response construction](https://github.com/denoland/deno/blob/v2.9.8/ext/fetch/23_response.js#L696),
[native callback](https://github.com/denoland/deno/blob/v2.9.8/ext/http/service.rs#L517),
[flat H1 response writer](https://github.com/denoland/deno/blob/v2.9.8/ext/http/http_next.rs#L3561),
[raw Tokio source](https://git.claviron.dev/claviron/bench-http/src/commit/5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7/src/bin/tokio.rs).

Cancellation sources: [Tokio watch implementation](https://github.com/tokio-rs/tokio/blob/tokio-1.54.1/tokio/src/sync/watch.rs#L1001),
[Notify registration](https://github.com/tokio-rs/tokio/blob/tokio-1.54.1/tokio/src/sync/notify.rs#L1146),
[Notify removal](https://github.com/tokio-rs/tokio/blob/tokio-1.54.1/tokio/src/sync/notify.rs#L1328),
[Deno cancellation polling](https://github.com/denoland/deno/blob/v2.9.8/libs/core/async_cancel.rs#L332).

Additional sources: [H1 selection / H2 Hyper branch](https://github.com/denoland/deno/blob/v2.9.8/ext/http/http_next.rs#L5300),
[httparse integration](https://github.com/denoland/deno/blob/v2.9.8/libs/http_h1/parse.rs#L137),
[compact response writer](https://github.com/denoland/deno/blob/v2.9.8/libs/http_h1/write.rs#L107),
[ASCII / UTF-8 conversion](https://github.com/denoland/deno/blob/v2.9.8/ext/http/v8_util.rs#L5).

## B — four-target baseline, 64 connections

```sh
wrk -t4 -c64 -d5m --latency \
  -s scripts/summary.lua http://127.0.0.1:8080/json
```

| Target | Requests/s | p99, ms | Server CPU, µs/request | Average server CPU equivalents | Errors |
|---|---:|---:|---:|---:|---:|
| Tokio TCP | 350,390.85 | 0.241 | 14.123 | 4.949 | 0 |
| Hyper | 347,477.42 | 0.193 | 19.496 | 6.774 | 0 |
| Axum | 342,874.24 | 0.252 | 26.790 | 9.185 | 0 |
| Claviron Bypass | 322,627.89 | 0.311 | 40.953 | 13.212 | 0 |

Benchmark revision `5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7`; Claviron
`41fbe91356ce7465f6568b21876628c05b7670dd`. All four used native release
builds with opt-level 3, fat LTO, one codegen unit, debug=0 and mimalloc.
Package versions: Tokio 1.54.1, Hyper 1.12.0, hyper-util 0.1.21, Axum 0.8.9,
serde_json 1.0.154, mimalloc 0.1.52 / libmimalloc-sys 0.1.49.

Each server had 41 OS threads, including 40 Tokio runtime workers, with no
child serving processes. The benchmark's supervisor feature was disabled;
Claviron's edge dependency still compiled internal supervisor code
transitively. This was standalone execution, not a supervisor-free build.

Bypass's charged CPU/request was 2.900× Tokio's, 2.101× Hyper's and 1.529×
Axum's in this baseline. The later Bypass revisions below were not present.
Generator CPU averaged 2.93–2.95 equivalents across four threads. Host idle
was 51–73%, with no observed FD, memory, swap, quota or thermal exhaustion.
This does not rule out transport/scheduling costs or establish maximum
server throughput.

[Full baseline and resource evidence](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-836).

## C — Hyper and original Bypass, 2,048 connections, 30 minutes

```sh
wrk -t16 -c2048 -d30m --latency \
  -s scripts/summary.lua http://127.0.0.1:8080/json
```

Same frozen source/binaries as B, with warm standalone servers:

| Target | Completed requests | Requests/s | p99, ms | Server CPU, µs/request | Connect/read/write/status | Timeout counter |
|---|---:|---:|---:|---:|---|---:|
| Hyper | 1,398,759,638 | 777,047.96 | 11.646 | 22.436 | 0/0/0/0 | **130** |
| Claviron Bypass | 925,450,184 | 514,113.87 | 16.874 | 43.843 | 0/0/0/0 | 0 |

Hyper's result is not error-free. In this wrk version, its timeout counter
includes completed responses beyond the default two-second histogram limit:
they remain in the completion total but are excluded from the histogram.
The reported p99 therefore does not describe the complete extreme tail.
The 130 counts are retained; their timing was not recorded.

Observed established connections were 2,048 for both. Listen backlogs differed
(Hyper 128, Bypass 1,024); startup listen-queue drops were observed for both.
Host-wide counters cannot attribute every packet to a target. Host idle was
about 7.5–8.5%, with generator and server sharing CPUs. This is a measured
co-located throughput result, not an isolated server ceiling.

Bypass's 514k result here and 323k in B used the same binary: thread count,
concurrency and duration changed together. The low-concurrency result was
not a fixed throughput limit.

[Full sustained comparison and timeout semantics](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-851).

## D — recorded Bypass improvements under profiling

```sh
wrk -t16 -c2048 -d5m --latency \
  -s scripts/summary.lua http://127.0.0.1:8080/json
```

Target-only perf record at 99 Hz (`cpu-clock`, 8 KiB DWARF stack snapshots)
ran alongside perf stat. All rows have zero reported wrk errors. Hyper is
a saved reference, not a fresh control for each later Bypass run.

| Target / Claviron revision | Requests/s | p99, ms | Process CPU, µs/request | perf task-clock, µs/request |
|---|---:|---:|---:|---:|
| Hyper, saved reference | 759,529.06 | 12.534 | 22.612 | 28.981 |
| Bypass `41fbe91` | 507,132.78 | 16.995 | 44.136 | 51.396 |
| Bypass `2fc5ee5` | 522,219 | 17.226 | 42.062 | 49.285 |
| Bypass `6f83a34` | 547,417 | 17.061 | 39.058 | 46.218 |
| Bypass `7de9d4b` | 567,340.16 | 16.842 | 37.607 | 44.646 |

The `6f83a34` Bypass run completed 164,283,762 requests in 300.107110 seconds,
with 6,416.54 s server CPU and 2,423.339 s generator CPU. Its counter residual
was 1,416, within the 2,048 possible in-flight requests.

Compared with the preceding Bypass capture, throughput rose 4.83% and
process CPU/request fell 7.14%. Against the original profiled Bypass, the
accumulated difference is +7.94% throughput and −11.51% process CPU/request.
Bypass `6f83a34` used 1.727× Hyper's process CPU/request in these
saved-reference observations.

The handler-future change reduced sampled copy time from approximately
4.738 to 1.866 sampled-clock µs/request. Compiled hot copy operands fell from
18,344 to 6,888 bytes and from 9,168 to 3,440 bytes. These are observed
compiled state-copy sizes, not HTTP payload sizes or copied bytes/request.
Request dispatch, authority parsing and typed context work remain visible
in the saved profiles; inclusive symbol times overlap and cannot be summed.

The recordings reported zero lost samples and full running time for requested
stat counters. Full call ancestry is limited by the 8 KiB stack snapshot and
optimized code. Profiler overhead was not isolated; process age, counter
width, source revisions and the shared generator/host also differ between
the sequential saved-reference captures. The small p99 change is not
evidence of a repeatable latency improvement.

Bypass `6f83a34` source: `6f83a348f2004dc28bd0c6b3bf4f849dfb63b574`;
benchmark source remains `5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7`.
`6f83a34` executable SHA-256:
`9fb9b9f177a891951d969ad7c8688145c4c7f95bb82e9b7d8968a4b1ef56ff55`.

Sources: [profiled baseline](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-867),
[intermediate Bypass](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-875),
[Bypass `6f83a34`](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-885),
[remaining profile observations](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-892),
[saved-callchain breakdown](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-894).

A follow-up measurement for Claviron `7de9d4b` was requested in
[the next-run plan](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-893).
That 40-CPU follow-up completed in [report
900](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-900), now
retained in D above. It completed 170,258,499 requests in 300.099499 seconds, used
6,402.90 s server CPU and 2,465.837 s generator CPU, with zero wrk errors and a counter
residual of 1,149. Against `6f83a34`, throughput rose 3.64%, process CPU/request fell
3.71%, and sampled memmove/request fell from 1.866 to 1.114 µs (40.28%). The hot
`call_cleartext` copy operand fell from 6,888 to 2,728 bytes. Its source matches the
Claviron revision measured in F, but D uses different hardware, load and profiling; the
measurements remain separate.

[The saved context/metadata
analysis](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-901)
identifies approximately 2.284 sampled-clock µs/request in context insertion and
metadata/policy cleanup. This subset is not the full CPU difference, and overlapping
nested costs must not be added again.

## E — earlier macOS terminal results

These five-minute terminal records lack exact binary revisions and CPU/request
measurements. They are retained separately from the Linux sets.

| Target | Requests/s | Mean latency, ms | p99, ms | Errors |
|---|---:|---:|---:|---|
| Tokio TCP | 164,260.94 | 6.21 | 8.272 | 0 |
| Hyper | 163,643.34 | 6.25 | 8.346 | 0 |
| Axum | 164,624.09 | 6.21 | 8.253 | **18 read** |
| Claviron Bypass | 154,782.09 | 6.60 | 8.333 | 0 |

Axum's result is not error-free. A separate Bypass `-t16 -c1024 -d5m` run
reported 151,518.31 requests/s, 2.071 ms p99 and **787 connect errors**.
Its active concurrency was not independently confirmed; it is not a valid
clean 1,024-connection comparison or evidence of a latency improvement.

[Original macOS record](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-831).

## Reproducing a series

Build with `python3 scripts/build.py --native` and retain the source revision,
resolved lockfile, compiler version, feature selection and executable hash.
Use the same wrk binary, thread/connection counts and duration for every
target in the intended comparison. Record server topology separately from
generator threads. Start one target at a time and validate its response before
loading `/json`; `/deno` is only the separate Deno reference's route.

Record all error counters, server CPU/request and the actual generator
duration. Preserve failures rather than ranking only successful completions.
Add another matched measurement as a new set; do not overwrite historical
results or treat a different load as a repeat of an earlier one.
