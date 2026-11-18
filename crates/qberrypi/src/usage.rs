//! How busy the board's cores and this process are, and how much memory the process holds: the
//! `cpu` and `memory` of `/api/v1/stats`.
//!
//! The supervisor's tick, every five seconds, calls [`sample`], which reads `/proc/stat`,
//! `/proc/self/stat` and `/proc/self/status` (three small files of the kernel's, microseconds on
//! the runtime thread) and keeps the readings of the last [`WINDOW`]. Nothing on the answer path
//! touches any of it: the statistics read what the last tick left, under a lock that only the
//! tick and a request of the statistics take, and the block set's and the cache's sizes, which
//! the resolver keeps as it goes, are read under the lock the statistics take for their counts.
//!
//! The front panel's SYSTEM screens read the board itself, with [`Readings`]: the uptime, the load, the
//! memory in use and the heat, from `/proc` and `/sys` on the caller's own thread (the panel's, never the
//! runtime's) every few seconds while one of those screens shows. Each figure is parsed from the text of
//! its file by a function of its own, and one that the board cannot give is `None`.

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use qb_bench::cpu::{CoreTimes, busy_percent, parse_cores, task_ticks, ticks_per_second};
use serde_json::{Value, json};

use crate::thermal::{self, BOARD_TEMP};

/// Where the kernel keeps the files [`Readings`] reads.
pub const PROC: &str = "/proc";

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

/// What the panel reads of the board itself, for UP and the SYSTEM wall: how long it has run, how busy it
/// has been over the last minute, how much of its memory is in use, and how hot it is. A figure that
/// could not be read, or whose file said something that is none, is `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Readings {
    /// The seconds since the kernel started (`/proc/uptime`).
    pub uptime_secs: Option<u64>,
    /// The load over the last minute in hundredths: `21` is `0.21` (`/proc/loadavg`).
    pub load_hundredths: Option<u32>,
    /// The share of the memory in use, in whole per cent: what is not `MemAvailable` of `MemTotal`
    /// (`/proc/meminfo`).
    pub memory_pct: Option<u8>,
    /// The processor's temperature in whole degrees Celsius (`thermal::read_c`).
    pub temp_c: Option<i8>,
}

impl Readings {
    /// The readings of the running board, taken now on the calling thread: four small files of the
    /// kernel's, microseconds each. Call it from the panel's thread, never from the runtime's.
    pub fn read() -> Self {
        Self::read_from(Path::new(PROC), Path::new(BOARD_TEMP))
    }

    /// The readings of the files `uptime`, `loadavg` and `meminfo` of `proc` and the temperature file
    /// `temp`, so that a test can read fixtures.
    pub fn read_from(proc: &Path, temp: &Path) -> Self {
        let text = |name: &str| fs::read_to_string(proc.join(name)).ok();
        Self {
            uptime_secs: text("uptime").as_deref().and_then(parse_uptime),
            load_hundredths: text("loadavg").as_deref().and_then(parse_load),
            memory_pct: text("meminfo").as_deref().and_then(parse_memory_pct),
            temp_c: thermal::read_c(temp),
        }
    }
}

/// The whole seconds of the first number of a `/proc/uptime` text (`12345.67 54321.00`).
pub fn parse_uptime(text: &str) -> Option<u64> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then_some(secs as u64)
}

/// The load of the last minute, in hundredths, from a `/proc/loadavg` text (`0.21 0.15 0.10 1/123 4567`).
pub fn parse_load(text: &str) -> Option<u32> {
    let load: f64 = text.split_whitespace().next()?.parse().ok()?;
    (load.is_finite() && (0.0..1_000_000.0).contains(&load)).then(|| (load * 100.0).round() as u32)
}

/// The share of the memory in use, in whole per cent, from a `/proc/meminfo` text: `MemTotal` less
/// `MemAvailable`, of `MemTotal`. `None` when either line is missing or the total is none.
pub fn parse_memory_pct(text: &str) -> Option<u8> {
    let kib = |key: &str| -> Option<u64> {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let (total, available) = (kib("MemTotal")?, kib("MemAvailable")?);
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(available.min(total));
    // At most 100, so a byte holds it.
    Some(((u128::from(used) * 100 + u128::from(total) / 2) / u128::from(total)) as u8)
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
    fn the_uptime_is_the_whole_seconds_of_the_first_number() {
        assert_eq!(parse_uptime("12345.67 54321.00\n"), Some(12_345));
        assert_eq!(parse_uptime("0.99 0.00\n"), Some(0));
        assert_eq!(parse_uptime("90061 1\n"), Some(90_061));
        for bad in ["", "\n", "up 1\n", "-3.0 1.0\n", "nan 1\n", "inf 1\n"] {
            assert_eq!(parse_uptime(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_load_is_the_minute_figure_in_hundredths() {
        assert_eq!(parse_load("0.21 0.15 0.10 1/123 4567\n"), Some(21));
        assert_eq!(parse_load("12.50 3.00 1.00 2/200 9\n"), Some(1250));
        assert_eq!(parse_load("0.00 0.00 0.00 1/1 1\n"), Some(0));
        // The kernel's two decimals are rounded, not cut, should it ever give a third.
        assert_eq!(parse_load("0.125 0 0 1/1 1"), Some(13));
        for bad in ["", "\n", "busy\n", "-0.5 0 0 1/1 1\n", "NaN 0 0 1/1 1\n"] {
            assert_eq!(parse_load(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_memory_in_use_is_what_is_not_available_of_the_total() {
        let meminfo = "MemTotal:         948304 kB\nMemFree:          100000 kB\nMemAvailable:     587968 kB\nBuffers:            1000 kB\n";
        // 360,336 of 948,304 kB is 38.0 per cent.
        assert_eq!(parse_memory_pct(meminfo), Some(38));
        let all_free = "MemTotal: 1000 kB\nMemAvailable: 1000 kB\n";
        let none_free = "MemTotal: 1000 kB\nMemAvailable: 0 kB\n";
        assert_eq!(parse_memory_pct(all_free), Some(0));
        assert_eq!(parse_memory_pct(none_free), Some(100));
        // More available than there is (a kernel's rounding): none in use.
        assert_eq!(
            parse_memory_pct("MemTotal: 1000 kB\nMemAvailable: 1200 kB\n"),
            Some(0)
        );
        // `MemTotal` is not taken for a line that only begins with it, and a line without both is no figure.
        assert_eq!(
            parse_memory_pct("MemTotalX: 5 kB\nMemAvailable: 1 kB\n"),
            None
        );
        assert_eq!(parse_memory_pct("MemTotal: 1000 kB\n"), None);
        assert_eq!(parse_memory_pct("MemAvailable: 1000 kB\n"), None);
        assert_eq!(
            parse_memory_pct("MemTotal: 0 kB\nMemAvailable: 0 kB\n"),
            None
        );
        assert_eq!(
            parse_memory_pct("MemTotal: lots\nMemAvailable: 1 kB\n"),
            None
        );
        assert_eq!(parse_memory_pct(""), None);
    }

    #[test]
    fn the_readings_come_from_the_files_and_each_figure_alone() {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path();
        let temp = dir.path().join("temp");
        // No file: no figure.
        assert_eq!(Readings::read_from(proc, &temp), Readings::default());
        fs::write(proc.join("uptime"), "90061.50 1.00\n").unwrap();
        fs::write(proc.join("loadavg"), "0.21 0.15 0.10 1/123 4567\n").unwrap();
        fs::write(
            proc.join("meminfo"),
            "MemTotal: 1000 kB\nMemAvailable: 620 kB\n",
        )
        .unwrap();
        fs::write(&temp, "46500\n").unwrap();
        assert_eq!(
            Readings::read_from(proc, &temp),
            Readings {
                uptime_secs: Some(90_061),
                load_hundredths: Some(21),
                memory_pct: Some(38),
                temp_c: Some(47),
            }
        );
        // A file that says nothing leaves its own figure unknown and the others as they were.
        fs::write(proc.join("loadavg"), "garbage\n").unwrap();
        fs::remove_file(&temp).unwrap();
        let readings = Readings::read_from(proc, &temp);
        assert_eq!(
            (
                readings.uptime_secs,
                readings.load_hundredths,
                readings.memory_pct,
                readings.temp_c
            ),
            (Some(90_061), None, Some(38), None)
        );
    }

    #[test]
    fn the_running_board_gives_its_uptime_load_and_memory() {
        let readings = Readings::read();
        assert!(readings.uptime_secs.is_some_and(|secs| secs > 0));
        assert!(readings.load_hundredths.is_some());
        assert!(readings.memory_pct.is_some_and(|pct| pct <= 100));
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
