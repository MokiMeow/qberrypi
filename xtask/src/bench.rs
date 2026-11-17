//! `cargo xtask bench`: how many answers a second the Pi sustains, and how fast they are, per
//! class of question (cached, blocked, forwarded), with `qb-bench`'s generator.
//!
//! - `--on-pi` runs the loads on the Pi itself, over its loopback, through the API's
//!   `POST /api/qbp/bench`: the figure of the software and the board, which the Pi's Wi-Fi does not
//!   limit. It logs in as `cargo xtask update` does (the password piped in, TLS pinned to
//!   `qberrypi-tls.pem`), and for each class runs a ramp, then a steady load at 80 per cent of the
//!   most the ramp sustained, each on a connection of its own.
//! - `--from-here` runs the same loads from this computer over the network: the figure that the
//!   Wi-Fi allows, which is not the Pi's.
//! - `--compare A.json B.json` reads two files and prints B against A, as `latency --compare` does.
//!
//! The forwarded class sends every question to the Pi's upstream, Quad9: its ramp stops at
//! `--forwarded-max-qps` (50 by default, 100 at most: the questions really go to Quad9, which
//! throttles a source that sends too many fresh names), and the command says so. The figures go to
//! `private/bench-<where>-<UTC time>.json` unless `--json` names a file; they hold no address. The
//! method and the table to fill are in `docs/bring-up/benchmark.md`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgGroup, Args, ValueEnum};
use qb_bench::cpu::Server;
use qb_bench::engine::{Engine, Limits, Transport};
use qb_bench::file::{self, ClassRun, SteadyRate, steady_rate};
use qb_bench::names::Class;
use qb_bench::run::{Bounds, FORWARDED_DEFAULT_MAX, Mode, Settings};
use qberrypi::api::bench::{FORWARDED_MAX_QPS, MAX_SECONDS, PATH};
use rustls::ClientConfig;
use serde_json::{Value, json};

use crate::card::{PRIVATE_DIR, make_private_dir};
use crate::{hashpw, pinned, probe, slot};

/// The arguments of `cargo xtask bench`.
#[derive(Args, Debug, PartialEq, Eq)]
#[command(group(ArgGroup::new("where").required(true).args(["on_pi", "from_here", "compare"])))]
pub struct BenchArgs {
    /// Run the loads on the Pi, over its own loopback, through its API (the password is piped in)
    #[arg(long)]
    pub on_pi: bool,
    /// Run the loads from this computer over the network: the figure the Wi-Fi allows
    #[arg(long)]
    pub from_here: bool,
    /// Read two files written by this command (or by qb-bench --json) and print B against A
    #[arg(long, num_args = 2, value_names = ["A.json", "B.json"],
          conflicts_with_all = ["server", "class", "seconds", "json", "p99_ms"])]
    pub compare: Option<Vec<PathBuf>>,
    /// The Pi's IPv4 address; without it, net.address of qberrypi.toml
    #[arg(long, value_name = "IPV4")]
    pub server: Option<Ipv4Addr>,
    /// A class to measure; again for more (default: all three)
    #[arg(long, value_enum)]
    pub class: Vec<ClassArg>,
    /// The longest a ramp, and a steady load, may take, in seconds (at most 30 on the Pi)
    #[arg(long, default_value_t = 20)]
    pub seconds: u64,
    /// The forwarded ramp's cap: each forwarded question goes to the Pi's upstream, Quad9
    #[arg(long, default_value_t = FORWARDED_DEFAULT_MAX, value_name = "QPS")]
    pub forwarded_max_qps: u32,
    /// The p99 bound of a cached or blocked step from here (on the Pi it is 10 ms)
    #[arg(long, default_value_t = 50, value_name = "MS")]
    pub p99_ms: u64,
    /// Where the figures go; default private/bench-<where>-<UTC time>.json
    #[arg(long, value_name = "FILE")]
    pub json: Option<PathBuf>,
}

/// A class on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ClassArg {
    Cached,
    Blocked,
    Forwarded,
}

impl ClassArg {
    fn class(self) -> Class {
        match self {
            ClassArg::Cached => Class::Cached,
            ClassArg::Blocked => Class::Blocked,
            ClassArg::Forwarded => Class::Forwarded,
        }
    }
}

/// What a measurement asks: the classes, the seconds and the forwarded cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub classes: Vec<Class>,
    pub seconds: u64,
    pub forwarded_max_qps: u32,
}

impl Plan {
    /// The plan of `args`; the error says what is out of range.
    fn of(args: &BenchArgs, on_pi: bool) -> Result<Plan> {
        if args.seconds == 0 || (on_pi && args.seconds > MAX_SECONDS) {
            bail!("--seconds is 1 to {MAX_SECONDS} on the Pi, and at least 1 from here");
        }
        if args.forwarded_max_qps == 0 || args.forwarded_max_qps > FORWARDED_MAX_QPS {
            bail!("--forwarded-max-qps is 1 to {FORWARDED_MAX_QPS}: each is a question to Quad9");
        }
        let mut classes: Vec<Class> = Vec::new();
        for class in args.class.iter().map(|c| c.class()) {
            if !classes.contains(&class) {
                classes.push(class);
            }
        }
        if classes.is_empty() {
            classes = Class::ALL.to_vec();
        }
        Ok(Plan {
            classes,
            seconds: args.seconds,
            forwarded_max_qps: args.forwarded_max_qps,
        })
    }

    /// The cap of `class`'s ramp in a request to the Pi: the forwarded one's, or none.
    fn ramp_cap(&self, class: Class) -> Option<u32> {
        (class == Class::Forwarded).then_some(self.forwarded_max_qps)
    }
}

/// The note on the forwarded class, said before it runs.
fn forwarded_note(plan: &Plan) -> String {
    format!(
        "bench: forwarded: every question goes to the Pi's upstream (Quad9), at most {} a second",
        plan.forwarded_max_qps
    )
}

/// How long the command waits for one run on the Pi: its seconds, the last step's wait, the
/// checks before it and the connection.
fn run_wait(seconds: u64) -> Duration {
    Duration::from_secs(seconds + 30)
}

/// How long the login may take.
const LOGIN_WAIT: Duration = Duration::from_secs(30);

/// The lines of a run's steps, from its JSON, as the table prints them.
fn steps_text(run: &Value) -> String {
    let mut text = String::new();
    let steps = run.get("steps").and_then(Value::as_array);
    for step in steps.into_iter().flatten() {
        let n = |key: &str| step.get(key).and_then(Value::as_f64);
        let cell = |v: Option<f64>, places: usize| {
            v.map_or_else(|| "-".to_owned(), |v| format!("{v:.places$}"))
        };
        let cpu = |key: &str| {
            step.get("cpu")
                .and_then(|c| c.get(key))
                .and_then(Value::as_f64)
        };
        let busiest = step
            .get("cpu")
            .and_then(|c| c.get("cores_busy_pct"))
            .and_then(Value::as_array)
            .and_then(|cores| cores.iter().filter_map(Value::as_f64).reduce(f64::max));
        let verdict = step.get("miss").and_then(Value::as_str).unwrap_or("pass");
        text.push_str(&format!(
            "{:>9} {:>10} {:>10} {:>7} {:>7} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6} {:>6}  {verdict}\n",
            cell(n("target_qps"), 0),
            cell(n("achieved_qps"), 1),
            cell(n("answered_qps"), 1),
            cell(n("lost"), 0),
            cell(n("failed"), 0),
            cell(n("loss_pct"), 3),
            cell(n("p50_ms"), 3),
            cell(n("p95_ms"), 3),
            cell(n("p99_ms"), 3),
            cell(cpu("generator_pct"), 1),
            cell(cpu("server_pct"), 1),
            cell(busiest, 1),
        ));
    }
    text
}

/// Asks the Pi for one run of `class`: `{"class", "mode", "seconds", "qps"?}`. The run, or why the
/// Pi did not make it.
fn ask_pi(
    session: &pinned::Session,
    class: Class,
    mode: &str,
    seconds: u64,
    qps: Option<u32>,
) -> Result<Value> {
    let mut body = json!({ "class": class.word(), "mode": mode, "seconds": seconds });
    if let Some(qps) = qps {
        body["qps"] = json!(qps);
    }
    let response = session.post_json(PATH, body.to_string().as_bytes(), run_wait(seconds))?;
    match response.status {
        200 => response
            .json()
            .and_then(|json| json.get("bench").cloned())
            .ok_or_else(|| anyhow!("the Pi's answer holds no run")),
        404 => bail!("the Pi's build has no bench route: update it to this build first"),
        status => bail!(
            "the Pi did not run it ({}, HTTP {status})",
            response.refusal()
        ),
    }
}

/// The loads of `plan` on the Pi whose API is at `api`, logged in with `password` over `client`;
/// the lines go to `out` as each run ends. Returns the file of the runs.
pub fn on_pi(
    api: SocketAddr,
    client: Arc<ClientConfig>,
    password: &[u8],
    plan: &Plan,
    unix_secs: u64,
    out: &mut dyn Write,
) -> Result<Value> {
    let session = pinned::Session::login(api, client, password, LOGIN_WAIT)?;
    let mut runs = Vec::new();
    for &class in &plan.classes {
        if class == Class::Forwarded {
            writeln!(out, "{}", forwarded_note(plan))?;
        }
        writeln!(
            out,
            "bench: {}: ramp on the Pi, at most {} s ...",
            class.word(),
            plan.seconds
        )?;
        let mut run = ClassRun {
            class,
            ramp: None,
            steady: None,
            error: None,
        };
        match ask_pi(&session, class, "ramp", plan.seconds, plan.ramp_cap(class)) {
            Ok(ramp) => {
                write!(
                    out,
                    "{}{}",
                    qb_bench::run::step_header() + "\n",
                    steps_text(&ramp)
                )?;
                let max = ramp.get("max_sustained_qps").and_then(Value::as_f64);
                run.ramp = Some(ramp);
                match max {
                    Some(max) => {
                        let qps = steady_rate(max);
                        writeln!(
                            out,
                            "bench: {}: most sustained {max:.1}/s; steady at {qps}/s ...",
                            class.word()
                        )?;
                        match ask_pi(&session, class, "steady", plan.seconds, Some(qps)) {
                            Ok(steady) => {
                                write!(out, "{}", steps_text(&steady))?;
                                run.steady = Some(steady);
                            }
                            Err(error) => run.error = Some(format!("{error:#}")),
                        }
                    }
                    None => {
                        run.error = Some(
                            "no step of the ramp passed, so there is no rate for the steady load"
                                .to_owned(),
                        );
                    }
                }
            }
            Err(error) => run.error = Some(format!("{error:#}")),
        }
        if let Some(error) = &run.error {
            writeln!(out, "bench: {}: {error}", class.word())?;
        }
        runs.push(run);
    }
    session.logout(LOGIN_WAIT);
    Ok(file::file_json("on-pi", unix_secs, &runs))
}

/// The loads of `plan` from this computer against the DNS server at `dns`, with the p99 bound of
/// a cached or blocked step at `p99`. Returns the file of the runs.
pub fn from_here(
    dns: SocketAddr,
    plan: &Plan,
    p99: Duration,
    unix_secs: u64,
    out: &mut dyn Write,
) -> Result<Value> {
    let limits = Limits {
        timeout: Duration::from_secs(2),
        max_in_flight: 10_000,
    };
    let mut engine =
        Engine::connect(dns, Transport::Udp, limits).context("cannot open a socket to the Pi")?;
    let mut runs = Vec::new();
    for &class in &plan.classes {
        if class == Class::Forwarded {
            writeln!(out, "{}", forwarded_note(plan))?;
        }
        writeln!(out, "bench: {}: ramp from here ...", class.word())?;
        let seconds = Duration::from_secs(plan.seconds);
        let ramp_mode = match (class, Mode::ramp_of(class)) {
            (Class::Forwarded, Mode::Ramp { start, .. }) => Mode::Ramp {
                start: start.min(plan.forwarded_max_qps),
                max: plan.forwarded_max_qps,
            },
            (_, mode) => mode,
        };
        let mut ramp = Settings::new(class, ramp_mode, seconds, Server::Unknown);
        if class != Class::Forwarded {
            ramp.bounds = Bounds::with_p99(p99);
        }
        let steady = Settings {
            mode: Mode::Steady { qps: 1 },
            ..ramp
        };
        let mut said = Vec::new();
        let run = file::measure(
            &mut engine,
            class,
            &[],
            Some(ramp),
            Some((steady, SteadyRate::FromRamp)),
            &mut |line| said.push(line.to_owned()),
        );
        for line in said {
            writeln!(out, "{line}")?;
        }
        if let Some(error) = &run.error {
            writeln!(out, "bench: {}: {error}", class.word())?;
        }
        runs.push(run);
    }
    Ok(file::file_json("from-here", unix_secs, &runs))
}

/// Writes `file` to `path`, mode 0600, its folder made private when it is the repository's
/// `private/`.
fn write_file(root: &Path, path: &Path, file: &Value) -> Result<()> {
    let folder = path.parent().unwrap_or(Path::new("."));
    if folder == root.join(PRIVATE_DIR) {
        make_private_dir(folder).with_context(|| format!("cannot make {}", folder.display()))?;
    }
    let mut text = serde_json::to_string_pretty(file)?;
    text.push('\n');
    let mut handle = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    handle
        .write_all(text.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Where the figures go without `--json`: `private/bench-<where>-<UTC time>.json`.
fn default_path(root: &Path, place: &str, unix_secs: u64) -> Result<PathBuf> {
    Ok(root.join(PRIVATE_DIR).join(format!(
        "bench-{place}-{}.json",
        crate::card::stamp(unix_secs)?
    )))
}

/// `cargo xtask bench --compare`: B against A; fails once printed if B is worse on a figure, or
/// lacks one of A's.
pub fn compare(a: &Path, b: &Path, out: &mut dyn Write) -> Result<()> {
    let read = |path: &Path| -> Result<Value> {
        let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a JSON file", path.display()))
    };
    let name = |path: &Path| {
        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    };
    let comparison = file::compare(&name(a), &read(a)?, &name(b), &read(b)?);
    write!(out, "{}", comparison.text)?;
    match comparison.failure() {
        Some(words) => bail!("{words}"),
        None => Ok(()),
    }
}

/// Runs `cargo xtask bench` with `args` in the repository at `root`.
pub fn run(root: &Path, args: &BenchArgs, unix_secs: u64, out: &mut dyn Write) -> Result<()> {
    if let Some([a, b]) = args.compare.as_deref() {
        return compare(a, b, out);
    }
    let address = match args.server {
        Some(address) => address,
        None => probe::configured_address(root, "--server <ipv4>")?,
    };
    let (place, file) = if args.on_pi {
        let plan = Plan::of(args, true)?;
        let password = hashpw::read_password_from_stdin()?;
        let pem = slot::read_tls_pem(root)?;
        let client = pinned::client_config(&pem)?;
        let api = SocketAddr::new(IpAddr::V4(address), qberrypi::api::PORT);
        (
            "on-pi",
            on_pi(api, client, &password, &plan, unix_secs, out)?,
        )
    } else {
        let plan = Plan::of(args, false)?;
        let dns = SocketAddr::new(IpAddr::V4(address), qberrypi::dns::DNS_PORT);
        let p99 = Duration::from_millis(args.p99_ms.max(1));
        ("from-here", from_here(dns, &plan, p99, unix_secs, out)?)
    };
    write!(out, "{}", file::summary_text(&file))?;
    let path = match &args.json {
        Some(path) => path.clone(),
        None => default_path(root, place, unix_secs)?,
    };
    write_file(root, &path, &file)?;
    writeln!(out, "bench: the figures are in {}", path.display())?;
    let stopped = file
        .get("classes")
        .and_then(Value::as_array)
        .map_or(0, |classes| {
            classes.iter().filter(|c| c.get("error").is_some()).count()
        });
    if stopped > 0 {
        bail!("{stopped} classes could not be measured in full");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert;
    use crate::standin::{PASSWORD, SID, TlsPi, error_reply, json_reply};
    use qb_bench::testserver::{Behaviour, TestServer};
    use qberrypi::api::http::Request;
    use std::sync::Mutex;

    fn pem() -> String {
        cert::make("qberrypi", Ipv4Addr::new(192, 0, 2, 53), 1_800_000_000).unwrap()
    }

    fn plan(classes: &[Class]) -> Plan {
        Plan {
            classes: classes.to_vec(),
            seconds: 3,
            forwarded_max_qps: 50,
        }
    }

    /// A run as the Pi answers it: a ramp that sustained `max`, or a steady step at `qps`.
    fn pi_run(body: &Value) -> Value {
        let class = body["class"].as_str().unwrap();
        match body["mode"].as_str().unwrap() {
            "ramp" if class == "blocked" => json!({"class": class, "mode": "ramp",
                "max_sustained_qps": null, "steps": [{"target_qps": 1000, "miss": "loss"}]}),
            "ramp" => json!({"class": class, "mode": "ramp", "transport": "udp",
                "max_sustained_qps": 10_000.0, "at_cap": false,
                "steps": [{"target_qps": 10_000, "answered_qps": 10_000.0, "p99_ms": 1.5,
                           "cpu": {"cores_busy_pct": [99.0, 5.0], "generator_pct": 60.0, "server_pct": 95.0}}]}),
            _ => json!({"class": class, "mode": "steady", "transport": "udp",
                "steps": [{"target_qps": body["qps"], "answered_qps": body["qps"], "p50_ms": 0.2,
                           "p95_ms": 0.4, "p99_ms": 0.9, "loss_pct": 0.0}]}),
        }
    }

    #[test]
    fn on_the_pi_each_class_is_a_ramp_then_a_steady_load_at_eighty_per_cent() {
        let pem = pem();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let pi = {
            let asked = asked.clone();
            TlsPi::serving(
                &pem,
                Box::new(move |request: &Request| {
                    assert_eq!(request.target, PATH);
                    if request.sid.as_deref() != Some(SID) {
                        return error_reply(401, "unauthorized", "Unauthorized");
                    }
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    asked.lock().unwrap().push(body.clone());
                    json_reply(
                        200,
                        &json!({ "bench": pi_run(&body), "took": 1.0 }).to_string(),
                    )
                }),
            )
        };
        let client = pinned::client_config(pem.as_bytes()).unwrap();
        let mut out = Vec::new();
        let file = on_pi(
            pi.addr,
            client,
            PASSWORD,
            &plan(&[Class::Cached, Class::Blocked, Class::Forwarded]),
            1_800_000_000,
            &mut out,
        )
        .unwrap();
        let asked = asked.lock().unwrap().clone();
        let words: Vec<String> = asked
            .iter()
            .map(|b| format!("{} {} {}", b["class"], b["mode"], b["qps"]))
            .collect();
        assert_eq!(
            words,
            [
                r#""cached" "ramp" null"#,
                r#""cached" "steady" 8000"#,
                r#""blocked" "ramp" null"#,
                r#""forwarded" "ramp" 50"#,
                r#""forwarded" "steady" 8000"#,
            ]
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("at most 50 a second"), "{text}");
        assert!(text.contains("most sustained 10000.0/s; steady at 8000/s"));
        assert!(text.contains("blocked: no step of the ramp passed"));
        assert_eq!(file["source"], "on-pi");
        assert_eq!(file["classes"][0]["steady"]["steps"][0]["target_qps"], 8000);
        assert!(file["classes"][1]["error"].is_string());
        // Logged in once, and out at the end.
        let seen = pi.seen();
        assert_eq!(seen.first().map(String::as_str), Some("POST auth"));
        assert_eq!(seen.last().unwrap(), &format!("DELETE auth sid={SID}"));
    }

    #[test]
    fn a_pi_without_the_route_says_to_update_it() {
        let pem = pem();
        let pi = TlsPi::serving(
            &pem,
            Box::new(|_: &Request| error_reply(404, "not_found", "Not found")),
        );
        let client = pinned::client_config(pem.as_bytes()).unwrap();
        let mut out = Vec::new();
        let file = on_pi(
            pi.addr,
            client,
            PASSWORD,
            &plan(&[Class::Cached]),
            1,
            &mut out,
        )
        .unwrap();
        assert!(
            file["classes"][0]["error"]
                .as_str()
                .unwrap()
                .contains("update it")
        );
    }

    #[test]
    fn from_here_measures_each_class_against_the_server_and_writes_a_private_file() {
        let server = TestServer::start(Behaviour::By(Arc::new(|name: &str| {
            use qb_bench::testserver::Reply;
            if name == "doubleclick.net" {
                Reply::Address(Ipv4Addr::UNSPECIFIED)
            } else {
                Reply::Address(Ipv4Addr::new(192, 0, 2, 1))
            }
        })));
        let mut out = Vec::new();
        let plan = Plan {
            seconds: 2,
            ..plan(&[Class::Cached, Class::Blocked])
        };
        let file = from_here(server.addr, &plan, Duration::from_millis(50), 1, &mut out).unwrap();
        assert_eq!(file["source"], "from-here");
        for i in 0..2 {
            assert!(
                file["classes"][i]["ramp"]["max_sustained_qps"].is_number(),
                "{file}"
            );
            assert!(file["classes"][i]["steady"]["steps"][0]["sent"].is_number());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PRIVATE_DIR).join("bench.json");
        fs::create_dir(dir.path().join(PRIVATE_DIR)).unwrap();
        write_file(dir.path(), &path, &file).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(dir.path().join(PRIVATE_DIR))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let again: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(again, file);
    }

    #[test]
    fn the_plan_holds_the_pi_to_its_limits() {
        let args = |extra: &[&str]| {
            use clap::Parser;
            #[derive(Parser)]
            struct Cli {
                #[command(flatten)]
                bench: BenchArgs,
            }
            Cli::try_parse_from(std::iter::once("bench").chain(extra.iter().copied()))
                .map(|cli| cli.bench)
        };
        let on = args(&["--on-pi"]).unwrap();
        assert_eq!(Plan::of(&on, true).unwrap().classes, Class::ALL);
        assert!(Plan::of(&args(&["--on-pi", "--seconds", "31"]).unwrap(), true).is_err());
        assert!(Plan::of(&args(&["--from-here", "--seconds", "60"]).unwrap(), false).is_ok());
        assert!(
            Plan::of(
                &args(&["--on-pi", "--forwarded-max-qps", "101"]).unwrap(),
                true
            )
            .is_err()
        );
        let two = args(&["--from-here", "--class", "blocked", "--class", "blocked"]).unwrap();
        assert_eq!(Plan::of(&two, false).unwrap().classes, [Class::Blocked]);
        // One of the three, and only one.
        assert!(args(&[]).is_err());
        assert!(args(&["--on-pi", "--from-here"]).is_err());
        assert!(args(&["--compare", "a.json"]).is_err());
        assert!(args(&["--compare", "a.json", "b.json", "--seconds", "5"]).is_err());
        assert!(args(&["--compare", "a.json", "b.json"]).is_ok());
    }

    #[test]
    fn compare_fails_when_b_is_worse() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, max: f64| {
            let file = file::file_json(
                "on-pi",
                1,
                &[ClassRun {
                    class: Class::Cached,
                    ramp: Some(json!({"max_sustained_qps": max})),
                    steady: Some(
                        json!({"steps": [{"target_qps": 800, "p50_ms": 0.1, "p95_ms": 0.2, "p99_ms": 0.3}]}),
                    ),
                    error: None,
                }],
            );
            let path = dir.path().join(name);
            fs::write(&path, file.to_string()).unwrap();
            path
        };
        let (a, b) = (write("a.json", 1000.0), write("b.json", 900.0));
        let mut out = Vec::new();
        assert!(compare(&a, &a, &mut out).is_ok());
        let error = compare(&a, &b, &mut out).unwrap_err();
        assert!(error.to_string().contains("worse in B"));
    }
}
