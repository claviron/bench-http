use std::time::Duration;

use brk::{Counts, LatencyStats, LoadStats, Phase, PhaseStats, Stats};

use super::*;

fn measured() -> PhaseStats {
    PhaseStats {
        phase: Phase {
            index: 0,
            connections: 1,
            requests_per_connection: 1,
            total_concurrency: 1,
            duration: Duration::from_secs(1),
        },
        counts: Counts {
            attempted: 1,
            successful: 1,
            ..Counts::default()
        },
        elapsed: Duration::from_secs(1),
        connections_at_target: 1,
        load: LoadStats::default(),
        latency: LatencyStats::default(),
        completed: true,
        cancelled: false,
    }
}

#[test]
fn a_first_phase_rejection_is_not_a_passing_failing_capacity_bracket() {
    let mut broken = Broken {
        reason: BreakReason::Acceptance,
        stats: Box::default(),
    };
    assert_eq!(classify(&broken), ("initial_load_rejected", 1));
    assert!(!broken.has_capacity_boundary());
    broken.stats.last_accepted = Some(measured());
    assert_eq!(classify(&broken), ("sla_broken", 1));
    assert!(broken.has_capacity_boundary());
}

#[test]
fn endpoint_setup_failure_is_not_mislabeled_as_a_client_resource_limit() {
    let mut phase = measured();
    phase.counts = Counts {
        attempted: 1,
        connect: 1,
        ..Counts::default()
    };
    let broken = Broken {
        reason: std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into(),
        stats: Box::new(Stats {
            last_phase: Some(phase),
            ..Stats::default()
        }),
    };
    assert_eq!(classify(&broken), ("connection_error", 2));
    assert!(!broken.has_capacity_boundary());
    let broken = Broken {
        reason: BreakReason::Resources("request slots"),
        stats: Box::default(),
    };
    assert_eq!(classify(&broken), ("client_error", 2));
}
