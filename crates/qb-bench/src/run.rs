//! A run: the checks before a load, then a ramp that finds the most a server sustains, or a
//! steady load at one rate, step by step, each with its latency, losses and processor time.
//!
//! A step **passes** when it lost under [`Bounds::loss`] of its questions (a SERVFAIL or REFUSED
//! answer is lost too), its p99 is under [`Bounds::p99`], and the generator sent at least
//! [`Bounds::min_rate`] of the rate asked: a step the generator could not keep up with says
//! nothing of the server. The **most sustained** is the answered rate of the best step that
//! passed.
//!
//! The ramp doubles the rate from its start until a step fails or the cap is reached, then halves
//! the gap between the best pass and the lowest failure until the two are within 5 per cent, as
//! long as the time allows: a step is begun only if it ends within the run's seconds. A step that
//! is still owed answers when it ends waits for them, up to the wait, so a run takes at most its
//! seconds and one wait.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::cpu::{self, CpuUse, Reading, Server};
use crate::engine::{Engine, StepCounts};
use crate::figures::{Latency, latency, ms_text, ms_value};
use crate::names::{BLOCKED_CANDIDATES, CACHED_NAMES, Class, Names, forwarded_patterns};
use crate::wire;

/// How close the ramp brings the best pass and the lowest failure: within this share of the pass.
const RAMP_RESOLUTION: f64 = 0.05;
/// How many names of a run its report lists; the rest are counted.
const NAMES_SHOWN: usize = 8;

/// What makes a step pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// The share of questions that may be lost, exclusive: 0.001 is under 0.1 per cent.
    pub loss: f64,
    /// The p99 must be under this.
    pub p99: Duration,
    /// The share of the rate the generator must have sent.
    pub min_rate: f64,
}

impl Bounds {
    /// Loss under 0.1 per cent, 95 per cent of the rate, and a p99 under `p99`.
    pub fn with_p99(p99: Duration) -> Bounds {
        Bounds {
            loss: 0.001,
            p99,
            min_rate: 0.95,
        }
    }

    /// The default p99 bound of `class`: 10 ms for an answer the server has (on the board's own
    /// loopback a healthy one is well under 1 ms), one second for one it must ask its upstream for.
    pub fn p99_of(class: Class) -> Duration {
        match class {
            Class::Cached | Class::Blocked => Duration::from_millis(10),
            Class::Forwarded => Duration::from_secs(1),
        }
    }
}

/// How the rate is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// From `start` up, at most `max`.
    Ramp { start: u32, max: u32 },
    /// One step at `qps`, for the whole run.
    Steady { qps: u32 },
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Ramp { .. } => "ramp",
            Mode::Steady { .. } => "steady",
        }
    }

    /// The ramp of `class` by default: from 1,000 to 200,000 a second for the answers a server
    /// has; for the forwarded class, which every question sends to the server's upstream (Quad9,
    /// for qberrypi), from 25 to [`FORWARDED_DEFAULT_MAX`] a second.
    pub fn ramp_of(class: Class) -> Mode {
        match class {
            Class::Cached | Class::Blocked => Mode::Ramp {
                start: 1_000,
                max: 200_000,
            },
            Class::Forwarded => Mode::Ramp {
                start: 25,
                max: FORWARDED_DEFAULT_MAX,
            },
        }
    }
}

/// The most forwarded questions a second that a run sends unless told otherwise: each of them is
/// a fresh name that really goes to someone else's server (Quad9, for qberrypi), which throttles a
/// source that sends too many (on 2026-11-12 a ramp to 200, and one to 100 then 80 a second, left
/// the household's address unanswered for 30 to 90 s). qberrypi itself holds all its devices to
/// 50 fresh names a second.
pub const FORWARDED_DEFAULT_MAX: u32 = 50;

/// What a run does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub class: Class,
    pub mode: Mode,
    /// How long the run may take, its last wait aside.
    pub seconds: Duration,
    /// How long each step of a ramp is.
    pub step: Duration,
    pub bounds: Bounds,
    /// Whose processor time is the server's.
    pub server: Server,
}

impl Settings {
    /// `class` in `mode` for `seconds`, with steps of one second and the class's p99 bound.
    pub fn new(class: Class, mode: Mode, seconds: Duration, server: Server) -> Settings {
        Settings {
            class,
            mode,
            seconds,
            step: Duration::from_secs(1),
            bounds: Bounds::with_p99(Bounds::p99_of(class)),
            server,
        }
    }
}

/// Why a step did not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// Too many lost.
    Loss,
    /// The p99 was over the bound.
    Slow,
    /// The generator sent too little of the rate.
    Behind,
}

impl Miss {
    pub fn word(self) -> &'static str {
        match self {
            Miss::Loss => "loss",
            Miss::Slow => "p99",
            Miss::Behind => "rate",
        }
    }
}

/// One step, as measured.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    pub target_qps: u32,
    pub length: Duration,
    pub counts: StepCounts,
    pub latency: Latency,
    pub cpu: CpuUse,
    /// `None` when it passed.
    pub miss: Option<Miss>,
}

impl StepReport {
    fn secs(&self) -> f64 {
        self.length.as_secs_f64().max(f64::MIN_POSITIVE)
    }

    /// The questions sent a second.
    pub fn achieved_qps(&self) -> f64 {
        self.counts.sent as f64 / self.secs()
    }

    /// The questions answered a second, SERVFAIL and REFUSED left out.
    pub fn answered_qps(&self) -> f64 {
        self.counts.answered as f64 / self.secs()
    }

    /// The share of the sent that were lost or failed.
    pub fn loss(&self) -> f64 {
        if self.counts.sent == 0 {
            0.0
        } else {
            (self.counts.lost + self.counts.failed) as f64 / self.counts.sent as f64
        }
    }

    pub fn passed(&self) -> bool {
        self.miss.is_none()
    }
}

/// What a step's figures say against `bounds`: the first of loss, p99 and rate that misses.
pub fn judge(
    target_qps: u32,
    length: Duration,
    counts: &StepCounts,
    latency: &Latency,
    bounds: &Bounds,
) -> Option<Miss> {
    let expected = f64::from(target_qps) * length.as_secs_f64();
    let lost = (counts.lost + counts.failed) as f64;
    if counts.sent == 0 || lost >= bounds.loss * counts.sent as f64 {
        return Some(Miss::Loss);
    }
    let p99 = latency.p99_us.map_or(u128::MAX, u128::from);
    if p99 >= bounds.p99.as_micros() {
        return Some(Miss::Slow);
    }
    if (counts.sent as f64) < bounds.min_rate * expected {
        return Some(Miss::Behind);
    }
    None
}

/// What a run measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub settings: Settings,
    /// The engine's wait and in-flight cap, and how it sends.
    pub timeout: Duration,
    pub max_in_flight: usize,
    pub transport: &'static str,
    /// The names it asked, or their patterns; at most [`NAMES_SHOWN`] of `names_count`.
    pub names: Vec<String>,
    pub names_count: usize,
    pub steps: Vec<StepReport>,
    /// Whether the ramp passed at its cap: the server sustains at least that, perhaps more.
    pub at_cap: bool,
    pub cpu: CpuUse,
    pub took: Duration,
}

impl Report {
    /// The step that passed with the highest answered rate.
    pub fn best(&self) -> Option<&StepReport> {
        self.steps
            .iter()
            .filter(|step| step.passed())
            .max_by(|a, b| a.answered_qps().total_cmp(&b.answered_qps()))
    }

    /// The most answers a second that a step sustained within the bounds.
    pub fn max_sustained_qps(&self) -> Option<f64> {
        self.best().map(StepReport::answered_qps)
    }
}

/// The names a load of `class` asks, checked against the server through `engine`: `given`, or the
/// class's own. A cached name must be answered (it is then in the server's cache), and those that
/// are not are left out; a blocked name must answer as blocked (a pattern is checked with one
/// fresh label), every given one, or the first of the candidates. Forwarded names are not
/// checked. The error says what each answered.
pub fn prepare(
    engine: &mut Engine,
    class: Class,
    given: &[String],
) -> Result<(Names, Vec<String>), String> {
    let wait = engine.limits().timeout;
    let describe = |engine: &mut Engine, name: &str| -> Result<wire::Answer, String> {
        match engine.ask(name, wait) {
            Ok(Some(reply)) => {
                wire::answer(&reply).ok_or_else(|| "an answer that is no DNS message".to_owned())
            }
            Ok(None) => Err(format!("no answer within {} ms", wait.as_millis())),
            Err(error) => Err(error.to_string()),
        }
    };
    match class {
        Class::Cached => {
            let list: Vec<String> = if given.is_empty() {
                CACHED_NAMES.iter().map(|&n| n.to_owned()).collect()
            } else {
                given.to_vec()
            };
            let mut kept = Vec::new();
            let mut seen = Vec::new();
            for name in &list {
                match describe(engine, name) {
                    Ok(answer) if !wire::is_failure(answer.rcode) => kept.push(name.clone()),
                    Ok(answer) => seen.push(format!("{name}: {}", answer.describe())),
                    Err(why) => seen.push(format!("{name}: {why}")),
                }
            }
            if kept.is_empty() {
                return Err(format!("no cached name was answered ({})", seen.join("; ")));
            }
            Ok((Names::new(&kept)?, kept))
        }
        Class::Blocked => {
            let (list, all): (Vec<String>, bool) = if given.is_empty() {
                (
                    BLOCKED_CANDIDATES.iter().map(|&n| n.to_owned()).collect(),
                    false,
                )
            } else {
                (given.to_vec(), true)
            };
            let mut kept = Vec::new();
            let mut seen = Vec::new();
            for name in &list {
                let asked = name.replace('*', "qb-bench-check");
                match describe(engine, &asked) {
                    Ok(answer) if answer.is_blocked() => {
                        kept.push(name.clone());
                        if !all {
                            break;
                        }
                    }
                    Ok(answer) => seen.push(format!("{name}: {}", answer.describe())),
                    Err(why) => seen.push(format!("{name}: {why}")),
                }
            }
            if kept.is_empty() || (all && !seen.is_empty()) {
                let whose = if all { "given" } else { "built-in" };
                return Err(format!(
                    "not every {whose} name answered as blocked ({}); name ones the server \
                     blocks with --blocked",
                    seen.join("; ")
                ));
            }
            Ok((Names::new(&kept)?, kept))
        }
        Class::Forwarded => {
            let list = if given.is_empty() {
                forwarded_patterns()
            } else {
                given.to_vec()
            };
            Ok((Names::new(&list)?, list))
        }
    }
}

/// One step at `qps` for `length`, with its figures and processor time.
fn step(
    engine: &mut Engine,
    names: &mut Names,
    settings: &Settings,
    qps: u32,
    length: Duration,
    ticks: u64,
) -> StepReport {
    let before = Reading::take(settings.server);
    let mut counts = engine.step(qps, length, names);
    let after = Reading::take(settings.server);
    let wait_us = u32::try_from(engine.limits().timeout.as_micros()).unwrap_or(u32::MAX);
    let unanswered = usize::try_from(counts.lost + counts.failed).unwrap_or(usize::MAX);
    let figures = latency(&mut counts.latencies_us, unanswered, wait_us);
    let miss = judge(qps, length, &counts, &figures, &settings.bounds);
    StepReport {
        target_qps: qps,
        length,
        counts,
        latency: figures,
        cpu: cpu::between(&before, &after, settings.server, ticks),
        miss,
    }
}

/// The next rate of a ramp after its steps so far, `None` when it is done: doubling up to `max`
/// until a failure, halving down below a first step that failed, then halving the gap between
/// the best pass and the lowest failure until it is within [`RAMP_RESOLUTION`].
pub fn next_rate(
    best_pass: Option<u32>,
    lowest_fail: Option<u32>,
    last: u32,
    max: u32,
) -> Option<u32> {
    match (best_pass, lowest_fail) {
        (_, None) if last >= max => None,
        (_, None) => Some(last.saturating_mul(2).min(max)),
        (None, Some(fail)) => Some(fail / 2).filter(|&rate| rate >= 1),
        (Some(pass), Some(fail)) => {
            let gap = f64::from(fail.saturating_sub(pass));
            (gap > f64::from(pass) * RAMP_RESOLUTION && fail > pass + 1)
                .then(|| pass + (fail - pass) / 2)
        }
    }
}

/// Runs the load of `settings` with `names` through `engine`, telling `progress` of each step as
/// it ends.
pub fn run(
    engine: &mut Engine,
    names: &mut Names,
    settings: &Settings,
    shown: (Vec<String>, usize),
    mut progress: impl FnMut(&StepReport),
) -> Report {
    let ticks = cpu::ticks_per_second();
    let started = Instant::now();
    let first = Reading::take(settings.server);
    let mut steps = Vec::new();
    let mut at_cap = false;
    match settings.mode {
        Mode::Steady { qps } => {
            let report = step(engine, names, settings, qps, settings.seconds, ticks);
            progress(&report);
            steps.push(report);
        }
        Mode::Ramp { start, max } => {
            let (mut pass, mut fail) = (None::<u32>, None::<u32>);
            let mut rate = start.clamp(1, max.max(1));
            loop {
                if started.elapsed() + settings.step > settings.seconds {
                    break;
                }
                let report = step(engine, names, settings, rate, settings.step, ticks);
                progress(&report);
                if report.passed() {
                    pass = pass.max(Some(rate));
                } else {
                    fail = Some(fail.map_or(rate, |f: u32| f.min(rate)));
                }
                steps.push(report);
                if fail.is_none() && rate >= max {
                    at_cap = true;
                }
                match next_rate(pass, fail, rate, max) {
                    Some(next) => rate = next,
                    None => break,
                }
            }
        }
    }
    let last = Reading::take(settings.server);
    let limits = engine.limits();
    Report {
        settings: *settings,
        timeout: limits.timeout,
        max_in_flight: limits.max_in_flight,
        transport: engine.transport().word(),
        names: shown.0,
        names_count: shown.1,
        steps,
        at_cap,
        cpu: cpu::between(&first, &last, settings.server, ticks),
        took: started.elapsed(),
    }
}

/// The names a report lists: at most [`NAMES_SHOWN`] of `names`, and how many there are.
pub fn shown(names: &[String]) -> (Vec<String>, usize) {
    (
        names.iter().take(NAMES_SHOWN).cloned().collect(),
        names.len(),
    )
}

/// A number rounded to `places` decimals.
fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Processor time as JSON: the cores' busy shares and the two per cent figures, rounded to one
/// decimal; `null` for what is not known.
pub fn cpu_json(cpu: &CpuUse) -> Value {
    json!({
        "cores_busy_pct": cpu.cores_busy_pct.as_ref().map(|cores| {
            cores.iter().map(|&pct| round(pct, 1)).collect::<Vec<_>>()
        }),
        "generator_pct": cpu.generator_pct.map(|pct| round(pct, 1)),
        "server_pct": cpu.server_pct.map(|pct| round(pct, 1)),
    })
}

/// One step as JSON.
pub fn step_json(step: &StepReport) -> Value {
    let mut object = json!({
        "target_qps": step.target_qps,
        "seconds": round(step.length.as_secs_f64(), 3),
        "sent": step.counts.sent,
        "answered": step.counts.answered,
        "failed": step.counts.failed,
        "lost": step.counts.lost,
        "late": step.counts.late,
        "socket_errors": step.counts.socket_errors,
        "achieved_qps": round(step.achieved_qps(), 1),
        "answered_qps": round(step.answered_qps(), 1),
        "loss_pct": round(step.loss() * 100.0, 3),
        "p50_ms": ms_value(step.latency.p50_us),
        "p95_ms": ms_value(step.latency.p95_us),
        "p99_ms": ms_value(step.latency.p99_us),
        "max_ms": ms_value(step.latency.max_us),
        "cpu": cpu_json(&step.cpu),
        "pass": step.passed(),
        "miss": step.miss.map(Miss::word),
    });
    if let Some(error) = &step.counts.first_error {
        object["first_socket_error"] = json!(error);
    }
    object
}

/// A run as JSON. It holds no address of the server.
pub fn report_json(report: &Report) -> Value {
    let settings = &report.settings;
    let mut object = json!({
        "class": settings.class.word(),
        "mode": settings.mode.word(),
        "transport": report.transport,
        "names": report.names,
        "names_count": report.names_count,
        "timeout_ms": report.timeout.as_millis() as u64,
        "max_in_flight": report.max_in_flight,
        "step_ms": settings.step.as_millis() as u64,
        "seconds": settings.seconds.as_secs_f64(),
        "bounds": {
            "loss_pct": round(settings.bounds.loss * 100.0, 3),
            "p99_ms": settings.bounds.p99.as_secs_f64() * 1000.0,
            "min_rate_pct": round(settings.bounds.min_rate * 100.0, 1),
        },
        "max_sustained_qps": report.max_sustained_qps().map(|qps| round(qps, 1)),
        "at_cap": report.at_cap,
        "cpu": cpu_json(&report.cpu),
        "took_secs": round(report.took.as_secs_f64(), 3),
        "steps": report.steps.iter().map(step_json).collect::<Vec<_>>(),
    });
    match settings.mode {
        Mode::Ramp { start, max } => {
            object["ramp"] = json!({ "start_qps": start, "max_qps": max });
        }
        Mode::Steady { qps } => object["qps"] = json!(qps),
    }
    object
}

/// A percentage with one decimal, or `-`.
fn pct_text(pct: Option<f64>) -> String {
    pct.map_or_else(|| "-".to_owned(), |pct| format!("{pct:.1}"))
}

/// The header of the text table of steps.
pub fn step_header() -> String {
    format!(
        "{:>9} {:>10} {:>10} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6}  {}",
        "target",
        "sent/s",
        "answered/s",
        "lost",
        "failed",
        "loss %",
        "p50 ms",
        "p95 ms",
        "p99 ms",
        "gen %",
        "srv %",
        "core %",
        "verdict"
    )
}

/// One step as a line of the text table: the busiest core is the core column.
pub fn step_line(step: &StepReport) -> String {
    let busiest = step
        .cpu
        .cores_busy_pct
        .as_ref()
        .and_then(|cores| cores.iter().copied().reduce(f64::max));
    format!(
        "{:>9} {:>10.1} {:>10.1} {:>7} {:>7} {:>8.3} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6}  {}",
        step.target_qps,
        step.achieved_qps(),
        step.answered_qps(),
        step.counts.lost,
        step.counts.failed,
        step.loss() * 100.0,
        ms_text(step.latency.p50_us),
        ms_text(step.latency.p95_us),
        ms_text(step.latency.p99_us),
        pct_text(step.cpu.generator_pct),
        pct_text(step.cpu.server_pct),
        pct_text(busiest),
        step.miss.map_or("pass", Miss::word),
    )
}

/// A run as plain text: what it was, the table of its steps and the most it sustained.
pub fn render_text(report: &Report) -> String {
    let settings = &report.settings;
    let mut text = format!(
        "qb-bench: {} {} over {}, {} names ({}{}); wait {} ms, at most {} in flight\n\
         qb-bench: a step passes with loss under {}%, p99 under {} ms and {}% of the rate sent\n",
        settings.class.word(),
        settings.mode.word(),
        report.transport,
        report.names_count,
        report.names.join(", "),
        if report.names_count > report.names.len() {
            ", ..."
        } else {
            ""
        },
        report.timeout.as_millis(),
        report.max_in_flight,
        round(settings.bounds.loss * 100.0, 3),
        settings.bounds.p99.as_secs_f64() * 1000.0,
        round(settings.bounds.min_rate * 100.0, 1),
    );
    text.push_str(&step_header());
    text.push('\n');
    for step in &report.steps {
        text.push_str(&step_line(step));
        text.push('\n');
    }
    match report.max_sustained_qps() {
        Some(qps) if report.at_cap => text.push_str(&format!(
            "qb-bench: {}: sustained {qps:.1} answers/s, at the ramp's cap: the server may do more\n",
            settings.class.word()
        )),
        Some(qps) => text.push_str(&format!(
            "qb-bench: {}: most sustained {qps:.1} answers/s\n",
            settings.class.word()
        )),
        None => text.push_str(&format!(
            "qb-bench: {}: no step passed\n",
            settings.class.word()
        )),
    }
    for step in &report.steps {
        if let Some(error) = &step.counts.first_error {
            text.push_str(&format!(
                "qb-bench: step at {} q/s: {} socket errors, the first: {error}\n",
                step.target_qps, step.counts.socket_errors
            ));
            break;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Limits, Transport};
    use crate::testserver::{Behaviour, Reply, TestServer};
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    fn counts(sent: u64, answered: u64, lost: u64, failed: u64) -> StepCounts {
        StepCounts {
            sent,
            answered,
            lost,
            failed,
            ..StepCounts::default()
        }
    }

    fn p99(us: u32) -> Latency {
        Latency {
            p99_us: Some(us),
            ..Latency::default()
        }
    }

    #[test]
    fn a_step_passes_only_within_every_bound() {
        let bounds = Bounds::with_p99(Duration::from_millis(10));
        let second = Duration::from_secs(1);
        let judged = |c: StepCounts, l: Latency| judge(1000, second, &c, &l, &bounds);
        assert_eq!(judged(counts(1000, 1000, 0, 0), p99(500)), None);
        // One lost of 1,000 is 0.1 per cent, which is not under it.
        assert_eq!(judged(counts(1000, 999, 1, 0), p99(500)), Some(Miss::Loss));
        assert_eq!(judged(counts(1000, 999, 0, 1), p99(500)), Some(Miss::Loss));
        assert_eq!(judged(counts(2000, 1999, 1, 0), p99(500)), None);
        assert_eq!(
            judged(counts(1000, 1000, 0, 0), p99(10_000)),
            Some(Miss::Slow)
        );
        assert_eq!(judged(counts(940, 940, 0, 0), p99(500)), Some(Miss::Behind));
        assert_eq!(
            judged(counts(0, 0, 0, 0), Latency::default()),
            Some(Miss::Loss)
        );
    }

    #[test]
    fn the_ramp_doubles_then_halves_the_gap() {
        assert_eq!(next_rate(Some(1000), None, 1000, 200_000), Some(2000));
        assert_eq!(
            next_rate(Some(128_000), None, 128_000, 200_000),
            Some(200_000)
        );
        assert_eq!(next_rate(Some(200_000), None, 200_000, 200_000), None);
        assert_eq!(next_rate(Some(4000), Some(8000), 8000, 200_000), Some(6000));
        assert_eq!(next_rate(Some(6000), Some(6200), 6200, 200_000), None);
        assert_eq!(next_rate(None, Some(1000), 1000, 200_000), Some(500));
        assert_eq!(next_rate(None, Some(1), 1, 200_000), None);
    }

    fn engine(server: &TestServer) -> Engine {
        Engine::connect(
            server.addr,
            Transport::Udp,
            Limits {
                timeout: Duration::from_millis(200),
                max_in_flight: 1000,
            },
        )
        .unwrap()
    }

    fn quick(class: Class, mode: Mode, millis: u64) -> Settings {
        Settings {
            step: Duration::from_millis(200),
            bounds: Bounds::with_p99(Duration::from_millis(50)),
            ..Settings::new(
                class,
                mode,
                Duration::from_millis(millis),
                Server::ThisProcess,
            )
        }
    }

    #[test]
    fn a_ramp_against_a_server_that_keeps_up_ends_passing_at_its_cap() {
        // Answers every question; the ramp goes from 100 to its cap of 800 and passes there.
        let server = TestServer::start(Behaviour::Answer);
        let mut engine = engine(&server);
        let (mut names, _) = prepare(&mut engine, Class::Cached, &[]).unwrap();
        let settings = quick(
            Class::Cached,
            Mode::Ramp {
                start: 100,
                max: 800,
            },
            2_000,
        );
        let mut seen = 0;
        let report = run(&mut engine, &mut names, &settings, (vec![], 0), |_| {
            seen += 1
        });
        let rates: Vec<u32> = report.steps.iter().map(|s| s.target_qps).collect();
        assert_eq!(rates, [100, 200, 400, 800]);
        assert_eq!(seen, 4);
        assert!(report.at_cap);
        let best = report.max_sustained_qps().unwrap();
        assert!((760.0..=800.0).contains(&best), "{best}");
        let json = report_json(&report);
        assert_eq!(json["class"], "cached");
        assert_eq!(json["steps"].as_array().unwrap().len(), 4);
        assert!(json["steps"][0]["cpu"]["generator_pct"].is_number());
        assert!(render_text(&report).contains("at the ramp's cap"));
    }

    #[test]
    fn a_server_that_drops_every_other_question_fails_every_step() {
        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let server = TestServer::start(Behaviour::By(Arc::new(move |_: &str| {
            if count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                .is_multiple_of(2)
            {
                Reply::Address(Ipv4Addr::new(192, 0, 2, 1))
            } else {
                Reply::Silent
            }
        })));
        let mut engine = engine(&server);
        let mut names = Names::new(&["a.example".to_owned()]).unwrap();
        let settings = quick(
            Class::Cached,
            Mode::Ramp {
                start: 100,
                max: 800,
            },
            1_500,
        );
        let report = run(&mut engine, &mut names, &settings, (vec![], 0), |_| {});
        // 100 fails, then 50, 25, ... while the time lasts.
        assert_eq!(report.steps[0].target_qps, 100);
        assert_eq!(report.steps[1].target_qps, 50);
        assert!(report.steps.iter().all(|s| s.miss == Some(Miss::Loss)));
        assert_eq!(report.max_sustained_qps(), None);
        assert!(render_text(&report).contains("no step passed"));
    }

    #[test]
    fn a_steady_run_is_one_step_at_its_rate() {
        let server = TestServer::start(Behaviour::Answer);
        let mut engine = engine(&server);
        let mut names = Names::new(&["a.example".to_owned()]).unwrap();
        let settings = quick(Class::Cached, Mode::Steady { qps: 300 }, 500);
        let report = run(&mut engine, &mut names, &settings, (vec![], 0), |_| {});
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].counts.sent, 150);
        assert!(report.steps[0].passed());
        assert_eq!(report_json(&report)["qps"], 300);
    }

    #[test]
    fn the_checks_keep_the_names_that_answer_as_their_class_needs() {
        let blocking = TestServer::start(Behaviour::By(Arc::new(|name: &str| {
            if name.ends_with("doubleclick.net") || name == "ads.example" {
                Reply::Address(Ipv4Addr::UNSPECIFIED)
            } else if name.starts_with("gone") {
                Reply::Servfail
            } else {
                Reply::Address(Ipv4Addr::new(192, 0, 2, 1))
            }
        })));
        let mut engine = engine(&blocking);
        // The first built-in candidate is blocked.
        let (names, list) = prepare(&mut engine, Class::Blocked, &[]).unwrap();
        assert_eq!((names.len(), list), (1, vec!["doubleclick.net".to_owned()]));
        // A pattern under it too; a name that is answered is refused, and says so.
        let given = |list: &[&str]| list.iter().map(|&n| n.to_owned()).collect::<Vec<_>>();
        assert!(
            prepare(
                &mut engine,
                Class::Blocked,
                &given(&["*.doubleclick.net", "ads.example"])
            )
            .is_ok()
        );
        let refused = prepare(
            &mut engine,
            Class::Blocked,
            &given(&["ads.example", "real.example"]),
        )
        .unwrap_err();
        assert!(
            refused.contains("real.example: NOERROR with 1 addresses"),
            "{refused}"
        );
        // A cached name that fails is left out; with none left it is an error.
        let (_, kept) = prepare(
            &mut engine,
            Class::Cached,
            &given(&["a.example", "gone.example"]),
        )
        .unwrap();
        assert_eq!(kept, ["a.example"]);
        assert!(prepare(&mut engine, Class::Cached, &given(&["gone.example"])).is_err());
        // Forwarded names are not asked.
        let before = blocking.questions();
        assert_eq!(
            prepare(&mut engine, Class::Forwarded, &[]).unwrap().1.len(),
            6
        );
        assert_eq!(blocking.questions(), before);
    }
}
