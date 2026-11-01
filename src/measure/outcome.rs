use brk::{BreakReason, Broken};

pub(super) fn classify(broken: &Broken) -> (&'static str, u8) {
    match &broken.reason {
        BreakReason::Acceptance if !broken.has_capacity_boundary() => ("initial_load_rejected", 1),
        BreakReason::Acceptance => ("sla_broken", 1),
        BreakReason::Cancelled => ("cancelled", 130),
        BreakReason::Unfulfilled => ("load_unfulfilled", 2),
        BreakReason::Protocol(_) => ("protocol_error", 2),
        BreakReason::Io(_) if broken.connection_failed() => ("connection_error", 2),
        BreakReason::Io(_) => ("io_error", 2),
        BreakReason::Resources(_) | BreakReason::Worker(_) => ("client_error", 2),
        BreakReason::Invalid(_) | BreakReason::Overflow => ("plan_error", 2),
    }
}

#[cfg(test)]
#[path = "../../tests/measure/outcome.rs"]
mod tests;
