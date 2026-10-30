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

Each set used sequential loads with the generator on the server's machine.
Tests used the system's existing scheduling and CPU affinity. There is one
observation per target/revision in each set, without a confidence interval.
Similar throughput does not establish equal CPU cost or a server capacity
ceiling. No matched five-target series has been recorded.

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

The latest Bypass run completed 164,283,762 requests in 300.107110 seconds,
with 6,416.54 s server CPU and 2,423.339 s generator CPU. Its counter residual
was 1,416, within the 2,048 possible in-flight requests.

Compared with the preceding Bypass capture, throughput rose 4.83% and
process CPU/request fell 7.14%. Against the original profiled Bypass, the
accumulated difference is +7.94% throughput and −11.51% process CPU/request.
The latest Bypass still uses 1.727× Hyper's process CPU/request in these
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

Latest Bypass source: `6f83a348f2004dc28bd0c6b3bf4f849dfb63b574`;
benchmark source remains `5a4950a5de736b2b60b1bdf66cf8eeac2a6fa3f7`.
Latest executable SHA-256:
`9fb9b9f177a891951d969ad7c8688145c4c7f95bb82e9b7d8968a4b1ef56ff55`.

Sources: [profiled baseline](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-867),
[intermediate Bypass](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-875),
[latest Bypass](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-885),
[remaining profile observations](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-892),
[saved-callchain breakdown](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-894).

A follow-up measurement for Claviron `7de9d4b` was requested in
[the next-run plan](https://git.claviron.dev/claviron/bench-http/issues/1#issuecomment-893).
No completed result for that revision was available at this report's cutoff
(`2026-10-30T03:36:15Z`); it does not replace the measured `6f83a34` row.

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
