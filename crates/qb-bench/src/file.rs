//! A file of runs: for each class a ramp, and a steady load at 80 per cent of the most the ramp
//! found, as `qb-bench --json`, `cargo xtask bench` and qberrypi's own route write them; the
//! summary table of such a file; and two of them compared, as `cargo xtask latency --compare`
//! compares its files.
//!
//! The file's shape: `{"tool": "qb-bench", "version", "source", "unix_secs", "classes": [{"class",
//! "ramp": <run>, "steady": <run>, "error"?}]}`, a run being [`crate::run::report_json`]'s object.
//! It holds no address.

use serde_json::{Value, json};

use crate::engine::Engine;
use crate::names::Class;
use crate::run::{self, Mode, Settings, prepare, render_text, report_json, step_header, step_line};

/// The share of the most sustained that the steady load runs at.
pub const STEADY_SHARE: f64 = 0.8;

/// The rate of the steady load after a ramp that sustained `max` answers a second: 80 per cent of
/// it, at least 1.
pub fn steady_rate(max: f64) -> u32 {
    let rate = (max * STEADY_SHARE).floor();
    if rate >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        (rate as u32).max(1)
    }
}

/// What was measured of one class: its ramp and its steady load as JSON, either missing when it
/// was not run, and why it stopped, if it did.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassRun {
    pub class: Class,
    pub ramp: Option<Value>,
    pub steady: Option<Value>,
    pub error: Option<String>,
}

/// The file of `runs`, made by `source` (`"qb-bench"`, `"on-pi"`, `"from-here"`) at `unix_secs`.
pub fn file_json(source: &str, unix_secs: u64, runs: &[ClassRun]) -> Value {
    let classes: Vec<Value> = runs
        .iter()
        .map(|run| {
            let mut object = json!({
                "class": run.class.word(),
                "ramp": run.ramp,
                "steady": run.steady,
            });
            if let Some(error) = &run.error {
                object["error"] = json!(error);
            }
            object
        })
        .collect();
    json!({
        "tool": "qb-bench",
        "version": env!("CARGO_PKG_VERSION"),
        "source": source,
        "unix_secs": unix_secs,
        "classes": classes,
    })
}

/// The rate of a steady load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteadyRate {
    Fixed(u32),
    /// [`steady_rate`] of what the ramp before it sustained; none if no step of it passed.
    FromRamp,
}

/// Measures `class` through `engine`: the checks of its names (`given`, or the class's own),
/// then the ramp of `ramp`, then the steady load of `steady` at its rate, each if given. `say` is
/// told each line of progress as it comes: the table's header, each step, and the closing lines.
/// A check that fails, or a steady load that has no rate because no step of the ramp passed,
/// ends the class with its reason in `error`.
pub fn measure(
    engine: &mut Engine,
    class: Class,
    given: &[String],
    ramp: Option<Settings>,
    steady: Option<(Settings, SteadyRate)>,
    say: &mut dyn FnMut(&str),
) -> ClassRun {
    let mut done = ClassRun {
        class,
        ramp: None,
        steady: None,
        error: None,
    };
    let (mut names, list) = match prepare(engine, class, given) {
        Ok(prepared) => prepared,
        Err(why) => {
            done.error = Some(why);
            return done;
        }
    };
    let mut sustained = None;
    if let Some(settings) = ramp {
        say(&step_header());
        let report = run::run(engine, &mut names, &settings, run::shown(&list), |step| {
            say(&step_line(step));
        });
        for line in render_text(&report)
            .lines()
            .filter(|l| l.starts_with("qb-bench"))
        {
            say(line);
        }
        sustained = report.max_sustained_qps();
        done.ramp = Some(report_json(&report));
    }
    if let Some((settings, rate)) = steady {
        let qps = match rate {
            SteadyRate::Fixed(qps) => qps,
            SteadyRate::FromRamp => match sustained {
                Some(max) => steady_rate(max),
                None => {
                    done.error = Some(
                        "no step of the ramp passed, so there is no rate for the steady load"
                            .to_owned(),
                    );
                    return done;
                }
            },
        };
        let settings = Settings {
            mode: Mode::Steady { qps },
            ..settings
        };
        say(&step_header());
        let report = run::run(engine, &mut names, &settings, run::shown(&list), |step| {
            say(&step_line(step));
        });
        done.steady = Some(report_json(&report));
    }
    done
}

fn classes(file: &Value) -> &[Value] {
    file.get("classes")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn class_name(entry: &Value) -> Option<&str> {
    entry.get("class")?.as_str()
}

/// The one step of a class's steady load.
fn steady_step(entry: &Value) -> Option<&Value> {
    entry.get("steady")?.get("steps")?.as_array()?.first()
}

fn number(value: Option<&Value>) -> Option<f64> {
    value?.as_f64()
}

/// A figure as text with `places` decimals, or `-`.
fn cell(value: Option<f64>, places: usize) -> String {
    value.map_or_else(|| "-".to_owned(), |v| format!("{v:.places$}"))
}

/// The busiest core of a step's processor time.
fn busiest(step: &Value) -> Option<f64> {
    step.get("cpu")?
        .get("cores_busy_pct")?
        .as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .reduce(f64::max)
}

/// The table of a file: per class, the most sustained of the ramp (`>=` where it passed at the
/// ramp's cap), and the steady load's rate, latency, loss and processor time.
pub fn summary_text(file: &Value) -> String {
    let mut text = format!(
        "{:<10} {:>12} {:>9} {:>11} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6}\n",
        "class",
        "max q/s",
        "steady",
        "answered/s",
        "loss %",
        "p50 ms",
        "p95 ms",
        "p99 ms",
        "srv %",
        "gen %",
        "core %"
    );
    for entry in classes(file) {
        let name = class_name(entry).unwrap_or("?");
        let ramp = entry.get("ramp");
        let max = number(ramp.and_then(|r| r.get("max_sustained_qps")));
        let at_cap = ramp
            .and_then(|r| r.get("at_cap"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_cell = match max {
            Some(max) if at_cap => format!(">={max:.0}"),
            other => cell(other, 0),
        };
        let step = steady_step(entry);
        let field = |key: &str| number(step.and_then(|s| s.get(key)));
        let cpu = |key: &str| number(step.and_then(|s| s.get("cpu")).and_then(|c| c.get(key)));
        text.push_str(&format!(
            "{:<10} {:>12} {:>9} {:>11} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6}\n",
            name,
            max_cell,
            cell(field("target_qps"), 0),
            cell(field("answered_qps"), 1),
            cell(field("loss_pct"), 3),
            cell(field("p50_ms"), 3),
            cell(field("p95_ms"), 3),
            cell(field("p99_ms"), 3),
            cell(cpu("server_pct"), 1),
            cell(cpu("generator_pct"), 1),
            cell(step.and_then(busiest), 1),
        ));
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            text.push_str(&format!("{name:<10} stopped: {error}\n"));
        }
    }
    text
}

/// How two files compare, as [`compare`] finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// What to print: the files, the table, the notes and a closing line.
    pub text: String,
    /// How many figures were compared.
    pub compared: usize,
    /// How many of them are worse in B than in A.
    pub worse: usize,
    /// How many figures A has that B does not.
    pub missing: usize,
}

impl Comparison {
    /// Why the comparison fails, in words, or `None` when B is no worse on any figure A has.
    pub fn failure(&self) -> Option<String> {
        let mut reasons = Vec::new();
        if self.worse > 0 {
            reasons.push(format!(
                "{} of {} compared figures are worse in B",
                self.worse, self.compared
            ));
        }
        if self.missing > 0 {
            reasons.push(format!("{} figures of A are not known in B", self.missing));
        }
        if self.compared == 0 {
            reasons.push("no figure could be compared".to_owned());
        }
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}

/// The figures compared for each class: the label, where the figure is, and whether more is
/// better.
const FIGURES: [(&str, &str, bool); 4] = [
    ("max q/s", "max_sustained_qps", true),
    ("steady p50", "p50_ms", false),
    ("steady p95", "p95_ms", false),
    ("steady p99", "p99_ms", false),
];

/// The figure `key` of a class: the ramp's most sustained, or the steady step's latency.
fn figure(entry: &Value, key: &str) -> Option<f64> {
    if key == "max_sustained_qps" {
        number(entry.get("ramp").and_then(|ramp| ramp.get(key)))
    } else {
        number(steady_step(entry).and_then(|step| step.get(key)))
    }
}

/// B against A, class by class in A's order: the most sustained (more is better) and the steady
/// load's p50, p95 and p99 (less is better), each `ok` or `worse`. A note says where the steady
/// loads ran at different rates (their latencies are then at different loads: give both runs the
/// same `--steady-qps` to compare them at one), and where the two ran over different transports
/// or from different places. `a_name` and `b_name` are what the text calls the files.
pub fn compare(a_name: &str, a: &Value, b_name: &str, b: &Value) -> Comparison {
    let mut text = String::new();
    for (label, name, file) in [("A", a_name, a), ("B", b_name, b)] {
        let source = file.get("source").and_then(Value::as_str).unwrap_or("?");
        let when = file.get("unix_secs").and_then(Value::as_u64).unwrap_or(0);
        text.push_str(&format!(
            "bench: {label} = {name}: from {source}, unix {when}, {} classes\n",
            classes(file).len()
        ));
    }
    let mut notes = Vec::new();
    let (mut compared, mut worse, mut missing) = (0, 0, 0);
    text.push_str(&format!(
        "{:<10}{:<12}{:>12}{:>12}  verdict\n",
        "class", "figure", "A", "B"
    ));
    for entry_a in classes(a) {
        let Some(name) = class_name(entry_a) else {
            continue;
        };
        let entry_b = classes(b)
            .iter()
            .find(|entry| class_name(entry) == Some(name));
        for (label, key, more_is_better) in FIGURES {
            let x = figure(entry_a, key);
            let y = entry_b.and_then(|entry| figure(entry, key));
            let verdict = match (x, y) {
                (Some(x), Some(y)) => {
                    compared += 1;
                    let better_or_same = if more_is_better { y >= x } else { y <= x };
                    if better_or_same {
                        "ok"
                    } else {
                        worse += 1;
                        "worse"
                    }
                }
                (Some(_), None) => {
                    missing += 1;
                    "missing"
                }
                (None, _) => "-",
            };
            let places = if more_is_better { 1 } else { 3 };
            text.push_str(&format!(
                "{name:<10}{label:<12}{:>12}{:>12}  {verdict}\n",
                cell(x, places),
                cell(y, places)
            ));
        }
        let Some(entry_b) = entry_b else {
            notes.push(format!("bench: {name}: not in {b_name}\n"));
            continue;
        };
        let rate = |entry: &Value| number(steady_step(entry).and_then(|s| s.get("target_qps")));
        if let (Some(x), Some(y)) = (rate(entry_a), rate(entry_b))
            && x != y
        {
            notes.push(format!(
                "bench: {name}: the steady loads ran at {x:.0} and {y:.0} q/s: their latencies \
                 are at different loads\n"
            ));
        }
        let transport = |entry: &Value| {
            entry
                .get("ramp")
                .or_else(|| entry.get("steady"))
                .and_then(|run| run.get("transport"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if transport(entry_a) != transport(entry_b) {
            notes.push(format!(
                "bench: {name}: the two ran over different transports\n"
            ));
        }
    }
    if a.get("source") != b.get("source") {
        notes.push(
            "bench: the files come from different places (on the board, or over the network): \
             their figures are not like for like\n"
                .to_owned(),
        );
    }
    for note in notes {
        text.push_str(&note);
    }
    let mut comparison = Comparison {
        text,
        compared,
        worse,
        missing,
    };
    let closing = match comparison.failure() {
        Some(words) => format!("bench: {words}\n"),
        None => format!("bench: all {compared} compared figures are ok\n"),
    };
    comparison.text.push_str(&closing);
    comparison
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with one class whose ramp sustained `max` and whose steady load at `rate` had `p50`,
    /// `p95` and `p99`.
    fn file(source: &str, max: f64, rate: u32, p: [f64; 3]) -> Value {
        let steady = json!({
            "transport": "udp",
            "steps": [{
                "target_qps": rate, "answered_qps": f64::from(rate), "loss_pct": 0.0,
                "p50_ms": p[0], "p95_ms": p[1], "p99_ms": p[2],
                "cpu": {"cores_busy_pct": [10.0, 95.5, 3.0, 2.0], "generator_pct": 40.0, "server_pct": 90.0},
            }],
        });
        let ramp = json!({"transport": "udp", "max_sustained_qps": max, "at_cap": false});
        file_json(
            source,
            1_800_000_000,
            &[ClassRun {
                class: Class::Cached,
                ramp: Some(ramp),
                steady: Some(steady),
                error: None,
            }],
        )
    }

    #[test]
    fn a_class_is_its_ramp_then_a_steady_load_at_eighty_per_cent() {
        use crate::cpu::Server;
        use crate::engine::{Limits, Transport};
        use crate::run::Bounds;
        use crate::testserver::{Behaviour, TestServer};
        use std::time::Duration;

        let server = TestServer::start(Behaviour::Blocked);
        let mut engine = Engine::connect(
            server.addr,
            Transport::Udp,
            Limits {
                timeout: Duration::from_millis(200),
                max_in_flight: 1000,
            },
        )
        .unwrap();
        let quick = |mode| Settings {
            step: Duration::from_millis(200),
            bounds: Bounds::with_p99(Duration::from_millis(50)),
            ..Settings::new(
                Class::Blocked,
                mode,
                Duration::from_millis(900),
                Server::ThisProcess,
            )
        };
        let mut lines = Vec::new();
        let run = measure(
            &mut engine,
            Class::Blocked,
            &[],
            Some(quick(Mode::Ramp {
                start: 100,
                max: 400,
            })),
            Some((quick(Mode::Steady { qps: 0 }), SteadyRate::FromRamp)),
            &mut |line| lines.push(line.to_owned()),
        );
        assert_eq!(run.error, None);
        let max = run.ramp.as_ref().unwrap()["max_sustained_qps"]
            .as_f64()
            .unwrap();
        let steady = &run.steady.as_ref().unwrap()["steps"][0];
        assert_eq!(
            steady["target_qps"].as_u64(),
            Some(u64::from(steady_rate(max)))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("most sustained") || line.contains("cap"))
        );
        assert_eq!(run.ramp.as_ref().unwrap()["names"][0], "doubleclick.net");

        // A server that blocks nothing ends the class at its check.
        let open = TestServer::start(Behaviour::Answer);
        let mut engine = Engine::connect(
            open.addr,
            Transport::Udp,
            Limits {
                timeout: Duration::from_millis(200),
                max_in_flight: 1000,
            },
        )
        .unwrap();
        let run = measure(&mut engine, Class::Blocked, &[], None, None, &mut |_| {});
        assert!(run.error.unwrap().contains("answered as blocked"));
    }

    #[test]
    fn the_steady_rate_is_eighty_per_cent_of_the_most() {
        assert_eq!(steady_rate(10_000.0), 8_000);
        assert_eq!(steady_rate(0.5), 1);
        assert_eq!(steady_rate(f64::MAX), u32::MAX);
    }

    #[test]
    fn the_summary_has_a_line_per_class_with_the_busiest_core() {
        let text = summary_text(&file("on-pi", 12_345.6, 9_876, [0.1, 0.2, 0.3]));
        let line = text.lines().nth(1).unwrap();
        for part in [
            "cached", "12346", "9876", "0.100", "0.300", "90.0", "40.0", "95.5",
        ] {
            assert!(line.contains(part), "{part} in {line}");
        }
    }

    #[test]
    fn b_is_ok_where_it_does_as_much_or_more_and_is_as_fast_or_faster() {
        let a = file("on-pi", 10_000.0, 8_000, [0.2, 0.4, 0.8]);
        let same = compare("a", &a, "b", &a);
        assert_eq!((same.compared, same.worse, same.missing), (4, 0, 0));
        assert_eq!(same.failure(), None);
        assert!(same.text.ends_with("all 4 compared figures are ok\n"));

        let slower = file("on-pi", 9_000.0, 7_200, [0.1, 0.5, 0.8]);
        let worse = compare("a", &a, "b", &slower);
        assert_eq!(worse.worse, 2);
        assert!(worse.text.contains("ran at 8000 and 7200 q/s"));
        assert!(worse.failure().unwrap().contains("2 of 4"));
    }

    #[test]
    fn a_class_that_b_lacks_is_missing_and_different_places_are_noted() {
        let a = file("on-pi", 10_000.0, 8_000, [0.2, 0.4, 0.8]);
        let b = file_json("from-here", 1, &[]);
        let comparison = compare("a", &a, "b", &b);
        assert_eq!((comparison.compared, comparison.missing), (0, 4));
        assert!(comparison.text.contains("cached: not in b"));
        assert!(comparison.text.contains("different places"));
        assert!(comparison.failure().is_some());
    }
}
