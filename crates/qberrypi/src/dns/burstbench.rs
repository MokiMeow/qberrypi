//! A burst of fresh names against two fake DNS-over-TLS upstreams that, like a public resolver,
//! will not take everything: what share is answered, how soon forwarding works again once the burst
//! is over, and how another device fares meanwhile. Ignored, as each takes a minute or two and is a
//! measurement; run them alone, in release:
//!
//! `cargo test -p qberrypi --release --lib burstbench -- --ignored --nocapture --test-threads=1`
//!
//! The service is the real one (listener, forward slots, exchange, connections) with the
//! connections' standard waits, over two [`TlsFakeUpstream`]s on the loopback. Nothing leaves this
//! computer. One client, `127.0.0.3`, sends fresh names on a schedule: [`BURST_RATE`] a second for
//! [`BURST_FOR`] in batches of [`BATCH`] (as a page or a DGA loop asks them), and in one scenario
//! a household's pace after that. Another, `127.0.0.2`, asks one fresh name every
//! [`VICTIM_EVERY`] from the start of the load until a while after its end. Each scenario prints
//! its figures, and how long both connections were down at once.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::client::{query, read_answer};
use super::dot::Link;
use super::testutil::{
    Reply, TlsFakeUpstream, TlsMode, closed_port, ip, ms, service, tls_for_fakes,
};
use super::upstream::{TlsState, UDP_BUFFER, Upstreams};

/// The questions a second of the burst.
const BURST_RATE: usize = 500;
/// How long the burst lasts.
const BURST_FOR: Duration = Duration::from_secs(5);
/// The questions that go out together, every `BATCH / BURST_RATE` seconds.
const BATCH: usize = 25;
/// How often the other device asks.
const VICTIM_EVERY: Duration = Duration::from_millis(50);
/// How long a client waits for its answer: as long as the service's own wait, and a little.
const CLIENT_WAIT: Duration = Duration::from_millis(2100);
/// How often the connections' links are looked at.
const SAMPLE_EVERY: Duration = Duration::from_millis(10);

/// One client's questions: when each went out (from the start of the load) and what came back.
struct Asked {
    /// The send time of each question, by its ID.
    sent: Vec<Option<Duration>>,
    /// When its answer came and its response code, by ID.
    got: Vec<Option<(Duration, u8)>>,
}

impl Asked {
    /// Whether the question `id` was answered with NOERROR within the client's wait.
    fn answered(&self, id: usize) -> bool {
        match (self.sent[id], self.got[id]) {
            (Some(sent), Some((at, rcode))) => rcode == 0 && at.saturating_sub(sent) <= CLIENT_WAIT,
            _ => false,
        }
    }
}

/// Sends the questions of `schedule` (each one's send time from `start`, and its name) from
/// `from`, to `server`, each under its index as its ID, and gathers the answers until the last one
/// had its wait.
fn ask_all(
    from: Ipv4Addr,
    server: SocketAddr,
    start: Instant,
    schedule: Vec<(Duration, String)>,
) -> Asked {
    let socket = UdpSocket::bind((from, 0)).unwrap();
    socket.set_read_timeout(Some(ms(20))).unwrap();
    let reader = socket.try_clone().unwrap();
    let count = schedule.len();
    let got = Arc::new(Mutex::new(vec![None; count]));
    let done = Arc::new(AtomicBool::new(false));
    let receiver = {
        let (got, done) = (got.clone(), done.clone());
        thread::spawn(move || {
            let mut buf = [0u8; UDP_BUFFER];
            while !done.load(Ordering::Relaxed) {
                let Ok(len) = reader.recv(&mut buf) else {
                    continue;
                };
                let at = start.elapsed();
                let id = usize::from(u16::from_be_bytes([buf[0], buf[1]]));
                let answer = read_answer(&buf[..len], id as u16);
                let mut got = got.lock().unwrap_or_else(PoisonError::into_inner);
                if let (Some(slot), Some(answer)) = (got.get_mut(id), answer)
                    && slot.is_none()
                {
                    *slot = Some((at, answer.rcode));
                }
            }
        })
    };
    let mut sent = vec![None; count];
    for (id, (when, name)) in schedule.into_iter().enumerate() {
        if let Some(wait) = (start + when).checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        let packet = query(id as u16, &name, 1).unwrap();
        if socket.send_to(&packet, server).is_ok() {
            sent[id] = Some(start.elapsed());
        }
    }
    thread::sleep(CLIENT_WAIT + ms(100));
    done.store(true, Ordering::Relaxed);
    receiver.join().unwrap();
    let got = got.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Asked { sent, got }
}

/// The load of one scenario: the send times of the flooding client's names, when the load ends,
/// and how long the other device goes on asking after that (the window in which forwarding must
/// come back).
struct Load {
    schedule: Vec<(Duration, String)>,
    ends: Duration,
    watch_after: Duration,
}

/// [`BURST_RATE`] fresh names a second for [`BURST_FOR`], [`BATCH`] at a time.
fn burst() -> Load {
    let schedule = (0..BURST_RATE * BURST_FOR.as_secs() as usize)
        .map(|n| {
            let batch = (n / BATCH) as u32;
            let when = Duration::from_secs(1) * batch * BATCH as u32 / BURST_RATE as u32;
            (when, format!("f{n}.burst.example.com"))
        })
        .collect();
    Load {
        schedule,
        ends: BURST_FOR,
        watch_after: Duration::from_secs(30),
    }
}

/// [`burst`], and then the pace of a busy household, 50 fresh names a second for 25 s, which
/// alone the upstreams carry easily: whether forwarding comes back while the load goes on, as it
/// did not on the Pi after the bench's 200-a-second step (every lower step lost all).
fn burst_then_household() -> Load {
    let mut load = burst();
    let after = Duration::from_secs(25);
    let count = 50 * after.as_secs() as u32;
    for n in 0..count {
        let name = format!("h{n}.after.example.com");
        load.schedule.push((load.ends + after * n / count, name));
    }
    load.ends += after;
    load.watch_after = Duration::from_secs(30);
    load
}

/// How the links of the connections went, sampled every [`SAMPLE_EVERY`].
#[derive(Default)]
struct Links {
    /// The time that every connection was down at once.
    all_down: Duration,
    /// The longest stretch of that.
    longest_all_down: Duration,
    /// The time that some connection was down.
    any_down: Duration,
}

/// What one scenario gave.
struct Figures {
    burst_answered: f64,
    victim_during: f64,
    /// From the end of the load to the first question of the other device after which every one
    /// was answered; `None` if the last ones were not.
    recovered_after: Option<Duration>,
    victim_after: f64,
    links: Links,
    summary: String,
}

/// The load and the other device against two fakes that answer `192.0.2.10` and `192.0.2.20`
/// after `reply_after`, under `mode`.
fn scenario(load: Load, reply_after: Duration, mode: TlsMode) -> Figures {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let first = TlsFakeUpstream::start_with(Reply::After(reply_after, ip("192.0.2.10")), mode);
    let second = TlsFakeUpstream::start_with(Reply::After(reply_after, ip("192.0.2.20")), mode);
    let Load {
        schedule,
        ends,
        watch_after,
    } = load;
    let (counts, asked, links) = runtime.block_on(async {
        let ups = Upstreams::with(
            vec![closed_port(), closed_port()],
            super::upstream::UDP_WAIT,
            super::upstream::TCP_WAIT,
        )
        .with_tls_at(
            tls_for_fakes(&[&first, &second]),
            vec![first.addr, second.addr],
        );
        ups.start_tls();
        let (dns, addr) = service(ups).await;
        while !matches!(dns.upstreams().tls(), TlsState::On { up: 2, .. }) {
            tokio::time::sleep(ms(5)).await;
        }
        // Warm: a latency figure, as a household gives it.
        for n in 0..20 {
            let name = format!("warm{n}.example.net");
            let _ = super::testutil::ask_udp(addr, &name).await;
        }
        let start = Instant::now() + ms(50);
        let victims = ((ends + watch_after).as_millis() / VICTIM_EVERY.as_millis()) as u32;
        let victim: Vec<(Duration, String)> = (0..victims)
            .map(|n| (VICTIM_EVERY * n, format!("v{n}.other.example.org")))
            .collect();
        let flood =
            tokio::task::spawn_blocking(move || ask_all(ip("127.0.0.3"), addr, start, schedule));
        let other =
            tokio::task::spawn_blocking(move || ask_all(ip("127.0.0.2"), addr, start, victim));
        let sampler = {
            let dns = dns.clone();
            tokio::spawn(async move {
                let mut links = Links::default();
                let mut stretch = Duration::ZERO;
                loop {
                    tokio::time::sleep(SAMPLE_EVERY).await;
                    let now = dns.upstreams().links();
                    if now.iter().all(|&link| link == Link::Down) {
                        links.all_down += SAMPLE_EVERY;
                        stretch += SAMPLE_EVERY;
                        links.longest_all_down = links.longest_all_down.max(stretch);
                    } else {
                        stretch = Duration::ZERO;
                    }
                    if now.contains(&Link::Down) {
                        links.any_down += SAMPLE_EVERY;
                    }
                    if Instant::now() > start + ends + watch_after {
                        return links;
                    }
                }
            })
        };
        let flood = flood.await.unwrap();
        let other = other.await.unwrap();
        let links = sampler.await.unwrap();
        let ups = dns.upstreams();
        let counts = match ups.tls() {
            TlsState::On { counts, up } => format!("{counts:?}, up {up}"),
            other => format!("{other:?}"),
        };
        (
            format!(
                "upstream failures {}, queries {}, shed {}, {counts}",
                ups.failures(),
                ups.queries(),
                dns.forwards_shed()
            ),
            (flood, other),
            links,
        )
    });
    let (flood, other) = asked;
    let share = |asked: &Asked, ids: &mut dyn Iterator<Item = usize>| {
        let (mut yes, mut all) = (0usize, 0usize);
        for id in ids {
            all += 1;
            yes += usize::from(asked.answered(id));
        }
        100.0 * yes as f64 / all.max(1) as f64
    };
    let burst_answered = share(&flood, &mut (0..flood.sent.len()));
    let during = |id: &usize| other.sent[*id].is_some_and(|sent| sent < ends);
    let victim_during = share(&other, &mut (0..other.sent.len()).filter(during));
    let victim_after = share(&other, &mut (0..other.sent.len()).filter(|id| !during(id)));
    // The first question after the load from which on every one was answered.
    let mut recovered_after = None;
    for id in (0..other.sent.len()).rev() {
        let Some(sent) = other.sent[id] else { continue };
        if sent < ends {
            recovered_after.get_or_insert(Duration::ZERO);
            break;
        }
        if !other.answered(id) {
            break;
        }
        recovered_after = Some(sent - ends);
    }
    let summary = format!(
        "{counts}; fakes: closed {} + {}, rate-dropped {} + {} (penalties {} + {}), queries {} + {}, \
         handshakes {} + {}",
        first.closed(),
        second.closed(),
        first.rate_dropped(),
        second.rate_dropped(),
        first.penalties(),
        second.penalties(),
        first.queries(),
        second.queries(),
        first.handshakes(),
        second.handshakes(),
    );
    Figures {
        burst_answered,
        victim_during,
        recovered_after,
        victim_after,
        links,
        summary,
    }
}

/// Prints one scenario's figures.
fn report(name: &str, figures: &Figures, watch_after: Duration) {
    let recovered = figures.recovered_after.map_or_else(
        || format!("> {} s", watch_after.as_secs()),
        |at| format!("{} ms", at.as_millis()),
    );
    let links = &figures.links;
    println!(
        "{name}: load answered {:.1} %, other device during the load {:.1} %, after it {:.1} %, \
         forwarding back {recovered} after the load; both connections down {} ms in all \
         (longest {} ms), one down {} ms\n    {}",
        figures.burst_answered,
        figures.victim_during,
        figures.victim_after,
        links.all_down.as_millis(),
        links.longest_all_down.as_millis(),
        links.any_down.as_millis(),
        figures.summary
    );
}

/// Prints one scenario's figures, and holds them to what a household needs: the other device is
/// answered during the load and after it, forwarding is back within 2 s of its end, and the
/// connections are never both down for more than a second.
fn report_and_check(name: &str, figures: &Figures, watch_after: Duration) {
    report(name, figures, watch_after);
    assert!(
        figures.victim_during >= 99.0 && figures.victim_after >= 99.0,
        "{name}: the other device was answered {:.1} % during and {:.1} % after",
        figures.victim_during,
        figures.victim_after
    );
    assert!(
        figures
            .recovered_after
            .is_some_and(|after| after <= Duration::from_secs(2)),
        "{name}: forwarding back {:?} after the load",
        figures.recovered_after
    );
    assert!(
        figures.links.longest_all_down <= Duration::from_secs(1),
        "{name}: both connections down for {:?} at once",
        figures.links.longest_all_down
    );
}

/// The server that ends a session on which too many questions are out.
fn closing() -> TlsMode {
    TlsMode {
        max_in_flight: Some(24),
        ..TlsMode::default()
    }
}

#[test]
#[ignore = "a measurement of a minute; see the module's text"]
fn burstbench_a_server_that_closes_a_session_with_too_many_questions_out() {
    let load = burst();
    let watch = load.watch_after;
    let figures = scenario(load, ms(40), closing());
    report_and_check(
        "500/s for 5 s, closes past 24 in flight, answers in 40 ms",
        &figures,
        watch,
    );
}

#[test]
#[ignore = "a measurement of a minute; see the module's text"]
fn burstbench_a_server_that_rate_limits() {
    let mode = TlsMode {
        rate: Some(200),
        ..TlsMode::default()
    };
    let load = burst();
    let watch = load.watch_after;
    let figures = scenario(load, ms(20), mode);
    report_and_check(
        "500/s for 5 s, answers 200/s per address and drops the rest, in 20 ms",
        &figures,
        watch,
    );
}

#[test]
#[ignore = "a measurement of a minute; see the module's text"]
fn burstbench_a_burst_then_a_households_pace_against_a_server_that_closes_busy_sessions() {
    let load = burst_then_household();
    let watch = load.watch_after;
    // A handshake as slow as the Pi's: the questions pile up while a session opens.
    let mode = TlsMode {
        handshake_delay: ms(150),
        ..closing()
    };
    let figures = scenario(load, ms(40), mode);
    report_and_check(
        "500/s for 5 s then 50/s for 25 s, closes past 24 in flight, answers in 40 ms, \
         handshakes in 150 ms",
        &figures,
        watch,
    );
}

#[test]
#[ignore = "a measurement of a minute; see the module's text"]
fn burstbench_a_server_that_throttles_a_source_that_sends_too_much() {
    // Past 100 a second at one address, it drops everything from this source for 10 s: what
    // forwarding loses then is the server's doing, and what it loses past that is ours.
    let mode = TlsMode {
        rate: Some(100),
        penalty: Some(Duration::from_secs(10)),
        ..TlsMode::default()
    };
    let load = burst();
    let watch = load.watch_after;
    let figures = scenario(load, ms(20), mode);
    let name = "500/s for 5 s, past 100/s per address drops all for 10 s, answers in 20 ms";
    report(name, &figures, watch);
    // Whether the server's penalty trips depends on the bursts that the admission lets through
    // (`admit::UPSTREAM_BURST`, `CLIENT_BURST`), which are a trade against a household's own
    // bursts. What is ours to hold: once the penalty is over (10 s after it began, which is in the
    // first second of the load), forwarding is back within 2 s, and the other device then too.
    assert!(
        figures
            .recovered_after
            .is_some_and(|after| after <= Duration::from_secs(10 - 5 + 2)),
        "{name}: forwarding back {:?} after the load",
        figures.recovered_after
    );
    assert!(
        figures.victim_after >= 80.0,
        "{name}: {:.1} %",
        figures.victim_after
    );
}
