use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use std::ffi::OsString;

use brk::{
    Acceptance, CancellationToken, ConnectionPlan, Counts, Observer, Phase, PhaseStats, Plan,
    Stats, Target,
};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use super::cpu::{Probe, Snapshot};
use super::outcome::classify;

#[derive(Parser)]
#[command(
    name = "brk-observe",
    about = "Record brk SLA outcomes and Linux process CPU at phase boundaries"
)]
struct Cli {
    #[command(subcommand)]
    test: Test,
}

/// The two brk tests, named as `brk` names them. The request test is the default.
#[derive(Subcommand)]
enum Test {
    /// Raise concurrent requests per connection over connections that stay open.
    #[command(alias = "req")]
    Requests(RequestArgs),
    /// Raise concurrent connections, each carrying a fixed number of requests.
    #[command(alias = "conn")]
    Connections(ConnectionArgs),
}

#[derive(clap::Args)]
struct Common {
    url: String,
    /// Connect to this address instead of resolving the URL's host.
    #[arg(long, value_name = "ADDRESS")]
    connect: Option<std::net::SocketAddr>,
    /// For https://, trust the authorities in this PEM file.
    #[arg(long, value_name = "FILE")]
    cacert: Option<PathBuf>,
    /// For https://, make every handshake a full one instead of resuming.
    #[arg(long)]
    no_resume: bool,
    /// Speak HTTP/2, as `brk --http2` does.
    #[arg(long)]
    http2: bool,
    /// For https://, offer only this TLS version: 1.2 or 1.3.
    #[arg(long, value_name = "VERSION")]
    tls_version: Option<String>,
    /// For https://, offer only this key exchange group.
    #[arg(long, value_name = "NAME")]
    tls_group: Option<String>,
    /// For https://, offer only this cipher suite.
    #[arg(long, value_name = "NAME")]
    tls_cipher: Option<String>,
    /// Serving PID(s), repeated or comma-separated; exclude the supervisor.
    #[arg(long, value_delimiter = ',', required = true)]
    server_pid: Vec<u32>,
    /// Supervisor PID, reported separately from serving CPU.
    #[arg(long)]
    master_pid: Option<u32>,
    /// New JSONL output file; existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Client worker threads; by default one for every CPU the process may
    /// run on, never more than there are connections.
    #[arg(long, value_name = "COUNT")]
    threads: Option<usize>,
    #[arg(long, value_parser = humantime::parse_duration)]
    phase: Duration,
    #[arg(long, value_parser = humantime::parse_duration)]
    timeout: Duration,
    #[arg(long, default_value_t = 99.99)]
    accept: f64,
    #[arg(long)]
    expect_body: Option<String>,
    /// Validate the plan and process counters without resolving or contacting the target.
    #[arg(long)]
    check_only: bool,
}

#[derive(clap::Args)]
struct RequestArgs {
    #[command(flatten)]
    common: Common,
    #[arg(long)]
    conn: usize,
    #[arg(long)]
    start: Option<usize>,
    #[arg(long)]
    pace: usize,
    /// Optional ceiling in requests per connection, not total requests.
    #[arg(long = "max")]
    max_concurrency: Option<usize>,
}

#[derive(clap::Args)]
struct ConnectionArgs {
    #[command(flatten)]
    common: Common,
    /// Initial concurrent connections; defaults to pace.
    #[arg(long)]
    start: Option<usize>,
    /// Additional concurrent connections in each phase.
    #[arg(long)]
    pace: usize,
    /// Exact number of requests each connection carries.
    #[arg(long)]
    requests: usize,
    /// Optional ceiling in concurrent connections.
    #[arg(long = "max")]
    max_owners: Option<usize>,
}

/// `brk-observe <url> ...` is the request test, as `brk <url> ...` is.
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Cli, clap::Error> {
    let mut arguments: Vec<OsString> = arguments.into_iter().collect();
    let named = arguments.get(1).and_then(|first| first.to_str()).is_some_and(|first| {
        matches!(
            first,
            "requests" | "req" | "connections" | "conn" | "help" | "-h" | "--help"
        )
    });
    if !named && arguments.len() > 1 {
        arguments.insert(1, "requests".into());
    }
    Cli::try_parse_from(arguments)
}

/// The selected test's validated plan.
#[derive(Clone, Copy)]
enum Selected {
    Requests(Plan),
    Connections(ConnectionPlan),
}

fn counts(counts: Counts) -> Value {
    json!({"sent": counts.sent, "attempted": counts.attempted, "successful": counts.successful,
        "failed": counts.failed(), "timeout": counts.timeout, "transport": counts.transport,
        "abandoned": counts.abandoned, "connect": counts.connect, "connect_timeout": counts.connect_timeout,
        "status": counts.status, "body": counts.body})
}

fn phase(phase: &Phase) -> Value {
    json!({"index": phase.index, "connections": phase.connections,
        "requests_per_connection": phase.requests_per_connection,
        "total_concurrency": phase.total_concurrency, "duration_seconds": phase.duration.as_secs_f64()})
}

fn per_request(seconds: f64, count: u64) -> Option<f64> {
    (count > 0).then(|| seconds * 1e6 / count as f64)
}

struct Measurements {
    probe: Probe,
    before: Option<Snapshot>,
    writer: BufWriter<File>,
    stop: CancellationToken,
    acceptance: Acceptance,
    error: Option<io::Error>,
}

impl Measurements {
    fn record(&mut self, record: &Value) -> io::Result<()> {
        serde_json::to_writer(&mut self.writer, record).map_err(io::Error::other)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    fn fail(&mut self, error: io::Error) {
        self.error.get_or_insert(error);
        self.stop.cancel();
    }

    fn finish_phase(&mut self, stats: &PhaseStats) -> io::Result<()> {
        // Capture before formatting or writing the phase record. All admitted attempts have drained.
        let after = self.probe.capture()?;
        let before = self
            .before
            .take()
            .ok_or_else(|| io::Error::other("missing phase-start CPU sample"))?;
        let cpu = self.probe.report(&before, &after)?;
        let measured = json!({"event": "phase_finished", "phase": phase(&stats.phase),
            "elapsed_seconds": stats.elapsed.as_secs_f64(), "counts": counts(stats.counts),
            "connections_at_target": stats.connections_at_target, "completed": stats.completed,
            "cancelled": stats.cancelled, "accepted": stats.accepted(self.acceptance),
            "success_ratio": (stats.counts.attempted > 0).then(|| stats.counts.successful as f64 / stats.counts.attempted as f64),
            "successful_requests_per_second": stats.requests_per_sec(),
            "request_latency": {
                "completed_responses": stats.latency.responses,
                "total_nanoseconds": stats.latency.total_nanos.to_string(),
                "mean_ms": stats.latency.mean_millis()
            },
            "load": {
                "active_seconds": stats.load.active_elapsed.as_secs_f64(),
                "drain_seconds": stats.drain_elapsed().as_secs_f64(),
                "mean_outstanding": stats.load.mean_outstanding(),
                "target_fraction": stats.load.target_fraction(stats.phase.total_concurrency),
                "connection_target_fraction": stats.load.connection_target_fraction(stats.phase.connections),
                "outstanding_request_nanoseconds": stats.load.outstanding_request_nanos.to_string(),
                "connection_nanoseconds_at_target": stats.load.connection_nanos_at_target.to_string(),
                "latest_connection_first_target_seconds": stats.load.latest_first_target.map(|time| time.as_secs_f64()),
                "connect_attempts": stats.load.connect_attempts,
                "connect_seconds": stats.load.connect_nanos as f64 / 1e9,
                "write_blocked": stats.load.write_blocked,
                "max_send_delay_seconds": stats.load.max_send_delay.as_secs_f64(),
                "max_deadline_lateness_seconds": stats.load.max_deadline_lateness.as_secs_f64()
            },
            "tls": stats.tls.map(|tls| json!({
                "version": tls.version, "cipher": tls.cipher, "group": tls.group
            })),
            "setups": stats.setups.map(|setups| json!({
                "attempted": setups.attempted, "established": setups.established,
                "failed": setups.failed, "timeout": setups.timeout, "resumed": setups.resumed,
                "established_per_second": stats.connections_per_sec(),
                "setup_mean_ms": setups.mean_millis(),
                "setup_max_seconds": setups.longest.as_secs_f64()
            })),
            "serving_cpu_us_per_established_connection": stats.setups
                .and_then(|setups| per_request(cpu.serving.total_seconds, setups.established)),
            "serving_cpu_us_per_attempt": per_request(cpu.serving.total_seconds, stats.counts.attempted),
            "serving_cpu_us_per_success": per_request(cpu.serving.total_seconds, stats.counts.successful),
            "total_runtime_cpu_us_per_success": per_request(cpu.total_runtime.total_seconds, stats.counts.successful),
            "client_cpu_us_per_success": per_request(cpu.client.total_seconds, stats.counts.successful),
            "cpu": cpu});
        self.record(&measured)?;
        if let Some(tls) = &stats.tls {
            println!(
                "TLS: version={} cipher={} group={}",
                tls.version,
                tls.cipher,
                tls.group.unwrap_or("none")
            );
        }
        if let Some(setups) = &stats.setups {
            println!(
                "Connections: established={} of {} attempts, failed={}, timeout={}, resumed={}",
                setups.established, setups.attempted, setups.failed, setups.timeout, setups.resumed
            );
        }
        match stats.latency.mean_millis() {
            Some(mean) => println!(
                "Request average: {mean:.3} ms over {} complete responses (TCP setup excluded)",
                stats.latency.responses
            ),
            None => println!("Request average: n/a (no complete response)"),
        }
        println!(
            "Phase {}: target={}, mean-outstanding={:.2}, target-coverage={:.2}%, accepted={}, successful={}, failed={}",
            stats.phase.index,
            stats.phase.total_concurrency,
            stats.load.mean_outstanding(),
            stats.load.target_fraction(stats.phase.total_concurrency) * 100.0,
            stats.accepted(self.acceptance),
            stats.counts.successful,
            stats.counts.failed()
        );
        Ok(())
    }
}

impl Observer for Measurements {
    fn phase_started(&mut self, started: &Phase) {
        if self.error.is_some() {
            return;
        }
        // Flush metadata while workers are still gated; output work is outside the CPU sample window.
        let result = self
            .record(&json!({"event": "phase_started", "phase": phase(started)}))
            .and_then(|()| self.probe.capture());
        match result {
            Ok(before) => self.before = Some(before),
            Err(error) => self.fail(error),
        }
    }

    fn phase_finished(&mut self, stats: &PhaseStats) {
        if self.error.is_none()
            && let Err(error) = self.finish_phase(stats)
        {
            self.fail(error);
        }
    }
}

fn boundary(stats: &Stats) -> Value {
    json!({"cpu_workers": stats.cpu_workers, "phases": stats.phases, "total_counts": counts(stats.total),
        "last_accepted": stats.last_accepted.as_ref().map(|s| phase(&s.phase)),
        "last_tested": stats.last_phase.as_ref().map(|s| phase(&s.phase))})
}

pub fn execute() -> io::Result<ExitCode> {
    let cli = parse(std::env::args_os()).unwrap_or_else(|error| error.exit());
    let (args, selected) = match cli.test {
        Test::Requests(test) => {
            let plan = Plan {
                conn: test.conn,
                start: test.start.unwrap_or(test.pace),
                pace: test.pace,
                phase_duration: test.common.phase,
                timeout: test.common.timeout,
                accept: test.common.accept,
                max_concurrency: test.max_concurrency,
                threads: test.common.threads,
            };
            (test.common, Selected::Requests(plan))
        }
        Test::Connections(test) => {
            let plan = ConnectionPlan {
                start: test.start.unwrap_or(test.pace),
                pace: test.pace,
                requests: test.requests,
                phase_duration: test.common.phase,
                timeout: test.common.timeout,
                accept: test.common.accept,
                max_owners: test.max_owners,
                threads: test.common.threads,
            };
            (test.common, Selected::Connections(plan))
        }
    };
    let acceptance = match selected {
        Selected::Requests(plan) => plan.validate(),
        Selected::Connections(plan) => plan.validate(),
    }
    .map_err(io::Error::other)?;
    let mut target = Target::new(&args.url).map_err(io::Error::other)?;
    if let Some(address) = args.connect {
        target = target.connect_to(address);
    }
    if let Some(path) = &args.cacert {
        target = target
            .trust_pem(&std::fs::read(path)?)
            .map_err(io::Error::other)?;
    }
    if args.no_resume {
        target = target.without_resumption();
    }
    if args.http2 {
        target = target.http2();
    }
    if let Some(version) = &args.tls_version {
        target = target.tls_version(version).map_err(io::Error::other)?;
    }
    if let Some(group) = &args.tls_group {
        target = target.tls_group(group).map_err(io::Error::other)?;
    }
    if let Some(cipher) = &args.tls_cipher {
        target = target.tls_cipher(cipher).map_err(io::Error::other)?;
    }
    let probe = Probe::new(&args.server_pid, args.master_pid)?;
    let placement = probe.placement()?;
    let writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args.output)?,
    );
    let stop = CancellationToken::new();
    let mut measurements = Measurements {
        probe,
        before: None,
        writer,
        stop: stop.clone(),
        acceptance,
        error: None,
    };
    // What each test raises. In the connection test `start`, `pace` and the
    // ceiling count concurrent connections.
    let ramp = match selected {
        Selected::Requests(plan) => json!({"test": "requests", "conn": plan.conn,
            "start": plan.start, "pace": plan.pace,
            "max_requests_per_connection": plan.max_concurrency}),
        Selected::Connections(plan) => json!({"test": "connections",
            "start": plan.start, "pace": plan.pace,
            "requests_per_connection": plan.requests, "max_connections": plan.max_owners}),
    };
    measurements.record(&json!({"event": "plan", "schema_version": 5, "url": args.url,
        "request_latency_includes_connection_setup": false,
        "server_pids": args.server_pid, "master_pid": args.master_pid, "client_pid": std::process::id(),
        "client_threads": args.threads, "cpus": placement,
        "ramp": ramp, "phase_seconds": args.phase.as_secs_f64(),
        "timeout_seconds": args.timeout.as_secs_f64(), "accept_percent": args.accept,
        "http2": args.http2, "tls_resumption": !args.no_resume,
        "tls_offer": {"version": args.tls_version, "group": args.tls_group, "cipher": args.tls_cipher},
        "expect_body": args.expect_body,
        "check_only": args.check_only}))?;
    if let Some(body) = args.expect_body {
        target = target.expect_body(body.into_bytes());
    }
    if args.check_only {
        let before = measurements.probe.capture()?;
        let after = measurements.probe.capture()?;
        let cpu = measurements.probe.report(&before, &after)?;
        measurements
            .record(&json!({"event": "check_only", "cpu": cpu, "traffic_started": false}))?;
        println!("Plan and process CPU counters validated; no target traffic started");
        return Ok(ExitCode::SUCCESS);
    }
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.cancel()).map_err(io::Error::other)?;
    let result = match selected {
        Selected::Requests(plan) => brk::run(&target, plan, &stop, &mut measurements),
        Selected::Connections(plan) => {
            brk::run_connections(&target, plan, &stop, &mut measurements)
        }
    };
    let stats = match &result {
        Ok(done) => &done.stats,
        Err(broken) => &broken.stats,
    };
    if let Some(error) = measurements.error.take() {
        measurements.record(&json!({"event": "result", "outcome": "measurement_error",
            "observed_sla_boundary": false, "reason": error.to_string(), "boundary": boundary(stats)}))?;
        return Err(error);
    }
    let (label, code, reason) = match &result {
        Ok(_) => ("ceiling_reached", 0, None),
        Err(broken) => {
            let (label, code) = classify(broken);
            (label, code, Some(broken.reason.to_string()))
        }
    };
    measurements.record(&json!({"event": "result", "outcome": label,
        "observed_sla_boundary": result.as_ref().err().is_some_and(|broken| broken.has_capacity_boundary()),
        "reason": reason, "boundary": boundary(stats)}))?;
    println!("Result: {label}");
    Ok(ExitCode::from(code))
}

#[cfg(test)]
#[path = "../../tests/measure/observe.rs"]
mod tests;
