use std::collections::HashSet;
use std::fs;
use std::io;
use std::time::Instant;

use serde::Serialize;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ticks {
    user: u64,
    system: u64,
}

impl Ticks {
    fn delta(self, before: Self) -> io::Result<Self> {
        Ok(Self {
            user: self
                .user
                .checked_sub(before.user)
                .ok_or_else(|| invalid("user CPU counter decreased"))?,
            system: self
                .system
                .checked_sub(before.system)
                .ok_or_else(|| invalid("system CPU counter decreased"))?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Reading {
    pid: u32,
    start_ticks: u64,
    cpu: Ticks,
}

impl Reading {
    fn parse(stat: &str) -> io::Result<Self> {
        let (identity, fields) = stat
            .rsplit_once(')')
            .ok_or_else(|| invalid("missing process name"))?;
        let (pid, _) = identity
            .split_once('(')
            .ok_or_else(|| invalid("missing process ID"))?;
        let fields: Vec<_> = fields.split_whitespace().collect();
        let number = |index: usize| {
            fields
                .get(index)
                .ok_or_else(|| invalid("incomplete process stat"))?
                .parse::<u64>()
                .map_err(|_| invalid("invalid process counter"))
        };
        if matches!(fields.first().copied(), Some("Z" | "X")) {
            return Err(invalid("measured process exited"));
        }
        Ok(Self {
            pid: pid
                .trim()
                .parse()
                .map_err(|_| invalid("invalid process ID"))?,
            // The suffix starts at field 3. Fields 16/17 are child CPU and are deliberately unused.
            cpu: Ticks {
                user: number(11)?,
                system: number(12)?,
            },
            start_ticks: number(19)?,
        })
    }

    fn read(pid: u32) -> io::Result<Self> {
        Self::parse(&fs::read_to_string(format!("/proc/{pid}/stat"))?)
    }

    fn same_process(self, other: Self) -> io::Result<()> {
        if self.pid != other.pid || self.start_ticks != other.start_ticks {
            return Err(invalid("measured process identity changed"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    Serving,
    Master,
    Client,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CpuTime {
    pub user_seconds: f64,
    pub system_seconds: f64,
    pub total_seconds: f64,
}

impl CpuTime {
    fn from_ticks(cpu: Ticks, hz: u64) -> Self {
        let user_seconds = cpu.user as f64 / hz as f64;
        let system_seconds = cpu.system as f64 / hz as f64;
        Self {
            user_seconds,
            system_seconds,
            total_seconds: user_seconds + system_seconds,
        }
    }

    fn add(&mut self, other: Self) {
        self.user_seconds += other.user_seconds;
        self.system_seconds += other.system_seconds;
        self.total_seconds += other.total_seconds;
    }
}

pub struct Snapshot {
    readings: Vec<Reading>,
    began: Instant,
    finished: Instant,
}

#[derive(Serialize)]
pub struct ProcessCpu {
    pid: u32,
    start_ticks: u64,
    role: Role,
    user_ticks: u64,
    system_ticks: u64,
    cpu: CpuTime,
}

#[derive(Serialize)]
pub struct Report {
    pub serving: CpuTime,
    pub master: Option<CpuTime>,
    pub client: CpuTime,
    pub total_runtime: CpuTime,
    pub window_seconds: f64,
    start_sample_seconds: f64,
    finish_sample_seconds: f64,
    ticks_per_second: u64,
    processes: Vec<ProcessCpu>,
}

pub struct Probe {
    identities: Vec<(Role, Reading)>,
    hz: u64,
}

impl Probe {
    pub fn new(serving: &[u32], master: Option<u32>) -> io::Result<Self> {
        // SAFETY: sysconf with _SC_CLK_TCK has no pointer or ownership preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let hz = u64::try_from(hz)
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| invalid("cannot determine process CPU clock frequency"))?;
        let mut seen = HashSet::new();
        let mut identities = Vec::new();
        let mut add = |role, pid| {
            if pid == 0 || !seen.insert(pid) {
                return Err(invalid(
                    "process IDs must be positive and distinct across all roles",
                ));
            }
            identities.push((role, Reading::read(pid)?));
            Ok(())
        };
        if serving.is_empty() {
            return Err(invalid("at least one serving process is required"));
        }
        for pid in serving {
            add(Role::Serving, *pid)?;
        }
        if let Some(pid) = master {
            add(Role::Master, pid)?;
        }
        add(Role::Client, std::process::id())?;
        Ok(Self { identities, hz })
    }

    pub fn capture(&self) -> io::Result<Snapshot> {
        let began = Instant::now();
        let readings = self
            .identities
            .iter()
            .map(|(_, identity)| {
                let reading = Reading::read(identity.pid)?;
                reading.same_process(*identity)?;
                Ok(reading)
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Snapshot {
            readings,
            began,
            finished: Instant::now(),
        })
    }

    pub fn report(&self, before: &Snapshot, after: &Snapshot) -> io::Result<Report> {
        let mut serving = CpuTime::default();
        let mut master = None;
        let mut client = CpuTime::default();
        let mut processes = Vec::with_capacity(self.identities.len());
        for ((role, _), (before, after)) in self
            .identities
            .iter()
            .zip(before.readings.iter().zip(&after.readings))
        {
            after.same_process(*before)?;
            let delta = after.cpu.delta(before.cpu)?;
            let cpu = CpuTime::from_ticks(delta, self.hz);
            match role {
                Role::Serving => serving.add(cpu),
                Role::Master => master = Some(cpu),
                Role::Client => client = cpu,
            }
            processes.push(ProcessCpu {
                pid: after.pid,
                start_ticks: after.start_ticks,
                role: *role,
                user_ticks: delta.user,
                system_ticks: delta.system,
                cpu,
            });
        }
        let mut total_runtime = serving;
        if let Some(cpu) = master {
            total_runtime.add(cpu);
        }
        Ok(Report {
            serving,
            master,
            client,
            total_runtime,
            window_seconds: after.finished.duration_since(before.finished).as_secs_f64(),
            start_sample_seconds: before.finished.duration_since(before.began).as_secs_f64(),
            finish_sample_seconds: after.finished.duration_since(after.began).as_secs_f64(),
            ticks_per_second: self.hz,
            processes,
        })
    }
}

#[cfg(test)]
#[path = "../../tests/measure/cpu.rs"]
mod tests;
