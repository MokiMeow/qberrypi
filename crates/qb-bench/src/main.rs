//! `qb-bench`: the load generator as a command, for a DNS server on this machine or another.
//!
//! Built static for the Pi (`cargo build -p qb-bench --release --target
//! aarch64-unknown-linux-musl`), it is copied to Raspberry Pi OS and run there against Pi-hole,
//! AdGuard Home, Unbound or Blocky over the loopback, the same loads that qberrypi runs against
//! itself. `qb-bench --help` lists the options; `docs/bring-up/benchmark.md` says how to use it.

use std::fs;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qb_bench::cpu::Server;
use qb_bench::engine::{Engine, Limits, Transport};
use qb_bench::file::{self, ClassRun, SteadyRate};
use qb_bench::names::Class;
use qb_bench::run::{Bounds, FORWARDED_DEFAULT_MAX, Mode, Settings};

const HELP: &str = "\
qb-bench: how many answers a second a DNS server sustains, and how fast they are

Usage: qb-bench [options]

For each class it checks the names, ramps the rate up until a step fails, and runs a
steady load at 80% of the most a step sustained. A step passes with loss under 0.1%
(SERVFAIL and REFUSED count as lost), p99 under the bound and 95% of the rate sent.

  --server <IPV4>        the server (default 127.0.0.1)
  --port <N>             its port (default 53)
  --tcp                  ask over one pipelined TCP connection instead of UDP
  --class <CLASS>        cached, blocked or forwarded; again for more (default: all three)
  --mode <MODE>          ramp, steady or both (default both)
  --seconds <N>          the longest a ramp, and a steady load, may take (default 20)
  --step-ms <N>          the length of a ramp's step (default 1000)
  --start-qps <N>        the ramp's first rate (default 1000; forwarded 25)
  --max-qps <N>          the ramp's cap (default 200000; forwarded 50)
  --qps <N>              the steady rate (default: 80% of the ramp's most; needed with --mode steady)
  --timeout-ms <N>       a question unanswered this long is lost (default 2000)
  --p99-ms <N>           the p99 bound of a step (default 10; forwarded 1000)
  --max-in-flight <N>    the most questions out at once (default 10000)
  --cached <NAME>        a name for the cached class instead of the built-in eight; again for more
  --blocked <NAME>       a name the server blocks; a `*` label is fresh each time; again for more
  --blocked-list <FILE>  names the server blocks, one a line (hosts lines too; # comments)
  --forwarded <NAME>     a forwarded name or pattern instead of `*.` six real zones; again for more
  --server-pid <PID>     the server's process, whose processor time is then reported
  --json <FILE>          write the figures there
  -h, --help             this text
  -V, --version          the version

Every forwarded question goes to the server's upstream, someone else's server: keep its
rate modest. The figures hold no address.
";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
struct Options {
    server: SocketAddr,
    transport: Transport,
    classes: Vec<Class>,
    mode: String,
    seconds: u64,
    step_ms: u64,
    start_qps: Option<u32>,
    max_qps: Option<u32>,
    qps: Option<u32>,
    timeout_ms: u64,
    p99_ms: Option<u64>,
    max_in_flight: usize,
    cached: Vec<String>,
    blocked: Vec<String>,
    blocked_list: Option<PathBuf>,
    forwarded: Vec<String>,
    server_pid: Option<u32>,
    json: Option<PathBuf>,
}

/// What the command is to do.
#[derive(Debug, PartialEq)]
enum Command {
    Run(Box<Options>),
    Help,
    Version,
}

/// Reads the arguments after the program's name. The error is a sentence for the user.
fn parse(args: &[String]) -> Result<Command, String> {
    let mut options = Options {
        server: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53),
        transport: Transport::Udp,
        classes: Vec::new(),
        mode: "both".to_owned(),
        seconds: 20,
        step_ms: 1000,
        start_qps: None,
        max_qps: None,
        qps: None,
        timeout_ms: 2000,
        p99_ms: None,
        max_in_flight: 10_000,
        cached: Vec::new(),
        blocked: Vec::new(),
        blocked_list: None,
        forwarded: Vec::new(),
        server_pid: None,
        json: None,
    };
    let mut address = Ipv4Addr::LOCALHOST;
    let mut port = 53u16;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let (flag, inline) = match flag.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (flag.as_str(), None),
        };
        let mut value = || -> Result<String, String> {
            inline
                .clone()
                .or_else(|| args.next().cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        fn number<T: std::str::FromStr>(flag: &str, text: String) -> Result<T, String> {
            text.parse()
                .map_err(|_| format!("{flag} takes a whole number, not {text:?}"))
        }
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--server" => {
                let text = value()?;
                address = text
                    .parse()
                    .map_err(|_| format!("--server takes an IPv4 address, not {text:?}"))?;
            }
            "--port" => port = number(flag, value()?)?,
            "--tcp" => options.transport = Transport::Tcp,
            "--class" => {
                let text = value()?;
                let class = Class::from_word(&text).ok_or_else(|| {
                    format!("--class is cached, blocked or forwarded, not {text:?}")
                })?;
                if !options.classes.contains(&class) {
                    options.classes.push(class);
                }
            }
            "--mode" => {
                let text = value()?;
                if !matches!(text.as_str(), "ramp" | "steady" | "both") {
                    return Err(format!("--mode is ramp, steady or both, not {text:?}"));
                }
                options.mode = text;
            }
            "--seconds" => options.seconds = number(flag, value()?)?,
            "--step-ms" => options.step_ms = number(flag, value()?)?,
            "--start-qps" => options.start_qps = Some(number(flag, value()?)?),
            "--max-qps" => options.max_qps = Some(number(flag, value()?)?),
            "--qps" => options.qps = Some(number(flag, value()?)?),
            "--timeout-ms" => options.timeout_ms = number(flag, value()?)?,
            "--p99-ms" => options.p99_ms = Some(number(flag, value()?)?),
            "--max-in-flight" => options.max_in_flight = number(flag, value()?)?,
            "--cached" => options.cached.push(value()?),
            "--blocked" => options.blocked.push(value()?),
            "--blocked-list" => options.blocked_list = Some(PathBuf::from(value()?)),
            "--forwarded" => options.forwarded.push(value()?),
            "--server-pid" => options.server_pid = Some(number(flag, value()?)?),
            "--json" => options.json = Some(PathBuf::from(value()?)),
            other => return Err(format!("{other} is not an option of qb-bench (see --help)")),
        }
    }
    options.server = SocketAddr::new(IpAddr::V4(address), port);
    if options.classes.is_empty() {
        options.classes = Class::ALL.to_vec();
    }
    if options.mode == "steady" && options.qps.is_none() {
        return Err("--mode steady needs --qps".to_owned());
    }
    for (flag, zero) in [
        ("--seconds", options.seconds == 0),
        ("--step-ms", options.step_ms == 0),
        ("--timeout-ms", options.timeout_ms == 0),
        ("--qps", options.qps == Some(0)),
        ("--max-qps", options.max_qps == Some(0)),
        ("--start-qps", options.start_qps == Some(0)),
        ("--max-in-flight", options.max_in_flight == 0),
    ] {
        if zero {
            return Err(format!("{flag} must be at least 1"));
        }
    }
    Ok(Command::Run(Box::new(options)))
}

/// The names of a list file: one a line, the name of a hosts line (`0.0.0.0 name`), `#` and what
/// follows it left out.
fn list_names(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or("").trim();
            let mut words = line.split_whitespace();
            let first = words.next()?;
            match words.next() {
                Some(name) if first.parse::<IpAddr>().is_ok() => Some(name.to_owned()),
                _ => Some(first.to_owned()),
            }
        })
        .collect()
}

/// The settings of `class`'s ramp and steady load under `options`.
fn settings_of(options: &Options, class: Class) -> (Settings, Settings) {
    let server = options.server_pid.map_or(Server::Unknown, Server::Pid);
    let seconds = Duration::from_secs(options.seconds);
    let Mode::Ramp { start, max } = Mode::ramp_of(class) else {
        unreachable!("a class's ramp is a ramp");
    };
    let ramp = Mode::Ramp {
        start: options.start_qps.unwrap_or(start),
        max: options.max_qps.unwrap_or(max),
    };
    let bounds = Bounds::with_p99(
        options
            .p99_ms
            .map_or_else(|| Bounds::p99_of(class), Duration::from_millis),
    );
    let base = |mode| Settings {
        step: Duration::from_millis(options.step_ms),
        bounds,
        ..Settings::new(class, mode, seconds, server)
    };
    (
        base(ramp),
        base(Mode::Steady {
            qps: options.qps.unwrap_or(1),
        }),
    )
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Runs what `options` asks: each class in turn, its lines printed as they come, then the
/// summary. Whether every class was measured.
fn run(options: &Options) -> Result<bool, String> {
    let mut blocked = options.blocked.clone();
    if let Some(path) = &options.blocked_list {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        blocked.extend(list_names(&text));
    }
    let limits = Limits {
        timeout: Duration::from_millis(options.timeout_ms),
        max_in_flight: options.max_in_flight,
    };
    let mut engine = Engine::connect(options.server, options.transport, limits)
        .map_err(|error| format!("cannot open a socket to the server: {error}"))?;
    let mut runs: Vec<ClassRun> = Vec::new();
    let mut out = std::io::stdout();
    for &class in &options.classes {
        let (ramp, steady) = settings_of(options, class);
        let _ = writeln!(out, "qb-bench: {} ...", class.word());
        if class == Class::Forwarded {
            let cap = match (options.mode.as_str(), ramp.mode) {
                ("steady", _) => options.qps.unwrap_or(1),
                (_, Mode::Ramp { max, .. }) => max,
                _ => FORWARDED_DEFAULT_MAX,
            };
            let _ = writeln!(
                out,
                "qb-bench: forwarded: every question goes to the server's upstream, at most {cap} a \
                 second"
            );
        }
        let given = match class {
            Class::Cached => &options.cached,
            Class::Blocked => &blocked,
            Class::Forwarded => &options.forwarded,
        };
        let steady_rate = options.qps.map_or(SteadyRate::FromRamp, SteadyRate::Fixed);
        let (ramp, steady) = match options.mode.as_str() {
            "ramp" => (Some(ramp), None),
            "steady" => (None, Some((steady, steady_rate))),
            _ => (Some(ramp), Some((steady, steady_rate))),
        };
        let done = file::measure(&mut engine, class, given, ramp, steady, &mut |line| {
            let _ = writeln!(out, "{line}");
        });
        if let Some(error) = &done.error {
            let _ = writeln!(out, "qb-bench: {}: {error}", class.word());
        }
        runs.push(done);
    }
    let json = file::file_json("qb-bench", unix_secs(), &runs);
    let _ = write!(out, "{}", file::summary_text(&json));
    if let Some(path) = &options.json {
        let mut text = serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?;
        text.push('\n');
        fs::write(path, text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(runs.iter().all(|run| run.error.is_none()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Command::Help) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("qb-bench {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(options)) => match run(&options) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("qb-bench: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("qb-bench: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|&a| a.to_owned()).collect()
    }

    fn options(list: &[&str]) -> Options {
        match parse(&args(list)).unwrap() {
            Command::Run(options) => *options,
            other => panic!("not a run: {other:?}"),
        }
    }

    #[test]
    fn with_no_options_it_loads_the_local_server_with_every_class() {
        let o = options(&[]);
        assert_eq!(o.server, "127.0.0.1:53".parse().unwrap());
        assert_eq!(o.classes, Class::ALL);
        assert_eq!(
            (o.mode.as_str(), o.seconds, o.timeout_ms),
            ("both", 20, 2000)
        );
        assert_eq!(o.transport, Transport::Udp);
    }

    #[test]
    fn the_options_are_read_in_either_form() {
        let o = options(&[
            "--server",
            "192.0.2.53",
            "--port=5353",
            "--tcp",
            "--class",
            "blocked",
            "--class=cached",
            "--mode",
            "steady",
            "--qps",
            "500",
            "--blocked",
            "*.ads.example",
            "--server-pid",
            "42",
            "--json",
            "out.json",
        ]);
        assert_eq!(o.server, "192.0.2.53:5353".parse().unwrap());
        assert_eq!(o.classes, [Class::Blocked, Class::Cached]);
        assert_eq!((o.qps, o.server_pid), (Some(500), Some(42)));
        assert_eq!(o.blocked, ["*.ads.example"]);
        assert_eq!(o.transport, Transport::Tcp);
        assert_eq!(o.json, Some(PathBuf::from("out.json")));
    }

    #[test]
    fn what_cannot_be_run_is_refused_with_a_sentence() {
        for (bad, says) in [
            (&["--class", "all"][..], "--class is cached"),
            (&["--mode", "steady"], "needs --qps"),
            (&["--server", "example.com"], "IPv4"),
            (&["--seconds", "0"], "at least 1"),
            (&["--qps"], "needs a value"),
            (&["--qps", "many"], "whole number"),
            (&["--nope"], "not an option"),
        ] {
            let error = parse(&args(bad)).unwrap_err();
            assert!(error.contains(says), "{bad:?}: {error}");
        }
        assert_eq!(parse(&args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse(&args(&["-V"])), Ok(Command::Version));
    }

    #[test]
    fn a_list_file_gives_plain_and_hosts_names_without_comments() {
        let text =
            "# a list\nads.example\n0.0.0.0 tracker.example # trailing\n\n  \nonly.example extra\n";
        assert_eq!(
            list_names(text),
            ["ads.example", "tracker.example", "only.example"]
        );
    }

    #[test]
    fn the_forwarded_class_has_its_own_modest_ramp_unless_told() {
        let (ramp, _) = settings_of(&options(&[]), Class::Forwarded);
        assert_eq!(ramp.mode, Mode::Ramp { start: 25, max: 50 });
        assert_eq!(ramp.bounds.p99, Duration::from_secs(1));
        let (ramp, steady) =
            settings_of(&options(&["--max-qps", "50", "--qps", "40"]), Class::Cached);
        assert_eq!(
            ramp.mode,
            Mode::Ramp {
                start: 1000,
                max: 50
            }
        );
        assert_eq!(steady.mode, Mode::Steady { qps: 40 });
        assert_eq!(ramp.bounds.p99, Duration::from_millis(10));
    }
}
