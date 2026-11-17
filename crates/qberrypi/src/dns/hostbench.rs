//! Timings on this computer of the DNS service's path from a datagram in to its answer out: how
//! many answers a second the listener gives over the loopback for each class, and where the time
//! of one answer goes. They profile the software without the Pi: the board's cores are slower,
//! but the shares are what to look at. Ignored, as timings are; run them in release, alone:
//!
//! `cargo test -p qberrypi --release hostbench -- --ignored --nocapture --test-threads=1`
//!
//! The service is set up as the board runs it: a block set of [`BLOCKED_NAMES`] names, the
//! watchdog's feed with its analysis on the listener's runtime, and the front panel's lane, which
//! a thread empties as the panel does. The upstream is a fake on the loopback, on a runtime of its
//! own, so a forwarded question leaves nothing but this computer's loopback.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use qb_bench::cpu::{Server, task_ticks, ticks_per_second};
use qb_bench::engine::{Engine, Limits, Transport};
use qb_bench::names::Class;
use qb_bench::run::{self, Bounds, Mode, Settings, prepare};
use qb_dns::{Action, BlockSet, BlockSetBuilder, Outcome, Stats, Transport as Via};

use super::client::query;
use super::listen::{bind, serve};
use super::testutil::{FakeUpstream, FixedOffset, Reply, ip, ms};
use super::upstream::Upstreams;
use super::{Dns, TYPE_A};
use crate::alerts::NullSink;
use crate::lane::LaneFeed;
use crate::watch::behaviour::Behaviour;
use crate::watch::rebind::RebindPolicy;
use crate::watch::{Watch, analysis, feed};

/// The names of the block set: about as many as the household's lists hold.
const BLOCKED_NAMES: usize = 1_200_000;
/// The blocked names a load asks: the generator's first candidate, and a few of the set.
const ASKED_BLOCKED: [&str; 4] = [
    "doubleclick.net",
    "t0000017.ads17.example",
    "t0500000.ads654.example",
    "t1199999.ads750.example",
];

/// A block set of [`BLOCKED_NAMES`] names under some thousands of domains, and `doubleclick.net`.
fn big_set() -> BlockSet {
    let mut text = String::with_capacity(BLOCKED_NAMES * 32);
    text.push_str("0.0.0.0 doubleclick.net\n");
    for i in 0..BLOCKED_NAMES {
        let _ = writeln!(text, "0.0.0.0 t{i:07}.ads{}.example", i % 4093);
    }
    let mut builder = BlockSetBuilder::new();
    builder.add_list(text.as_bytes());
    builder.build()
}

/// The CPU time of the thread `tid` of this process, in ticks.
fn thread_ticks(tid: i32) -> Option<u64> {
    task_ticks(&std::fs::read_to_string(format!("/proc/self/task/{tid}/stat")).ok()?)
}

/// A `Dns` as the board has it, over `upstream`: the big set, a watch whose analysis the caller
/// runs, and a lane that a thread empties every 33 ms until `stop`.
fn board_dns(
    upstream: SocketAddr,
    stop: &Arc<AtomicBool>,
) -> (Arc<Dns>, std::sync::mpsc::Receiver<crate::watch::Event>) {
    let dns = Dns::new(
        big_set(),
        Upstreams::with(vec![upstream], ms(300), ms(600)),
        Box::new(FixedOffset(0)),
    );
    // The fake upstream is this computer's: the board's limit on fresh names a second to the
    // upstreams would make the forwarded class a measure of that limit.
    dns.lift_upstream_rate();
    let (feed, events) = feed();
    dns.set_watch(Watch {
        feed,
        rebind: RebindPolicy::new(Vec::new()),
        sink: Arc::new(NullSink),
    });
    let lane = LaneFeed::new();
    dns.set_lane(lane.clone());
    let stop = stop.clone();
    thread::spawn(move || {
        let mut out = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            lane.take(&mut out);
            out.clear();
            thread::sleep(Duration::from_millis(33));
        }
    });
    (dns, events)
}

/// A fake upstream on a runtime of its own, answering every question at once; it lives until
/// `stop`.
fn upstream(stop: &Arc<AtomicBool>) -> SocketAddr {
    let (tx, rx) = mpsc::channel();
    let stop = stop.clone();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let up = FakeUpstream::start(Reply::Answer(ip("192.0.2.10"))).await;
            tx.send(up.addr).unwrap();
            while !stop.load(Ordering::Relaxed) {
                tokio::time::sleep(ms(50)).await;
            }
        });
    });
    rx.recv().unwrap()
}

/// The ramp of one class against the listener at `addr`, with the share of a core that the
/// listener's thread `tid` took at the best step.
fn ramp(addr: SocketAddr, class: Class, tid: i32) -> String {
    let limits = Limits {
        timeout: Duration::from_millis(500),
        max_in_flight: 10_000,
    };
    let mut engine = Engine::connect(addr, Transport::Udp, limits).unwrap();
    let (mut names, list) = prepare(&mut engine, class, &[]).unwrap();
    let settings = Settings {
        step: Duration::from_millis(500),
        bounds: Bounds::with_p99(Duration::from_millis(5)),
        ..Settings::new(
            class,
            Mode::Ramp {
                start: 5_000,
                max: 1_000_000,
            },
            Duration::from_secs(8),
            Server::ThisProcess,
        )
    };
    let ticks = ticks_per_second();
    let mut listener_pct = Vec::new();
    let mut before = (Instant::now(), thread_ticks(tid));
    let report = run::run(
        &mut engine,
        &mut names,
        &settings,
        run::shown(&list),
        |_| {
            let after = (Instant::now(), thread_ticks(tid));
            let secs = after.0.duration_since(before.0).as_secs_f64();
            let pct = after
                .1
                .zip(before.1)
                .map(|(a, b)| (a - b) as f64 * 100.0 / ticks as f64 / secs);
            listener_pct.push(pct);
            before = (Instant::now(), thread_ticks(tid));
        },
    );
    let best = report
        .steps
        .iter()
        .zip(&listener_pct)
        .filter(|(step, _)| step.passed())
        .max_by(|a, b| a.0.answered_qps().total_cmp(&b.0.answered_qps()));
    // Each step's rate and how it ended, so that what stopped the ramp is seen.
    let steps: Vec<String> = report
        .steps
        .iter()
        .zip(&listener_pct)
        .map(|(step, pct)| {
            format!(
                "{}:{}@{:.0}%",
                step.target_qps,
                step.miss.map_or("pass", qb_bench::run::Miss::word),
                pct.unwrap_or(0.0)
            )
        })
        .collect();
    let steps = format!(
        "{:<10}steps (rate:verdict@listener): {}\n",
        "",
        steps.join(" ")
    );
    let line = match best {
        Some((step, pct)) => format!(
            "{:<10}{:>12.0}{:>10}{:>10}{:>10.1}{:>12.1}{}\n",
            class.word(),
            step.answered_qps(),
            qb_bench::figures::ms_text(step.latency.p50_us),
            qb_bench::figures::ms_text(step.latency.p99_us),
            step.cpu.generator_pct.unwrap_or(0.0),
            pct.unwrap_or(0.0),
            if report.at_cap { "  (at the cap)" } else { "" },
        ),
        None => format!("{:<10}no step passed\n", class.word()),
    };
    line + &steps
}

#[test]
#[ignore = "a timing, for release builds; see the module's text"]
fn hostbench_listener_answers_per_second_by_class() {
    let stop = Arc::new(AtomicBool::new(false));
    let up = upstream(&stop);
    let (dns, events) = board_dns(up, &stop);
    let (tx, rx) = mpsc::channel();
    {
        let dns = dns.clone();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listeners = bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                    .await
                    .unwrap();
                tx.send((
                    listeners.addr(),
                    rustix::thread::gettid().as_raw_nonzero().get(),
                ))
                .unwrap();
                tokio::spawn(analysis::run(
                    events,
                    Behaviour::new(Vec::new()),
                    Arc::new(NullSink),
                    analysis::clock(),
                ));
                let _ = serve(dns, listeners).await;
            });
        });
    }
    let (addr, tid) = rx.recv().unwrap();
    println!(
        "listener on the loopback of this computer, one runtime thread, {BLOCKED_NAMES} names \
         blocked, watch and lane on; ramp steps of 0.5 s, p99 under 5 ms, loss under 0.1%"
    );
    println!(
        "{:<10}{:>12}{:>10}{:>10}{:>10}{:>12}",
        "class", "answers/s", "p50 ms", "p99 ms", "gen %", "listener %"
    );
    for class in Class::ALL {
        print!("{}", ramp(addr, class, tid));
    }
    println!(
        "watch events dropped (feed full): {}; prefetches skipped: {}",
        dns.watch_events_dropped(),
        dns.prefetches_skipped()
    );
    stop.store(true, Ordering::Relaxed);
}

/// The sections of one answer's path through `serve_udp`, as timed.
const SECTIONS: [&str; 12] = [
    "recv_from (syscall)",
    "note_client",
    "decide: filter_for (exempt check)",
    "decide: now() (two clock reads)",
    "decide: resolver lock (+ hold marks)",
    "decide: lane count",
    "decide: handle_query_filtered",
    "decide: lane offer",
    "decide: unlock (hold mark)",
    "try_send_to (syscall)",
    "note_answered",
    "observe (watch feed offer)",
];

/// Runs `batches` of `batch` questions of `packets` through a replica of `serve_udp`'s path for an
/// answer known at once, with a section timed at each step: the nanoseconds of each section per
/// answer, the clock's own cost taken out.
fn sections(dns: &Arc<Dns>, packets: &[Vec<u8>], batches: usize, batch: usize) -> [f64; 12] {
    let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    client.connect(server.local_addr().unwrap()).unwrap();
    server.set_nonblocking(true).unwrap();
    let mut buf = vec![0u8; 4096];
    let mut reply_buf = vec![0u8; 4096];
    let mut total = [0u128; 12];
    // The cost of one clock read, which every section has once.
    let clock = {
        let started = Instant::now();
        for _ in 0..100_000 {
            std::hint::black_box(Instant::now());
        }
        started.elapsed().as_nanos() as f64 / 100_000.0
    };
    let mut answers = 0usize;
    for round in 0..batches {
        for i in 0..batch {
            client
                .send(&packets[(round * batch + i) % packets.len()])
                .unwrap();
        }
        for _ in 0..batch {
            let mut t = [Instant::now(); 13];
            let (len, from) = loop {
                if let Ok(got) = server.recv_from(&mut buf) {
                    break got;
                }
                t[0] = Instant::now();
            };
            t[1] = Instant::now();
            dns.note_client(from.ip());
            t[2] = Instant::now();
            let client_ip = Some(from.ip());
            let filter = dns.filter_for(client_ip);
            t[3] = Instant::now();
            let now = dns.now();
            t[4] = Instant::now();
            let mut resolver = dns.resolver();
            t[5] = Instant::now();
            let lane = dns
                .lane
                .get()
                .map(|lane| (lane, resolver.stats().total().queries));
            t[6] = Instant::now();
            let (action, _hit) = resolver.handle_query_filtered(&buf[..len], Via::Udp, now, filter);
            t[7] = Instant::now();
            if let Some((lane, before)) = lane {
                lane.offer_latest(&resolver, before, now, client_ip);
            }
            t[8] = Instant::now();
            drop(resolver);
            t[9] = Instant::now();
            let reply = match action {
                Action::Reply(reply) | Action::ReplyAndRefresh(reply, _) => reply,
                other => panic!("not answered at once: {other:?}"),
            };
            server.send_to(&reply, from).unwrap();
            t[10] = Instant::now();
            dns.note_answered(from.ip());
            t[11] = Instant::now();
            dns.observe(from.ip(), &reply);
            t[12] = Instant::now();
            for (k, section) in total.iter_mut().enumerate() {
                *section += t[k + 1].duration_since(t[k]).as_nanos();
            }
            answers += 1;
        }
        for _ in 0..batch {
            client.recv(&mut reply_buf).unwrap();
        }
    }
    let mut out = [0.0; 12];
    for (k, section) in total.iter().enumerate() {
        out[k] = (*section as f64 / answers as f64 - clock).max(0.0);
    }
    out
}

/// The nanoseconds a call of `work` takes, over `calls`.
fn per_call(calls: usize, mut work: impl FnMut(usize)) -> f64 {
    let started = Instant::now();
    for i in 0..calls {
        work(i);
    }
    started.elapsed().as_nanos() as f64 / calls as f64
}

#[test]
#[ignore = "a timing, for release builds; see the module's text"]
fn hostbench_where_the_time_of_one_answer_goes() {
    let stop = Arc::new(AtomicBool::new(false));
    let up = upstream(&stop);
    let (dns, events) = board_dns(up, &stop);
    // The analysis is not timed here: its share of the runtime thread is the listener's test's.
    thread::spawn(move || while events.recv().is_ok() {});
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ask = |name: &str, id: u16| query(id, name, TYPE_A).unwrap();
    // The cached names are asked once, through the upstream, so that they are in the cache.
    let cached_names = ["example.com", "example.org", "example.net", "iana.org"];
    for name in cached_names {
        let reply = runtime.block_on(dns.answer_for(&ask(name, 1), Via::Udp, None));
        assert!(reply.is_some(), "{name}");
    }
    let cached: Vec<Vec<u8>> = (0..1024)
        .map(|i| ask(cached_names[i % cached_names.len()], i as u16))
        .collect();
    let blocked: Vec<Vec<u8>> = (0..1024)
        .map(|i| ask(ASKED_BLOCKED[i % ASKED_BLOCKED.len()], i as u16))
        .collect();
    let (batches, batch) = (4_000, 32);
    // A round of each first, so that every table is warm.
    sections(&dns, &cached, 100, batch);
    sections(&dns, &blocked, 100, batch);
    let rows = [
        ("cached", sections(&dns, &cached, batches, batch)),
        ("blocked", sections(&dns, &blocked, batches, batch)),
    ];
    println!(
        "where one answer's time goes on this computer, ns per answer over {} answers, one \
         clock read taken out of each section:",
        batches * batch
    );
    println!(
        "{:<38}{:>10}{:>8}{:>10}{:>8}",
        "section", "cached", "%", "blocked", "%"
    );
    let sums: Vec<f64> = rows.iter().map(|(_, s)| s.iter().sum()).collect();
    for (k, name) in SECTIONS.iter().enumerate() {
        println!(
            "{name:<38}{:>10.0}{:>7.1}%{:>10.0}{:>7.1}%",
            rows[0].1[k],
            rows[0].1[k] * 100.0 / sums[0],
            rows[1].1[k],
            rows[1].1[k] * 100.0 / sums[1]
        );
    }
    println!(
        "{:<38}{:>10.0}{:>8}{:>10.0}  (so at most {:.0} and {:.0} answers/s on one core)",
        "total",
        sums[0],
        "",
        sums[1],
        1e9 / sums[0],
        1e9 / sums[1]
    );

    // Inside `handle_query_filtered`: the lookup in the big set, and the statistics' record of the
    // query (the counts, the history, the log entry with its copy of the name, and for a block the
    // blocked-name counts), each on its own.
    let set = big_set();
    let names_blocked: Vec<String> = ASKED_BLOCKED.iter().map(|&n| n.to_owned()).collect();
    let names_open: Vec<String> = cached_names.iter().map(|&n| n.to_owned()).collect();
    let calls = 1_000_000;
    let verdict_blocked = per_call(calls, |i| {
        std::hint::black_box(set.verdict(&names_blocked[i % names_blocked.len()]));
    });
    let verdict_open = per_call(calls, |i| {
        std::hint::black_box(set.verdict(&names_open[i % names_open.len()]));
    });
    let now = dns.now();
    let mut stats = Stats::new(
        qb_dns::stats::QUERY_LOG_CAPACITY,
        qb_dns::stats::BLOCKED_NAMES_TRACKED,
    );
    // Full, as on a board that has run a while: each record drops the oldest entry.
    for i in 0..qb_dns::stats::QUERY_LOG_CAPACITY {
        stats.record(&names_open[i % 4], TYPE_A, Outcome::Cached, now);
    }
    let record_cached = per_call(calls, |i| {
        stats.record(&names_open[i % 4], TYPE_A, Outcome::Cached, now);
    });
    let record_blocked = per_call(calls, |i| {
        stats.record(&names_blocked[i % 4], TYPE_A, Outcome::Blocked, now);
    });
    // The whole decision on the resolver, without the listener around it.
    let decide_cached = per_call(calls, |i| {
        std::hint::black_box(
            dns.resolver()
                .handle_query(&cached[i % 1024], Via::Udp, now),
        );
    });
    let decide_blocked = per_call(calls, |i| {
        std::hint::black_box(
            dns.resolver()
                .handle_query(&blocked[i % 1024], Via::Udp, now),
        );
    });
    println!("inside the resolver, ns per call ({calls} calls each):");
    println!(
        "  handle_query (lock included): cached {decide_cached:.0}, blocked {decide_blocked:.0}"
    );
    println!(
        "  BlockSet::verdict on {BLOCKED_NAMES} names: a listed name {verdict_blocked:.0}, an \
         unlisted one {verdict_open:.0}"
    );
    println!(
        "  Stats::record with the log full: a cached query {record_cached:.0}, a blocked one \
         {record_blocked:.0}"
    );
    stop.store(true, Ordering::Relaxed);
}
