use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use brk::{
    Acceptance, BreakReason, CancellationToken, Counts, Observer, Phase, PhaseStats, Plan, Stats,
    Target,
};
use clap::Parser;
use serde_json::{Value, json};

use super::cpu::{Probe, Snapshot};

#[derive(Parser)]
#[command(
    name = "brk-observe",
    about = "Record brk SLA outcomes and Linux process CPU at phase boundaries"
)]
struct Args {
    url: String,
    /// Serving PID(s), repeated or comma-separated; exclude the supervisor.
    #[arg(long, value_delimiter = ',', required = true)]
    server_pid: Vec<u32>,
    /// Supervisor PID, reported separately from serving CPU.
    #[arg(long)]
    master_pid: Option<u32>,
    /// New JSONL output file; existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    conn: usize,
    #[arg(long)]
    start: Option<usize>,
    #[arg(long)]
    pace: usize,
    #[arg(long, value_parser = humantime::parse_duration)]
    phase: Duration,
    #[arg(long, value_parser = humantime::parse_duration)]
    timeout: Duration,
    #[arg(long, default_value_t = 99.99)]
    accept: f64,
    /// Optional ceiling in requests per connection, not total requests.
    #[arg(long = "max")]
    max_concurrency: Option<usize>,
    #[arg(long)]
    expect_body: Option<String>,
    /// Validate the plan and process counters without resolving or contacting the target.
    #[arg(long)]
    check_only: bool,
}

fn counts(counts: Counts) -> Value {
    json!({"sent": counts.sent, "attempted": counts.attempted, "successful": counts.successful,
        "failed": counts.failed(), "timeout": counts.timeout, "transport": counts.transport,
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
            "serving_cpu_us_per_attempt": per_request(cpu.serving.total_seconds, stats.counts.attempted),
            "serving_cpu_us_per_success": per_request(cpu.serving.total_seconds, stats.counts.successful),
            "total_runtime_cpu_us_per_success": per_request(cpu.total_runtime.total_seconds, stats.counts.successful),
            "client_cpu_us_per_success": per_request(cpu.client.total_seconds, stats.counts.successful),
            "cpu": cpu});
        self.record(&measured)?;
        println!(
            "Phase {}: {} outstanding, accepted={}, successful={}, failed={}",
            stats.phase.index,
            stats.phase.total_concurrency,
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

fn outcome(reason: &BreakReason) -> (&'static str, u8) {
    match reason {
        BreakReason::Acceptance => ("sla_broken", 1),
        BreakReason::Cancelled => ("cancelled", 130),
        BreakReason::Unfulfilled => ("client_unfulfilled", 2),
        BreakReason::Protocol(_) => ("protocol_error", 2),
        BreakReason::Io(_) | BreakReason::Resources(_) | BreakReason::Worker(_) => {
            ("client_error", 2)
        }
        BreakReason::Invalid(_) | BreakReason::Overflow => ("plan_error", 2),
    }
}

pub fn execute() -> io::Result<ExitCode> {
    let args = Args::parse();
    let plan = Plan {
        conn: args.conn,
        start: args.start.unwrap_or(args.pace),
        pace: args.pace,
        phase_duration: args.phase,
        timeout: args.timeout,
        accept: args.accept,
        max_concurrency: args.max_concurrency,
    };
    let acceptance = plan.validate().map_err(io::Error::other)?;
    let mut target = Target::new(&args.url).map_err(io::Error::other)?;
    let probe = Probe::new(&args.server_pid, args.master_pid)?;
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
    measurements.record(&json!({"event": "plan", "schema_version": 1, "url": args.url,
        "server_pids": args.server_pid, "master_pid": args.master_pid, "client_pid": std::process::id(),
        "conn": plan.conn, "start": plan.start, "pace": plan.pace, "phase_seconds": plan.phase_duration.as_secs_f64(),
        "timeout_seconds": plan.timeout.as_secs_f64(), "accept_percent": plan.accept,
        "max_requests_per_connection": plan.max_concurrency, "expect_body": args.expect_body,
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
    let result = brk::run(&target, plan, &stop, &mut measurements);
    let stats = match &result {
        Ok(done) => &done.stats,
        Err(broken) => &broken.stats,
    };
    if let Some(error) = measurements.error.take() {
        measurements.record(&json!({"event": "result", "outcome": "measurement_error",
            "server_capacity_claim": false, "reason": error.to_string(), "boundary": boundary(stats)}))?;
        return Err(error);
    }
    let (label, code, reason) = match &result {
        Ok(_) => ("ceiling_reached", 0, None),
        Err(broken) => {
            let (label, code) = outcome(&broken.reason);
            (label, code, Some(broken.reason.to_string()))
        }
    };
    measurements.record(&json!({"event": "result", "outcome": label,
        "server_capacity_claim": label == "sla_broken", "reason": reason, "boundary": boundary(stats)}))?;
    println!("Result: {label}");
    Ok(ExitCode::from(code))
}
