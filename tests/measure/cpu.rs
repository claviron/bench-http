use super::*;

fn stat(pid: u32, user: u64, system: u64, birth: u64) -> String {
    let mut fields = vec!["0".to_owned(); 20];
    fields[0] = "S".to_owned();
    fields[11] = user.to_string();
    fields[12] = system.to_string();
    fields[13] = "900000".to_owned();
    fields[14] = "800000".to_owned();
    fields[19] = birth.to_string();
    format!("{pid} (worker (with spaces)) {}", fields.join(" "))
}

#[test]
fn process_names_with_parentheses_do_not_shift_cpu_fields_or_include_children() {
    let before = Reading::parse(&stat(123, 10, 20, 7)).unwrap();
    let after = Reading::parse(&stat(123, 13, 25, 7)).unwrap();
    after.same_process(before).unwrap();
    assert_eq!(
        after.cpu.delta(before.cpu).unwrap(),
        Ticks { user: 3, system: 5 }
    );
}

#[test]
fn pid_reuse_and_decreasing_counters_invalidate_a_measurement() {
    let before = Reading::parse(&stat(123, 10, 20, 7)).unwrap();
    let reused = Reading::parse(&stat(123, 13, 25, 8)).unwrap();
    assert!(reused.same_process(before).is_err());
    let restarted = Reading::parse(&stat(123, 9, 25, 7)).unwrap();
    assert!(restarted.cpu.delta(before.cpu).is_err());
}

#[test]
fn worker_cpu_is_summed_once_with_master_and_client_separate() {
    let roles = [Role::Serving, Role::Serving, Role::Master, Role::Client];
    let before: Vec<_> = (1..=4)
        .map(|pid| Reading::parse(&stat(pid, 10, 20, 7)).unwrap())
        .collect();
    let after: Vec<_> = (1..=4)
        .map(|pid| Reading::parse(&stat(pid, 10 + u64::from(pid), 20, 7)).unwrap())
        .collect();
    let probe = Probe {
        identities: roles.into_iter().zip(before.iter().copied()).collect(),
        hz: 100,
    };
    let now = Instant::now();
    let snapshot = |readings| Snapshot {
        readings,
        began: now,
        finished: now,
    };
    let report = probe.report(&snapshot(before), &snapshot(after)).unwrap();
    assert!((report.serving.total_seconds - 0.03).abs() < f64::EPSILON);
    assert!((report.master.unwrap().total_seconds - 0.03).abs() < f64::EPSILON);
    assert!((report.total_runtime.total_seconds - 0.06).abs() < f64::EPSILON);
    assert!((report.client.total_seconds - 0.04).abs() < f64::EPSILON);
    assert_eq!(report.processes.len(), 4);
}

#[test]
fn a_client_on_serving_cpus_is_declared_and_a_separate_one_is_not() {
    let status = |cpus: &str| format!("Name:\tworker\nCpus_allowed:\tff\nCpus_allowed_list:\t{cpus}\n");
    let placed = |serving: &str, client: &str| {
        let (serving, master, client) = (status(serving), status("0-39"), status(client));
        Placement::read([
            (Role::Serving, 1, serving.as_str()),
            (Role::Master, 2, master.as_str()),
            (Role::Client, 3, client.as_str()),
        ])
        .unwrap()
    };
    // The supervisor's CPUs are recorded and do not decide the answer.
    let apart = placed("0-31", "32-39");
    assert!(!apart.client_shares_serving_cpus);
    assert_eq!(&*apart.processes[2].cpus, "32-39");
    assert!(placed("0-31,36", "32-39").client_shares_serving_cpus);
    assert!(placed("0-39", "0-39").client_shares_serving_cpus);
    assert!(Placement::read([(Role::Client, 3, "Name:\tworker\n")]).is_err());
}
