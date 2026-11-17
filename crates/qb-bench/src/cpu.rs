//! Where the processor time of a run went: each core's busy share from `/proc/stat`, the
//! generator's own thread from `/proc/thread-self/stat`, and the server's process from
//! `/proc/<pid>/stat`.
//!
//! The generator runs on the same board as the server it loads, so its own processor time is time
//! the server did not get. A report shows the two apart: a server that peaks at 3,000 answers a
//! second while the generator takes a whole core is not a server that peaks at 3,000 on its own.
//!
//! The kernel counts in clock ticks (`USER_HZ`, 100 a second on every Linux board in use), so a
//! figure over one second is good to about one per cent of a core; a run's figures are over
//! seconds. Every reader gives `None` where the file cannot be read or does not parse: a figure
//! that is not known is left out, never made up.

use std::fs;
use std::time::Instant;

/// One core's time since boot, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreTimes {
    /// User, nice, system, irq, softirq and steal.
    pub busy: u64,
    /// `busy` and the idle and iowait time.
    pub total: u64,
}

/// The `cpuN` lines of `/proc/stat`, in the file's order; the summed `cpu` line is left out.
pub fn parse_cores(stat: &str) -> Vec<CoreTimes> {
    stat.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let number = name.strip_prefix("cpu")?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let values: Vec<u64> = fields.map_while(|f| f.parse().ok()).collect();
            // user nice system idle iowait irq softirq steal; kernels before 2.6.11 had fewer.
            let at = |i: usize| values.get(i).copied().unwrap_or(0);
            if values.len() < 4 {
                return None;
            }
            let busy = at(0) + at(1) + at(2) + at(5) + at(6) + at(7);
            let idle = at(3) + at(4);
            Some(CoreTimes {
                busy,
                total: busy + idle,
            })
        })
        .collect()
}

/// Each core's busy share between two readings, in per cent: `None` when the two do not have the
/// same cores. A core whose clock did not move (no tick between them) is 0.
pub fn busy_percent(before: &[CoreTimes], after: &[CoreTimes]) -> Option<Vec<f64>> {
    if before.len() != after.len() || before.is_empty() {
        return None;
    }
    Some(
        before
            .iter()
            .zip(after)
            .map(|(b, a)| {
                let total = a.total.saturating_sub(b.total);
                let busy = a.busy.saturating_sub(b.busy).min(total);
                if total == 0 {
                    0.0
                } else {
                    busy as f64 * 100.0 / total as f64
                }
            })
            .collect(),
    )
}

/// The user and system time of a `/proc/<pid>/stat` or `/proc/<pid>/task/<tid>/stat` line, in
/// clock ticks. The command name, in parentheses, may hold spaces and parentheses itself, so the
/// fields are counted from its last `)`.
pub fn task_ticks(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    // After the name: state is field 3, utime 14 and stime 15.
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

/// The kernel's clock ticks a second, from the auxiliary vector.
pub fn ticks_per_second() -> u64 {
    rustix::param::clock_ticks_per_second().max(1)
}

/// The cores as `/proc/stat` has them now.
pub fn read_cores() -> Option<Vec<CoreTimes>> {
    let cores = parse_cores(&fs::read_to_string("/proc/stat").ok()?);
    (!cores.is_empty()).then_some(cores)
}

/// The calling thread's processor time, in ticks.
pub fn thread_ticks() -> Option<u64> {
    task_ticks(&fs::read_to_string("/proc/thread-self/stat").ok()?)
}

/// Whose processor time is the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Server {
    /// Not known: no server figure.
    Unknown,
    /// This process, the generator's thread taken out: qberrypi running the generator itself.
    ThisProcess,
    /// Another process, all its threads.
    Pid(u32),
}

impl Server {
    /// The server's processor time now, in ticks, the generator's thread included where it is
    /// this process.
    fn ticks(self) -> Option<u64> {
        let path = match self {
            Server::Unknown => return None,
            Server::ThisProcess => "/proc/self/stat".to_owned(),
            Server::Pid(pid) => format!("/proc/{pid}/stat"),
        };
        task_ticks(&fs::read_to_string(path).ok()?)
    }
}

/// One reading of the clocks, taken on the generator's thread.
#[derive(Debug, Clone)]
pub struct Reading {
    pub at: Instant,
    pub cores: Option<Vec<CoreTimes>>,
    pub generator: Option<u64>,
    pub server: Option<u64>,
}

impl Reading {
    /// The clocks now; must be called on the generator's thread.
    pub fn take(server: Server) -> Reading {
        Reading {
            at: Instant::now(),
            cores: read_cores(),
            generator: thread_ticks(),
            server: server.ticks(),
        }
    }
}

/// The processor time between two readings.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuUse {
    /// Each core's busy share, in per cent of that core.
    pub cores_busy_pct: Option<Vec<f64>>,
    /// The generator's thread, in per cent of one core.
    pub generator_pct: Option<f64>,
    /// The server's process, in per cent of one core (over 100 for a server that uses several);
    /// the generator's thread is not in it.
    pub server_pct: Option<f64>,
}

/// What `before` and `after`, both taken on the generator's thread, say of the time between:
/// `ticks_per_sec` is the kernel's clock rate. Where `server` is this process, the generator's
/// ticks are taken out of the process's.
pub fn between(before: &Reading, after: &Reading, server: Server, ticks_per_sec: u64) -> CpuUse {
    let wall = after.at.saturating_duration_since(before.at).as_secs_f64();
    let pct = |ticks: u64| {
        (wall > 0.0).then(|| ticks as f64 * 100.0 / ticks_per_sec.max(1) as f64 / wall)
    };
    let delta = |a: Option<u64>, b: Option<u64>| Some(b?.saturating_sub(a?));
    let generator = delta(before.generator, after.generator);
    let mut server_ticks = delta(before.server, after.server);
    if server == Server::ThisProcess {
        server_ticks = server_ticks
            .zip(generator)
            .map(|(s, g)| s.saturating_sub(g));
    }
    CpuUse {
        cores_busy_pct: match (&before.cores, &after.cores) {
            (Some(b), Some(a)) => busy_percent(b, a),
            _ => None,
        },
        generator_pct: generator.and_then(pct),
        server_pct: server_ticks.and_then(pct),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const STAT: &str = "cpu  100 0 50 800 50 0 0 0 0 0\n\
                        cpu0 10 0 5 80 5 0 0 0 0 0\n\
                        cpu1 90 0 45 720 45 0 0 0 0 0\n\
                        intr 1 2 3\nctxt 99\n";

    #[test]
    fn the_cores_are_the_numbered_lines() {
        assert_eq!(
            parse_cores(STAT),
            [
                CoreTimes {
                    busy: 15,
                    total: 100
                },
                CoreTimes {
                    busy: 135,
                    total: 900
                }
            ]
        );
        // irq, softirq and steal are busy; iowait is idle.
        let busy = parse_cores("cpu3 1 2 3 4 5 6 7 8 9 10\n");
        assert_eq!(
            busy,
            [CoreTimes {
                busy: 1 + 2 + 3 + 6 + 7 + 8,
                total: 36
            }]
        );
        assert!(parse_cores("cpux 1 2 3 4\ncpu0 1 2\n").is_empty());
    }

    #[test]
    fn the_busy_share_is_per_core_between_two_readings() {
        let before = parse_cores(STAT);
        let after = [
            CoreTimes {
                busy: 15 + 50,
                total: 100 + 100,
            },
            CoreTimes {
                busy: 135,
                total: 900,
            },
        ];
        assert_eq!(busy_percent(&before, &after), Some(vec![50.0, 0.0]));
        assert_eq!(busy_percent(&before, &after[..1]), None);
        assert_eq!(busy_percent(&[], &[]), None);
    }

    #[test]
    fn a_tasks_ticks_are_counted_after_the_last_parenthesis() {
        let line = "1234 (a (b) c) S 1 1234 1234 0 -1 4194560 100 0 0 0 7 3 0 0 20 0 1 0 5 0 0";
        assert_eq!(task_ticks(line), Some(10));
        assert_eq!(task_ticks("1234 (cut) S 1"), None);
        assert_eq!(task_ticks("no name at all"), None);
    }

    #[test]
    fn this_threads_and_this_processs_clocks_can_be_read() {
        let before = Reading::take(Server::ThisProcess);
        assert!(before.generator.is_some() && before.server.is_some());
        assert!(before.cores.as_ref().is_some_and(|cores| !cores.is_empty()));
        assert!(ticks_per_second() >= 1);
    }

    #[test]
    fn the_server_of_this_process_leaves_the_generators_thread_out() {
        let at = Instant::now();
        let reading = |secs: u64, generator: u64, server: u64| Reading {
            at: at + Duration::from_secs(secs),
            cores: None,
            generator: Some(generator),
            server: Some(server),
        };
        let (before, after) = (reading(0, 0, 0), reading(2, 100, 300));
        let mine = between(&before, &after, Server::ThisProcess, 100);
        assert_eq!(mine.generator_pct, Some(50.0));
        assert_eq!(mine.server_pct, Some(100.0));
        assert_eq!(mine.cores_busy_pct, None);
        let other = between(&before, &after, Server::Pid(1), 100);
        assert_eq!(other.server_pct, Some(150.0));
        let unknown = Reading {
            server: None,
            ..reading(2, 100, 0)
        };
        assert_eq!(
            between(&before, &unknown, Server::Unknown, 100).server_pct,
            None
        );
    }
}
