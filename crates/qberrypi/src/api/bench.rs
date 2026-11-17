//! `POST /api/qbp/bench`: the load generator of `qb-bench`, run on the board against its own DNS
//! service over the loopback, so that the figure of what qberrypi sustains is not the Wi-Fi's.
//!
//! The body is `{"class": "cached" | "blocked" | "forwarded", "seconds": 1..=30, "mode": "ramp" |
//! "steady", "qps": n}`. `seconds` is 10 and `mode` is `ramp` when left out; `qps` is the steady
//! load's rate, which it needs, or the ramp's cap. A forwarded load asks the upstreams (Quad9) for
//! every question, and each of those really goes to Quad9, which throttles a source that sends too
//! many fresh names: its rate is at most [`FORWARDED_MAX_QPS`] and its ramp's cap
//! [`FORWARDED_DEFAULT_MAX`] (50) by default.
//! Any other key, or a value out of range, is a 400 and nothing runs.
//!
//! The answer is `{"bench": <run>, "took"}`, the run being `qb_bench::run::report_json`'s object:
//! every step with its rate, answers, losses, latency percentiles, each core's busy share, and the
//! processor time of the generator's thread apart from the rest of the process, which is the
//! server's. A check of the class's names that fails (a blocked name that is not blocked, say) is a
//! 422 that says what was answered.
//!
//! One run at a time: a request while one runs is a 409. The run takes the thread of the API's
//! connection, which is a thread of the blocking pool (`api::serve`), for as long as it lasts, at
//! most its seconds and one two-second wait; nothing of it runs on the runtime's thread, and
//! nothing of it exists while no run does. Its questions come from `127.0.0.1`, which the table of
//! devices does not count; they are counted in the statistics and the query log as any question
//! is, and the watchdog sees their answers.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use qb_bench::cpu::Server;
use qb_bench::engine::{Engine, Limits, Transport};
use qb_bench::names::Class;
use qb_bench::run::{self, FORWARDED_DEFAULT_MAX, Mode, Settings, prepare, report_json};
use serde_json::{Value, json};

use super::http::Reply;
use super::routes::{Error, error, error_with, ok};

/// The path of the route.
pub const PATH: &str = "/api/qbp/bench";
/// The longest run.
pub const MAX_SECONDS: u64 = 30;
/// The seconds of a run that does not say.
pub const DEFAULT_SECONDS: u64 = 10;
/// The most forwarded questions a second a run may ask for: each is a fresh name that goes to the
/// upstream, Quad9, and past 50 a second the resolver paces them itself
/// ([`UPSTREAM_RATE`](crate::dns::admit::UPSTREAM_RATE)).
pub const FORWARDED_MAX_QPS: u32 = 100;
/// How long a question of a run waits for its answer before it counts as lost.
pub const WAIT: Duration = Duration::from_secs(2);
/// The most questions of a run that are out at once.
pub const IN_FLIGHT: usize = 10_000;

/// A body that is not what the route takes.
const BAD_BODY: Error = (
    "bad_request",
    "the body must be {\"class\": \"cached\"|\"blocked\"|\"forwarded\", \"seconds\": 1 to 30, \
     \"mode\": \"ramp\"|\"steady\", \"qps\": n}, with qps for a steady load, and at most 100 for \
     the forwarded class",
);
/// A run while another runs.
const BUSY: Error = (
    "bench_running",
    "a bench is running; ask again when it is done",
);

/// What a request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    pub class: Class,
    pub seconds: u64,
    pub mode: Mode,
}

/// The run that a body asks for, or `None` for one that is not a body of the route.
pub fn asked(body: &[u8]) -> Option<Asked> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let fields = value.as_object()?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "class" | "seconds" | "mode" | "qps"))
    {
        return None;
    }
    let class = Class::from_word(fields.get("class")?.as_str()?)?;
    let seconds = match fields.get("seconds") {
        None => DEFAULT_SECONDS,
        Some(value) => value.as_u64().filter(|s| (1..=MAX_SECONDS).contains(s))?,
    };
    let qps = match fields.get("qps") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|&q| q >= 1)
                .and_then(|q| u32::try_from(q).ok())?,
        ),
    };
    if class == Class::Forwarded && qps.is_some_and(|q| q > FORWARDED_MAX_QPS) {
        return None;
    }
    let ramp = |max: u32| {
        let Mode::Ramp {
            start,
            max: default,
        } = Mode::ramp_of(class)
        else {
            return None;
        };
        let max = if class == Class::Forwarded && max == 0 {
            FORWARDED_DEFAULT_MAX
        } else if max == 0 {
            default
        } else {
            max
        };
        Some(Mode::Ramp {
            start: start.min(max),
            max,
        })
    };
    let mode = match fields.get("mode").map(Value::as_str) {
        None | Some(Some("ramp")) => ramp(qps.unwrap_or(0))?,
        Some(Some("steady")) => Mode::Steady { qps: qps? },
        Some(_) => return None,
    };
    Some(Asked {
        class,
        seconds,
        mode,
    })
}

/// What carries out a run: the real one loads `127.0.0.1:53`; a test's may be anything.
type Runner = Box<dyn Fn(&Asked) -> Result<Value, String> + Send + Sync>;

/// The route's state: whether a run is under way, and what carries one out.
pub struct Bench {
    running: AtomicBool,
    runner: Runner,
}

/// Holds the one run's place, and gives it back when dropped.
struct Place<'a>(&'a AtomicBool);

impl Drop for Place<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Bench {
    /// The route over the DNS service at `target`, which is the board's own `127.0.0.1:53`.
    pub fn new(target: SocketAddr) -> Bench {
        Bench::with_runner(Box::new(move |asked| load(target, asked)))
    }

    /// The route with `runner` in place of the real load.
    pub fn with_runner(runner: Runner) -> Bench {
        Bench {
            running: AtomicBool::new(false),
            runner,
        }
    }

    /// Answers a `POST` of `body` (the session, the method and the query string have been
    /// checked): 400 for a body that is not the route's, 409 while a run is under way, and the run.
    /// It blocks for as long as the run lasts.
    pub fn handle(&self, body: &[u8], started: Instant) -> Reply {
        let Some(asked) = asked(body) else {
            return error(400, BAD_BODY, started);
        };
        if self
            .running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return error(409, BUSY, started);
        }
        let _place = Place(&self.running);
        log::info!(
            "api: bench: {} {} for at most {} s",
            asked.class.word(),
            asked.mode.word(),
            asked.seconds
        );
        match (self.runner)(&asked) {
            Ok(report) => {
                match report.get("max_sustained_qps").and_then(Value::as_f64) {
                    Some(qps) => log::info!("api: bench: done, {qps:.0} answers/s sustained"),
                    None => log::info!("api: bench: done, no step passed"),
                }
                ok(json!({ "bench": report }), started)
            }
            Err(why) => {
                log::warn!("api: bench: not run: {why}");
                error_with(422, "bench_failed", &why, started)
            }
        }
    }
}

/// The real run: the class's checks and its load against `target`, over UDP, on this thread.
fn load(target: SocketAddr, asked: &Asked) -> Result<Value, String> {
    let limits = Limits {
        timeout: WAIT,
        max_in_flight: IN_FLIGHT,
    };
    let mut engine = Engine::connect(target, Transport::Udp, limits)
        .map_err(|error| format!("cannot open a socket to the DNS service: {error}"))?;
    let (mut names, list) = prepare(&mut engine, asked.class, &[])?;
    let settings = Settings::new(
        asked.class,
        asked.mode,
        Duration::from_secs(asked.seconds),
        Server::ThisProcess,
    );
    let report = run::run(
        &mut engine,
        &mut names,
        &settings,
        run::shown(&list),
        |_| {},
    );
    Ok(report_json(&report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use crate::dns::testutil::{Reply as Upstream, ip, one};

    fn body(text: &str) -> Option<Asked> {
        asked(text.as_bytes())
    }

    #[test]
    fn a_body_names_the_class_and_may_give_the_seconds_the_mode_and_the_rate() {
        assert_eq!(
            body(r#"{"class": "cached"}"#),
            Some(Asked {
                class: Class::Cached,
                seconds: 10,
                mode: Mode::Ramp {
                    start: 1_000,
                    max: 200_000
                },
            })
        );
        assert_eq!(
            body(r#"{"class": "blocked", "seconds": 30, "mode": "steady", "qps": 5000}"#)
                .map(|a| (a.seconds, a.mode)),
            Some((30, Mode::Steady { qps: 5000 }))
        );
        // The rate of a ramp is its cap; a forwarded ramp's cap is modest by default.
        assert_eq!(
            body(r#"{"class": "cached", "qps": 50000}"#).map(|a| a.mode),
            Some(Mode::Ramp {
                start: 1_000,
                max: 50_000
            })
        );
        assert_eq!(
            body(r#"{"class": "forwarded"}"#).map(|a| a.mode),
            Some(Mode::Ramp { start: 25, max: 50 })
        );
        assert_eq!(
            body(r#"{"class": "forwarded", "qps": 10}"#).map(|a| a.mode),
            Some(Mode::Ramp { start: 10, max: 10 })
        );
    }

    #[test]
    fn a_body_out_of_range_or_with_another_key_is_refused() {
        for bad in [
            "",
            "[]",
            r#"{"seconds": 5}"#,
            r#"{"class": "all"}"#,
            r#"{"class": "cached", "seconds": 0}"#,
            r#"{"class": "cached", "seconds": 31}"#,
            r#"{"class": "cached", "seconds": 2.5}"#,
            r#"{"class": "cached", "mode": "steady"}"#,
            r#"{"class": "cached", "mode": "burst"}"#,
            r#"{"class": "cached", "mode": 1}"#,
            r#"{"class": "cached", "qps": 0}"#,
            r#"{"class": "cached", "qps": 5000000000}"#,
            r#"{"class": "forwarded", "mode": "steady", "qps": 101}"#,
            r#"{"class": "cached", "target": "192.0.2.1"}"#,
        ] {
            assert_eq!(body(bad), None, "{bad}");
        }
    }

    fn json(reply: &Reply) -> Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn one_run_at_a_time_and_a_second_is_a_409() {
        let (entered_tx, entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let bench = Arc::new(Bench::with_runner(Box::new(move |asked| {
            entered_tx.send(()).unwrap();
            released.lock().unwrap().recv().unwrap();
            Ok(json!({ "class": asked.class.word(), "max_sustained_qps": 1.0 }))
        })));
        let first = {
            let bench = bench.clone();
            std::thread::spawn(move || bench.handle(br#"{"class": "cached"}"#, Instant::now()))
        };
        entered.recv().unwrap();
        let second = bench.handle(br#"{"class": "blocked"}"#, Instant::now());
        assert_eq!(second.status, 409);
        assert_eq!(json(&second)["error"]["key"], "bench_running");
        // A bad body is refused before the place is looked at.
        assert_eq!(bench.handle(b"{}", Instant::now()).status, 400);
        release.send(()).unwrap();
        let first = first.join().unwrap();
        assert_eq!(first.status, 200);
        assert_eq!(json(&first)["bench"]["class"], "cached");
        assert!(json(&first)["took"].is_number());
        // The place is free again.
        release.send(()).unwrap();
        assert_eq!(
            bench
                .handle(br#"{"class": "cached"}"#, Instant::now())
                .status,
            200
        );
    }

    #[test]
    fn a_run_that_cannot_be_made_is_a_422_that_says_why() {
        let bench = Bench::with_runner(Box::new(|_| Err("doubleclick.net: NOERROR".to_owned())));
        let reply = bench.handle(br#"{"class": "blocked"}"#, Instant::now());
        assert_eq!(reply.status, 422);
        assert_eq!(json(&reply)["error"]["message"], "doubleclick.net: NOERROR");
        // And the place is free after it.
        assert_eq!(
            bench
                .handle(br#"{"class": "blocked"}"#, Instant::now())
                .status,
            422
        );
    }

    #[tokio::test]
    async fn the_board_loads_its_own_dns_and_the_generator_is_counted_apart() {
        let (_dns, addr, _upstream) = one(Upstream::Answer(ip("192.0.2.10"))).await;
        let bench = Arc::new(Bench::new(addr));
        let reply = tokio::task::spawn_blocking(move || {
            bench.handle(
                br#"{"class": "cached", "mode": "steady", "qps": 200, "seconds": 1}"#,
                Instant::now(),
            )
        })
        .await
        .unwrap();
        assert_eq!(reply.status, 200);
        let run = &json(&reply)["bench"];
        assert_eq!(run["class"], "cached");
        let step = &run["steps"][0];
        assert_eq!(
            (step["sent"].as_u64(), step["answered"].as_u64()),
            (Some(200), Some(200))
        );
        assert!(step["cpu"]["generator_pct"].is_number());
        assert!(step["cpu"]["server_pct"].is_number());
        assert!(
            step["cpu"]["cores_busy_pct"]
                .as_array()
                .is_some_and(|c| !c.is_empty())
        );
        // No address is in it.
        assert!(!String::from_utf8_lossy(&reply.body).contains("127.0.0.1"));
    }
}
