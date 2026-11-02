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

Native targets do not compile the optional Claviron dependency:

```sh
python3 scripts/build.py --native --target native
```

The manifest selects a local Claviron checkout with version requirement
`0.2.0-alpha.24`. This version is not published yet. Cargo reads the checkout
from the dependency's `path` and checks its declared version.

```sh
python3 scripts/build.py --native
```

The default target is `all`. `--target claviron` builds only the three Claviron
entries; a binary name selects one target. `--supervisor` exposes `--workers` on
Claviron entries. The script reports the source location, version and Git revision
selected by Cargo; it does not rewrite the manifest or generate source overrides.

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
scripts/run.sh claviron-other --config-dir config --name bench-http --fleet bhttp
```

With `--supervisor` selected at build time, `--workers 0` is standalone and a
positive count starts supervised worker processes. The counter remains local
to each process. Mio's `--threads N` selects Poll threads inside one process.

The launcher supplies `BENCH_HTTP_EXECUTION=shared` when unset. The top-level
`plain_execution` field in `config/http.yaml` reads that value through the config
API. Direct binary launches must supply the variable explicitly.

### Axum execution comparison

Build `claviron-axum` once with supervisor support. Use that same binary for all
three cases:

```sh
python3 scripts/build.py --native --target claviron-axum --supervisor
```

Start each case separately, stopping the previous server first:

```sh
# Standalone, shared Tokio runtime.
BENCH_HTTP_EXECUTION=shared scripts/run.sh claviron-axum --workers 0

# Multiple processes, each using its shared Tokio runtime.
BENCH_HTTP_EXECUTION=shared scripts/run.sh claviron-axum --workers 4

# Same process count, each with a listener-owned HTTP executor.
BENCH_HTTP_EXECUTION=current_thread scripts/run.sh claviron-axum --workers 4
```

The two supervised cases must use the same worker count; four is an example.
Keep the address, backlog, workload and wrk parameters identical in all cases.
The first two show the effect of worker count with the default shared runtimes;
the latter two compare HTTP execution at a fixed worker count. The first and
last show their combined effect.

Report serving-process CPU/request, requests/s, p99 and all error counters for
each case. Sum CPU time across the serving workers; report the supervisor's CPU
separately so HTTP execution cost and total process-management cost remain visible.

`shared` uses each serving process's main Tokio runtime. `current_thread` gives
each serving process one executor thread per plain HTTP listener for accept,
connections, HostPipeline and Axum handler polling. This configuration declares
one listener. Other services keep the main runtime. The execution mode applies
when the listener's runnable starts; reload does not migrate existing connections.
No performance gain is assumed.

All targets request the same declared TCP backlog and enable TCP_NODELAY. Native
listeners default to 2048 when BENCH_BACKLOG is absent; Claviron requires the
exported value. Mio's `--backlog` overrides its environment value. The OS may cap
the queue: verify the effective backlog, especially when connections are opened
in a burst. Backlog is not an active-request or keep-alive connection limit.
Shutdown drains owned work without a benchmark-specific termination timeout.

## Measure

### Acceptance capacity

The optional `brk` command uses the `break` source library to raise concurrent
load until an acceptance phase breaks. Its manifest path selects unreleased load
diagnostics; crates.io 0.1.0 does not contain them.

Both client binaries are gated by the optional `client` feature:

| Source | Binary | Purpose |
| --- | --- | --- |
| `src/client.rs` | `brk` | The break CLI, with phase outcomes and the tested acceptance boundary. |
| `src/measure/main.rs` | `brk-observe` | The library runner with phase-boundary Linux CPU measurements and JSONL output. |

Select a client explicitly with the Python build script. Neither client is part
of the default server group, and neither requires the Claviron feature. The
`brk` binary delegates to the library's CLI; `brk-observe` supplies its own
`Observer` to the same library runner.

```sh
python3 scripts/build.py --native --target brk
./target/release/brk http://127.0.0.1:8080/plain \
  --conn 4 --start 1 --pace 1 --phase 10s --timeout 2s --accept 99.99 \
  --expect-body 'Hello, World!'
```

`--conn` fixes the connection-owner count. Phase zero starts at `--start` concurrent
requests per connection; each phase adds `--pace`. Completed requests are replaced
immediately. There is no warmup or request-rate target. Plain HTTP/1.1 uses real
pipelining; a client-side outstanding request need not be executing on the server.
Counters reset each phase, with pending attempts drained under their original
deadlines. The report distinguishes the last accepted level, acceptance failure,
and client I/O/resource/protocol limits. `--max` optionally caps requests per
connection without claiming a breakpoint. Building `brk` leaves the default server
group unchanged and does not enable Claviron.

Library consumers can implement `Observer::phase_started` and `phase_finished`
to sample server CPU/RSS around each phase. End callbacks follow drain, including
failed phases. Keep server and generator resource measurements separate.

#### Phase CPU measurements on Linux

`brk-observe` uses the library's `Observer` callbacks and Linux process counters
to record each phase's SLA outcomes and CPU deltas in a new JSONL file:

```sh
python3 scripts/build.py --native --target brk-observe
./target/release/brk-observe http://127.0.0.1:8080/plain \
  --server-pid "$SERVER_PID" --output phases.jsonl \
  --conn 256 --start 1 --pace 1 --phase 10s --timeout 16ms \
  --accept 99.99 --max 16 --expect-body 'Hello, World!'
```

For a supervised server, supply every serving worker through repeated
`--server-pid` arguments or a comma-separated list, and identify the supervisor
with `--master-pid`. The caller supplies and owns this stable process set; the
client never discovers, starts or signals server processes. `--check-only`
validates the plan and PID counters without DNS or target traffic. It still
requires a new output filename.

For example, after identifying the serving worker PIDs and master PID of a
running supervised server:

```sh
./target/release/brk-observe http://127.0.0.1:8080/plain \
  --server-pid "$WORKER_PIDS" --master-pid "$MASTER_PID" \
  --output counter-check.jsonl --check-only \
  --conn 256 --start 1 --pace 1 --phase 10s --timeout 16ms \
  --accept 99.99 --max 16 --expect-body 'Hello, World!'
```

Here `WORKER_PIDS` is a comma-separated list. After validation, run the same
command without `--check-only`, using a different output filename, to apply
load. `--check-only` does not exercise traffic, deadlines or acceptance.

Each `phase_finished` record contains the phase's attempted/successful/error
counts, acceptance decision, successful requests/s, and serving, master, total
runtime and client CPU seconds. Serving CPU sums workers' own user+system time;
master and client are separate. Child CPU fields and an extra division by worker
count are never used. Per-process deltas, process birth counters and the system
clock-tick frequency are retained. A changed/missing PID, decreasing counter or
output failure cancels the run and invalidates CPU measurement rather than
reporting a server capacity limit.

Schema 2 adds observed mean outstanding, target coverage, connection time at full
depth, active/drain times, setup attempts/time, blocked writes and send/deadline
diagnostics under `load`. Raw nanosecond integrals are decimal strings, preserving
their integer precision. `connections_at_target` means first attainment per owner,
not sustained or simultaneous global load. No extra coverage threshold is applied.
Failed attempts distinguish timeout, transport, abandoned followers and TCP setup.
An initial rejection reports `initial_load_rejected`. `observed_sla_boundary`
requires a passing level followed by an SLA rejection; it makes no attribution to
the server alone and is not a confidence interval. Setup errors and local resource
failures have different outcome labels.

Schema 3 adds `request_latency` to each finished phase: `completed_responses`,
the exact `total_nanoseconds` as a decimal string, and `mean_ms` (`null` without
a complete response). The console also prints the mean for every phase. Timing
starts at request admission after TCP setup and ends after complete HTTP response
validation, including drain. It includes send backpressure and queueing, not only
handler execution. Fully decoded late or invalid replies contribute; incomplete
timeouts and abandoned attempts do not contribute zero-duration samples. Always
read the sample count and error counts with the mean. Connection setup remains
separate under `load`; the existing SLA/deadline behavior is unchanged.

Workers are gated before the start sample and drained before the finish sample.
PID reads are sequential, not atomic; records include both sampling spans and
the CPU accounting window. CPU precision follows the OS clock ticks. Client CPU
includes work inside that process between samples, including some observer
overhead; record formatting/output occurs outside the samples. Serving CPU
includes background work inside those processes. No latency percentile is
invented from the SLA deadline.

CPU per attempt and per successful response have explicit denominators; a zero
denominator produces `null`. Compare CPU efficiency at the same accepted phase
and account for errors rather than equating averages from different ramp lengths.
The final record distinguishes `sla_broken`, `ceiling_reached`, cancellation,
client/protocol failures and invalid CPU measurement. Exit codes are 0 for the
declared ceiling, 1 for SLA failure, 2 for measurement/client/protocol/plan errors,
and 130 for cancellation. The JSONL file retains completed phases and is never
overwritten.

Read the output as one JSON object per line:

| `event` | Meaning |
| --- | --- |
| `plan` | URL, load/SLA parameters, expected body and process roles. |
| `phase_started` | The next load level, before worker release. |
| `phase_finished` | Drained phase outcomes, acceptance and matching CPU samples. |
| `result` | Final outcome, total counts, last accepted and last tested phase. |
| `check_only` | Counter validation only; `traffic_started` is false. |

In each measured phase, `cpu.serving`, `cpu.master`, `cpu.client` and
`cpu.total_runtime` contain user, system and total CPU seconds. `cpu.processes`
retains each PID's role, birth counter and own CPU delta. A standalone target
has no master measurement. `connections_at_target` verifies whether every
connection owner established its requested pipeline depth.

For the example plan, 256 fixed connections and pipeline depths 1 through 16
produce phases of 256, 512, ..., 4096 outstanding requests. There are at most
16 ten-second admission windows per target, plus setup and drain; the first
failed phase ends the ramp. Successful responses must have the exact expected
body and complete within 16 ms at a ratio of at least 99.99%. Reaching the final
level successfully records a tested ceiling, not a discovered breakpoint.

To compare targets, start and validate one server, supply its serving PIDs, run
the client, retain the JSONL file, then stop that server before selecting the
next. Keep the URL/workload, connection count, load steps, deadline, acceptance,
backlog and descriptor limits identical. Declare each server's process/executor
topology and use a fresh file for every run. Compare the last accepted and first
rejected phase, then compare throughput and CPU at shared load levels. A client
resource, protocol or measurement error is not evidence of a server SLA limit.

### Fixed load

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
