//! How busy the board's cores and this process are, and how much memory the process holds: the
//! `cpu` and `memory` of `/api/v1/stats`.
//!
//! The supervisor's tick, every five seconds, calls [`sample`], which reads `/proc/stat`,
//! `/proc/self/stat` and `/proc/self/status` (three small files of the kernel's, microseconds on
//! the runtime thread) and keeps the readings of the last [`WINDOW`]. Nothing on the answer path
//! touches any of it: the statistics read what the last tick left, under a lock that only the
//! tick and a request of the statistics take, and the block set's and the cache's sizes, which
//! the resolver keeps as it goes, are read under the lock the statistics take for their counts.

use std::collections::VecDeque;
use std::fs;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use qb_bench::cpu::{CoreTimes, busy_percent, parse_cores, task_ticks, ticks_per_second};
use serde_json::{Value, json};

/// How far back the busy shares reach: the newest reading against the newest that is at least
/// this much older, or against the oldest kept while there is none yet.
pub const WINDOW: Duration = Duration::from_secs(10);

/// One reading of the clocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at: Instant,
    pub cores: Vec<CoreTimes>,
    /// This process's user and system time, in ticks.
    pub process: Option<u64>,
}

/// The readings of the last [`WINDOW`], and the resident memory of the last.
#[derive(Debug, Default)]
pub struct Usage {
    samples: VecDeque<Sample>,
    rss_kib: Option<u64>,
}

/// What the readings say.
#[derive(Debug, Clone, PartialEq)]
pub struct Figures {
    /// Each core's busy share, in per cent of that core; `None` before two readings.
    pub cores: Option<Vec<f64>>,
    /// This process, in per cent of one core (up to 100 times the cores).
    pub process: Option<f64>,
    /// The seconds between the two readings.
    pub window_secs: Option<f64>,
    pub rss_kib: Option<u64>,
}

impl Usage {
    /// Takes in `sample` and the resident memory, and lets go of the readings that the window no
    /// longer needs: all but the newest that is at least [`WINDOW`] older than this one.
    pub fn note(&mut self, sample: Sample, rss_kib: Option<u64>) {
        let newest = sample.at;
        self.samples.push_back(sample);
        while self.samples.len() > 2
            && newest.saturating_duration_since(self.samples[1].at) >= WINDOW
        {
            self.samples.pop_front();
        }
        self.rss_kib = rss_kib;
    }

    /// The busy shares between the oldest reading kept and the newest, with `ticks` a second.
    pub fn figures(&self, ticks: u64) -> Figures {
        let pair = match (self.samples.front(), self.samples.back()) {
            (Some(first), Some(last)) if self.samples.len() >= 2 => Some((first, last)),
            _ => None,
        };
        let secs = pair.map(|(a, b)| b.at.saturating_duration_since(a.at).as_secs_f64());
        Figures {
            cores: pair.and_then(|(a, b)| busy_percent(&a.cores, &b.cores)),
            process: pair.zip(secs).and_then(|((a, b), secs)| {
                let used = b.process?.checked_sub(a.process?)?;
                (secs > 0.0).then(|| used as f64 * 100.0 / ticks.max(1) as f64 / secs)
            }),
            window_secs: secs,
            rss_kib: self.rss_kib,
        }
    }
}

/// The resident memory of a `/proc/<pid>/status` text, in KiB.
pub fn rss_kib(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// This process's resident memory in KiB, read from `/proc/self/status` now; `None` where the file
/// cannot be read or says nothing of it.
pub fn read_rss_kib() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| rss_kib(&text))
}

/// The readings, for the tick to write and the statistics to read.
static USAGE: Mutex<Usage> = Mutex::new(Usage {
    samples: VecDeque::new(),
    rss_kib: None,
});

fn usage() -> std::sync::MutexGuard<'static, Usage> {
    USAGE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes one reading now, for the supervisor's tick. A file that cannot be read leaves its
/// figure unknown; the lock is held only to put the reading in.
pub fn sample() {
    let cores = fs::read_to_string("/proc/stat")
        .map(|text| parse_cores(&text))
        .unwrap_or_default();
    let process = fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|text| task_ticks(&text));
    let rss = read_rss_kib();
    let reading = Sample {
        at: Instant::now(),
        cores,
        process,
    };
    usage().note(reading, rss);
}

/// A number rounded to one decimal.
fn tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// The `cpu` and `memory` objects of the statistics, from the last readings and the resolver's
/// `block_set_bytes` and `cache_bytes`. What is not known yet is `null`.
pub fn json(block_set_bytes: usize, cache_bytes: usize) -> (Value, Value) {
    let figures = usage().figures(ticks_per_second());
    let kib = |bytes: usize| (bytes as u64).div_ceil(1024);
    (
        json!({
            "cores": figures.cores.map(|cores| cores.into_iter().map(tenths).collect::<Vec<_>>()),
            "process": figures.process.map(tenths),
            "window_secs": figures.window_secs.map(tenths),
        }),
        json!({
            "rss_kib": figures.rss_kib,
            "block_set_kib": kib(block_set_bytes),
            "cache_kib": kib(cache_bytes),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reading at `at` of two cores that have each counted `total` ticks, the first with
    /// `busy` of them busy and the second none, and of a process that has used `process`.
    fn reading(at: Instant, busy: u64, total: u64, process: u64) -> Sample {
        Sample {
            at,
            cores: vec![CoreTimes { busy, total }, CoreTimes { busy: 0, total }],
            process: Some(process),
        }
    }

    #[test]
    fn one_reading_says_nothing_and_two_say_the_shares_between() {
        let t0 = Instant::now();
        let mut usage = Usage::default();
        usage.note(reading(t0, 0, 0, 0), Some(5_000));
        let one = usage.figures(100);
        assert_eq!((one.cores, one.process), (None, None));
        assert_eq!(one.rss_kib, Some(5_000));
        // 500 of 1,000 ticks busy on the first core, and 250 ticks of the process in 5 s.
        usage.note(
            reading(t0 + Duration::from_secs(5), 500, 1000, 250),
            Some(6_000),
        );
        let two = usage.figures(100);
        assert_eq!(two.cores, Some(vec![50.0, 0.0]));
        assert_eq!(two.process, Some(50.0));
        assert_eq!(two.window_secs, Some(5.0));
        assert_eq!(two.rss_kib, Some(6_000));
    }

    #[test]
    fn the_window_is_the_last_ten_seconds() {
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        let mut usage = Usage::default();
        for (secs, busy) in [(0, 0), (5, 100), (10, 300), (15, 600), (20, 1000)] {
            usage.note(reading(at(secs), busy, secs * 100, 0), None);
        }
        // Kept: the readings at 10, 15 and 20 s; the shares are those of 10 to 20 s.
        assert_eq!(usage.samples.len(), 3);
        let figures = usage.figures(100);
        assert_eq!(figures.window_secs, Some(10.0));
        // 700 busy of the 1,000 ticks from 10 to 20 s.
        assert_eq!(figures.cores, Some(vec![70.0, 0.0]));
    }

    #[test]
    fn the_resident_memory_is_read_from_the_status_file() {
        let text = "Name:\tqberrypi\nVmPeak:\t  9000 kB\nVmRSS:\t  4321 kB\nThreads:\t7\n";
        assert_eq!(rss_kib(text), Some(4321));
        assert_eq!(rss_kib("Name:\tx\n"), None);
    }

    #[test]
    fn this_process_reads_its_own_resident_memory() {
        assert!(read_rss_kib().is_some_and(|kib| kib > 0));
    }

    #[test]
    fn a_sample_of_this_process_reads_every_file() {
        sample();
        sample();
        let (cpu, memory) = json(2048, 1);
        assert!(
            cpu["cores"]
                .as_array()
                .is_some_and(|cores| !cores.is_empty())
        );
        assert!(cpu["process"].is_number());
        assert!(memory["rss_kib"].as_u64().is_some_and(|kib| kib > 0));
        assert_eq!(
            (
                memory["block_set_kib"].as_u64(),
                memory["cache_kib"].as_u64()
            ),
            (Some(2), Some(1))
        );
    }
}
