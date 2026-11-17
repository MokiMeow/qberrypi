//! The self-test of the DNS and status servers, for the boot in QEMU.
//!
//! The guest there has no network card, so nothing outside can ask the servers anything. With
//! the kernel argument [`DNS_ARG`] init asks them itself, over loopback and through the sockets
//! as an outside client would, and logs what came back. A boot that goes well shows that the
//! listeners are up on the real kernel, that the block set blocks, that a plain name is
//! NXDOMAIN, that a query that cannot be forwarded is answered SERVFAIL, and that the status
//! server answers. The marker that every snapshot holds is asked for too: it is blocked only if
//! the snapshot in the initramfs was loaded before the service listened.
//!
//! The API's step is the panel OS's (spec 9): it logs in over loopback, as qnet does, and asks
//! `GET /api/qbp/devices`, which must answer with a `rev`. The self-test's last line says this
//! process's resident memory.
//!
//! The names asked for are fixed and made up for this, so the card's log holds none of the
//! household's.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream as StdTcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError, SignatureScheme,
    StreamOwned, version,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::{sleep, timeout};

use crate::dns::client::{Answer, query, read_answer};
use crate::dns::upstream::UDP_BUFFER;
use crate::lists::snapshot::MARKER;
use crate::services::blocking;

/// The kernel argument that has init run the self-test.
pub const DNS_ARG: &str = "qberrypi.selftest.dns";
/// The name that the scenario's configuration denies, which the self-test asks for.
pub const DENIED: &str = "qberrypi-selftest.invalid";
/// How long each question is given: an answer that has not come by then is `no answer`.
pub const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// The kernel's file of the flags of `lo`: hexadecimal, `0x9` when it is up.
pub const LO_FLAGS: &str = "/sys/class/net/lo/flags";
/// How long to wait for `lo` to come up.
pub const LO_WAIT: Duration = Duration::from_secs(60);

/// How often the flags of `lo` are read while waiting for them.
const LO_POLL: Duration = Duration::from_millis(200);
/// What is said where there is no answer to a question.
const NO_ANSWER: &str = "no answer";
/// A name of one label, which the resolver answers NXDOMAIN without asking an upstream.
const PLAIN_NAME: &str = "printer";
/// A name that only an upstream can answer, which in the QEMU guest cannot be reached.
const OUTSIDE_NAME: &str = "example.com";
/// The path that the status server is asked for.
const STATS_PATH: &str = "/api/v1/stats";
/// The type number of an A record.
const TYPE_A: u16 = 1;
/// The ID of every query: each goes out from a socket or a connection of its own.
const QUERY_ID: u16 = 0x5e1f;
/// The longest start of the status server's answer that is read: its first line is shorter.
const STATUS_HEAD: usize = 64;

/// Whether the flags of an interface, as the kernel writes them, have the bit 0 set, which is
/// "up". The file holds hexadecimal with the prefix `0x`; anything else is not up.
pub fn is_up(flags: &str) -> bool {
    flags
        .trim()
        .strip_prefix("0x")
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .is_some_and(|flags| flags & 1 != 0)
}

/// Waits until the file `flags` says that the interface is up, reading it every 200 ms, for at
/// most `wait`; whether it is up. A file that is not there yet is read again.
///
/// The read is of a file in `/sys`, which the kernel answers from memory, so it does not hold
/// the runtime thread.
pub async fn lo_comes_up(flags: &Path, wait: Duration) -> bool {
    let up = async {
        while !std::fs::read_to_string(flags).is_ok_and(|text| is_up(&text)) {
            sleep(LO_POLL).await;
        }
    };
    timeout(wait, up).await.is_ok()
}

/// Asks the DNS service and the status server as an outside client would, and says what came
/// back, one line for each question, in this order:
///
/// ```text
/// selftest: dns: udp qberrypi-selftest.invalid A: NOERROR 0.0.0.0
/// selftest: dns: tcp qberrypi-selftest.invalid A: NOERROR 0.0.0.0
/// selftest: dns: udp printer A: NXDOMAIN
/// selftest: dns: udp qberrypi-snapshot.invalid A: NOERROR 0.0.0.0
/// selftest: dns: udp example.com A: SERVFAIL
/// selftest: status: /api/v1/stats: 200
/// ```
///
/// A question that has no answer within [`ANSWER_WAIT`] has `no answer` in the place of one.
pub async fn dns(dns_server: SocketAddr, status_server: SocketAddr) -> Vec<String> {
    dns_within(dns_server, status_server, ANSWER_WAIT).await
}

/// [`dns`], with each question given `wait` instead of [`ANSWER_WAIT`].
pub(crate) async fn dns_within(
    dns_server: SocketAddr,
    status_server: SocketAddr,
    wait: Duration,
) -> Vec<String> {
    let summary =
        |answer: Option<Answer>| answer.map_or_else(|| NO_ANSWER.to_owned(), |a| a.summary());
    let denied_udp = udp(dns_server, DENIED, wait).await;
    let denied_tcp = tcp(dns_server, DENIED, wait).await;
    let plain = udp(dns_server, PLAIN_NAME, wait).await;
    let marker = udp(dns_server, MARKER, wait).await;
    let outside = udp(dns_server, OUTSIDE_NAME, wait).await;
    let code = status_code(status_server, wait)
        .await
        .map_or_else(|| NO_ANSWER.to_owned(), |code| code.to_string());
    vec![
        format!("selftest: dns: udp {DENIED} A: {}", summary(denied_udp)),
        format!("selftest: dns: tcp {DENIED} A: {}", summary(denied_tcp)),
        format!("selftest: dns: udp {PLAIN_NAME} A: {}", summary(plain)),
        format!("selftest: dns: udp {MARKER} A: {}", summary(marker)),
        format!("selftest: dns: udp {OUTSIDE_NAME} A: {}", summary(outside)),
        format!("selftest: status: {STATS_PATH}: {code}"),
    ]
}

/// What `server` answers over UDP to the question `name` A within `wait`. Datagrams that are not
/// the answer to this question are not taken for it; an error ends the wait.
async fn udp(server: SocketAddr, name: &str, wait: Duration) -> Option<Answer> {
    let packet = query(QUERY_ID, name, TYPE_A).ok()?;
    let exchange = async {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.ok()?;
        socket.connect(server).await.ok()?;
        socket.send(&packet).await.ok()?;
        let mut buf = vec![0u8; UDP_BUFFER];
        loop {
            let len = socket.recv(&mut buf).await.ok()?;
            if let Some(answer) = read_answer(&buf[..len], QUERY_ID) {
                return Some(answer);
            }
        }
    };
    timeout(wait, exchange).await.ok().flatten()
}

/// What `server` answers over TCP to the question `name` A within `wait`.
async fn tcp(server: SocketAddr, name: &str, wait: Duration) -> Option<Answer> {
    let packet = query(QUERY_ID, name, TYPE_A).ok()?;
    let len = u16::try_from(packet.len()).ok()?;
    let exchange = async {
        let mut stream = TcpStream::connect(server).await.ok()?;
        stream
            .write_all(&[&len.to_be_bytes()[..], &packet[..]].concat())
            .await
            .ok()?;
        let mut head = [0u8; 2];
        stream.read_exact(&mut head).await.ok()?;
        let mut reply = vec![0u8; usize::from(u16::from_be_bytes(head))];
        stream.read_exact(&mut reply).await.ok()?;
        read_answer(&reply, QUERY_ID)
    };
    timeout(wait, exchange).await.ok().flatten()
}

/// The status code that `server` answers to a request for the statistics within `wait`.
async fn status_code(server: SocketAddr, wait: Duration) -> Option<u16> {
    let request =
        format!("GET {STATS_PATH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let exchange = async {
        let mut stream = TcpStream::connect(server).await.ok()?;
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut head = [0u8; STATUS_HEAD];
        let mut filled = 0;
        // Up to the end of the first line, which holds the code.
        while filled < head.len() && !head[..filled].contains(&b'\n') {
            let read = stream.read(&mut head[filled..]).await.ok()?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Some(String::from_utf8_lossy(&head[..filled]).into_owned())
    };
    status_code_of(&timeout(wait, exchange).await.ok()??)
}

/// The code in the first line of an HTTP response, `HTTP/1.1 200 OK` for instance.
fn status_code_of(head: &str) -> Option<u16> {
    let mut words = head.lines().next()?.split(' ');
    words
        .next()
        .filter(|version| version.starts_with("HTTP/"))?;
    words.next()?.parse().ok()
}

// ---------------------------------------------------------------------------------------------
// The API step (panel OS, P1)

/// The password of the API in the scenario that runs the self-test: no secret, the card of a QEMU
/// run is a throwaway and `cargo xtask qemu-test` makes the hash of this word for it.
pub const API_PASSWORD: &str = "qemu";
/// How long the API step is given in all, from the first try to connect to the last byte: the API
/// binds only after its TLS configuration is made, which in QEMU takes a moment.
pub const API_WAIT: Duration = Duration::from_secs(20);

/// The path of the login.
const LOGIN_PATH: &str = "/api/auth";
/// The path that the API step asks for after the login: the device inventory, which says its
/// revision (`rev`).
const DEVICES_PATH: &str = "/api/qbp/devices";
/// How long a connection to the API may take to be made, once tried.
const API_CONNECT: Duration = Duration::from_secs(2);
/// How long the step waits before it tries to connect again.
const API_RETRY: Duration = Duration::from_millis(200);
/// The longest start of a reply that is taken.
const API_MAX_HEAD: usize = 8 * 1024;
/// The longest body of a reply that is taken.
const API_MAX_BODY: usize = 1024 * 1024;
/// The name asked of the certificate: the pin does not look at it.
const API_NAME: &str = "localhost";

/// Accepts the certificate whose SHA-256 is the pin, whatever the names and dates in it say, and
/// checks the handshake's signatures with the provider's own algorithms: qnet's trust in the board's
/// certificate, for the self-test that reads it from the same card file the API does.
#[derive(Debug)]
struct Pin {
    sha256: [u8; 32],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if Sha256::digest(end_entity).as_slice() == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General("not the pinned certificate".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A TLS client, on the board's provider, that trusts the certificate with this pin and nothing else.
fn pinned_client(pin: [u8; 32]) -> Option<Arc<ClientConfig>> {
    let provider = Arc::new(crate::https::provider());
    let verifier = Pin {
        sha256: pin,
        algorithms: provider.signature_verification_algorithms,
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&version::TLS13, &version::TLS12])
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Some(Arc::new(config))
}

/// Logs in to the API at `server` with `password` and asks it for the devices, as qnet does, and
/// says what came back, as the one line
///
/// ```text
/// selftest: api: /api/qbp/devices: 200 rev
/// ```
///
/// The certificate is the first of `pem` (the card's `tls.pem`, which the API serves), taken by its
/// pin. Where the line ends `200 rev` the devices came with a revision; `200 no rev` is a reply
/// without one, `login <code>` a login that was refused, any other number the code of the devices'
/// reply, `no certificate` a card with no `tls.pem` and `no answer` a step that could not be done
/// within `wait`. The API may not have bound yet, so the connection is tried again until `wait` is
/// over. The line names no session, password or byte of the reply.
///
/// The whole exchange is blocking work of the pool, the handshake included.
pub async fn api(server: SocketAddr, pem: Option<&[u8]>, password: &str, wait: Duration) -> String {
    let outcome = match pem.and_then(crate::api::tls::leaf_sha256) {
        None => "no certificate".to_owned(),
        Some(pin) => {
            let password = password.to_owned();
            let deadline = Instant::now() + wait;
            blocking("selftest api", move || {
                api_exchange(server, pin, &password, deadline)
            })
            .await
            .unwrap_or_else(|_| NO_ANSWER.to_owned())
        }
    };
    format!("selftest: api: {DEVICES_PATH}: {outcome}")
}

/// [`api`]'s outcome, the words after the path.
fn api_exchange(server: SocketAddr, pin: [u8; 32], password: &str, deadline: Instant) -> String {
    api_outcome(server, pin, password, deadline).unwrap_or_else(str::to_owned)
}

/// The steps of the exchange: connect, handshake, login, devices. An `Err` is the words that say
/// where it ended.
fn api_outcome(
    server: SocketAddr,
    pin: [u8; 32],
    password: &str,
    deadline: Instant,
) -> Result<String, &'static str> {
    let config = pinned_client(pin).ok_or(NO_ANSWER)?;
    let tcp = connect(server, deadline)?;
    limit(&tcp, deadline)?;
    let name = ServerName::try_from(API_NAME).map_err(|_| NO_ANSWER)?;
    let conn = ClientConnection::new(config, name).map_err(|_| NO_ANSWER)?;
    let mut stream = StreamOwned::new(conn, tcp);
    while stream.conn.is_handshaking() {
        limit(&stream.sock, deadline)?;
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|_| NO_ANSWER)?;
    }

    let login = json!({ "password": password }).to_string();
    let (code, body) = exchange(
        &mut stream,
        deadline,
        &request_text("POST", LOGIN_PATH, None, Some(&login), false),
    )?;
    if code != 200 {
        return Ok(format!("login {code}"));
    }
    let sid = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|reply| reply["session"]["sid"].as_str().map(str::to_owned))
        .ok_or("login without a session")?;

    let (code, body) = exchange(
        &mut stream,
        deadline,
        &request_text("GET", DEVICES_PATH, Some(&sid), None, true),
    )?;
    let has_rev = serde_json::from_slice::<Value>(&body).is_ok_and(|reply| reply["rev"].is_u64());
    // Say goodbye as a client does; whether the server has it by now is of no matter.
    stream.conn.send_close_notify();
    let _ = stream.flush();
    Ok(match (code, has_rev) {
        (200, true) => "200 rev".to_owned(),
        (200, false) => "200 no rev".to_owned(),
        (code, _) => code.to_string(),
    })
}

/// A connection to `server`, tried again every [`API_RETRY`] until `deadline`.
fn connect(server: SocketAddr, deadline: Instant) -> Result<StdTcpStream, &'static str> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(NO_ANSWER);
        }
        if let Ok(tcp) = StdTcpStream::connect_timeout(&server, left.min(API_CONNECT)) {
            return Ok(tcp);
        }
        std::thread::sleep(API_RETRY.min(left));
    }
}

/// Gives every read and write on `tcp` what is left to `deadline`, and no more.
fn limit(tcp: &StdTcpStream, deadline: Instant) -> Result<(), &'static str> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(NO_ANSWER);
    }
    tcp.set_read_timeout(Some(left))
        .and_then(|()| tcp.set_write_timeout(Some(left)))
        .map_err(|_| NO_ANSWER)
}

/// The text of a request: HTTP/1.1, JSON, the session in `X-FTL-SID` when there is one, and the
/// connection kept unless this is the last request.
fn request_text(
    method: &str,
    path: &str,
    sid: Option<&str>,
    body: Option<&str>,
    last: bool,
) -> String {
    let mut text = format!(
        "{method} {path} HTTP/1.1\r\nHost: {API_NAME}\r\nUser-Agent: selftest\r\nAccept: application/json\r\n"
    );
    if let Some(sid) = sid {
        text.push_str(&format!("X-FTL-SID: {sid}\r\n"));
    }
    if let Some(body) = body {
        text.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    text.push_str(if last {
        "Connection: close\r\n\r\n"
    } else {
        "Connection: keep-alive\r\n\r\n"
    });
    text.push_str(body.unwrap_or(""));
    text
}

/// What one request on `stream` is answered with: the status code and the body, as long as the
/// reply's `Content-Length` says, which every reply of the API has.
fn exchange(
    stream: &mut StreamOwned<ClientConnection, StdTcpStream>,
    deadline: Instant,
    request: &str,
) -> Result<(u16, Vec<u8>), &'static str> {
    limit(&stream.sock, deadline)?;
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|_| NO_ANSWER)?;
    let mut buf = Vec::new();
    let (code, head, length) = loop {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut reply = httparse::Response::new(&mut headers);
        match reply.parse(&buf) {
            Ok(httparse::Status::Complete(head)) => {
                let length = reply
                    .headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                    .and_then(|header| std::str::from_utf8(header.value).ok()?.trim().parse().ok())
                    .unwrap_or(0_usize);
                break (reply.code.ok_or(NO_ANSWER)?, head, length);
            }
            Ok(httparse::Status::Partial) if buf.len() < API_MAX_HEAD => {}
            _ => return Err(NO_ANSWER),
        }
        read_more(stream, deadline, &mut buf)?;
    };
    if length > API_MAX_BODY {
        return Err(NO_ANSWER);
    }
    while buf.len() < head + length {
        read_more(stream, deadline, &mut buf)?;
    }
    Ok((code, buf[head..head + length].to_vec()))
}

/// Reads what the stream has next onto the end of `buf`; the end of the stream is an error, as the
/// reply is not whole.
fn read_more(
    stream: &mut StreamOwned<ClientConnection, StdTcpStream>,
    deadline: Instant,
    buf: &mut Vec<u8>,
) -> Result<(), &'static str> {
    limit(&stream.sock, deadline)?;
    let mut chunk = [0u8; 2048];
    match stream.read(&mut chunk) {
        Ok(0) | Err(_) => Err(NO_ANSWER),
        Ok(read) => {
            buf.extend_from_slice(&chunk[..read]);
            Ok(())
        }
    }
}

/// The line that ends the self-test: this process's resident memory in kB, as `/proc/self/status`
/// says, for the memory check of the panel's milestones (the QEMU guest has no panel, so the
/// painters' sizes are measured by a unit test, and this is the rest). `unknown` where it cannot
/// be read.
pub fn vmrss_line() -> String {
    match crate::usage::read_rss_kib() {
        Some(kib) => format!("selftest: vmrss: {kib} kB"),
        None => "selftest: vmrss: unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use tokio::net::TcpListener;
    use tokio::sync::watch;

    use crate::dns::Dns;
    use crate::dns::testutil::{FakeUpstream, FixedOffset, Reply, closed_port, listen, ms};
    use crate::dns::upstream::Upstreams;
    use crate::lists::{self, ListState, Lists, Source};
    use crate::logbuf::LogBuf;
    use crate::status::Status;

    /// A DNS service that denies [`DENIED`] and whose only upstream is a port where nothing
    /// listens, and a status server over it; both on `127.0.0.1` ports, with room for two clients
    /// that may take 200 ms each. Returns the addresses of the DNS service and of the server.
    async fn service() -> (SocketAddr, SocketAddr) {
        let dns = Dns::new(
            lists::from_config(&[DENIED.to_owned(), MARKER.to_owned()], &[]).build(),
            Upstreams::with(vec![closed_port()], ms(300), ms(600)),
            Box::new(FixedOffset(0)),
        );
        let dns_addr = listen(dns.clone()).await;
        let log: &'static Mutex<LogBuf> = Box::leak(Box::new(Mutex::new(LogBuf::new(4096))));
        let (clock_set, synced) = watch::channel(false);
        // A receiver whose sender is gone says the same, but the real one is never dropped.
        std::mem::forget(clock_set);
        let lists = Lists::new(ListState {
            source: Source::Config,
            names: 1,
            fetched_unix_secs: None,
            last_error: None,
        });
        let status = Status {
            dns,
            lists,
            log,
            synced,
            started: Instant::now(),
            link: Arc::new(crate::live::WifiStatus::default),
            board_c: Arc::new(|| None),
            unknown_devices: Arc::new(|| 0),
            update: crate::update::UpdateCtl::off(),
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let status_addr = listener.local_addr().unwrap();
        tokio::spawn(status.serve_with(listener, 2, ms(200)));
        (dns_addr, status_addr)
    }

    #[tokio::test]
    async fn the_self_test_reports_what_an_outside_client_would_see() {
        let (dns_addr, status_addr) = service().await;
        assert_eq!(
            dns(dns_addr, status_addr).await,
            [
                "selftest: dns: udp qberrypi-selftest.invalid A: NOERROR 0.0.0.0",
                "selftest: dns: tcp qberrypi-selftest.invalid A: NOERROR 0.0.0.0",
                "selftest: dns: udp printer A: NXDOMAIN",
                "selftest: dns: udp qberrypi-snapshot.invalid A: NOERROR 0.0.0.0",
                "selftest: dns: udp example.com A: SERVFAIL",
                "selftest: status: /api/v1/stats: 200",
            ]
        );
    }

    #[test]
    fn lo_is_up_when_bit_0_of_its_flags_is_set() {
        assert!(is_up("0x9\n") && is_up("0x1") && !is_up("0x8\n") && !is_up("") && !is_up("up"));
    }

    #[tokio::test]
    async fn servers_that_are_not_there_are_reported_and_do_not_hang_it() {
        let lines = dns_within(closed_port(), closed_port(), ms(200)).await;
        assert_eq!(lines.len(), 6);
        assert!(
            lines.iter().all(|line| line.ends_with(": no answer")),
            "{lines:?}"
        );
    }

    #[tokio::test]
    async fn servers_that_say_nothing_are_given_their_wait_and_no_more() {
        // Takes the queries and sends nothing back, over UDP and over TCP.
        let silent = FakeUpstream::start(Reply::Silent).await;
        // A listener that never accepts: the connection is made, and nothing is said on it.
        let quiet = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();

        let started = Instant::now();
        let lines = dns_within(silent.addr, quiet.local_addr().unwrap(), ms(200)).await;

        assert!(
            lines.iter().all(|line| line.ends_with(": no answer")),
            "{lines:?}"
        );
        // Six questions, 200 ms each.
        let took = started.elapsed();
        assert!(took >= ms(1000) && took < ms(4000), "{took:?}");
    }

    #[tokio::test]
    async fn an_api_that_is_not_there_is_given_the_wait_and_no_more() {
        let pem = crate::api::testutil::pem();
        let started = Instant::now();
        let line = api(closed_port(), Some(pem.as_bytes()), "right", ms(700)).await;

        assert_eq!(line, "selftest: api: /api/qbp/devices: no answer");
        let took = started.elapsed();
        assert!(took >= ms(600) && took < ms(4000), "{took:?}");
    }

    #[tokio::test]
    async fn an_api_that_says_nothing_after_the_connection_is_given_the_wait_and_no_more() {
        // Takes the connection and never answers the handshake.
        let quiet = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let pem = crate::api::testutil::pem();

        let started = Instant::now();
        let line = api(
            quiet.local_addr().unwrap(),
            Some(pem.as_bytes()),
            "right",
            ms(600),
        )
        .await;

        assert_eq!(line, "selftest: api: /api/qbp/devices: no answer");
        let took = started.elapsed();
        assert!(took >= ms(500) && took < ms(4000), "{took:?}");
    }

    #[test]
    fn the_requests_name_the_session_and_the_body_and_keep_the_connection_but_the_last() {
        let login = request_text(
            "POST",
            "/api/auth",
            None,
            Some("{\"password\":\"x\"}"),
            false,
        );
        assert!(login.starts_with("POST /api/auth HTTP/1.1\r\n"), "{login}");
        assert!(login.contains("Content-Length: 16\r\n") && !login.contains("X-FTL-SID"));
        assert!(login.ends_with("Connection: keep-alive\r\n\r\n{\"password\":\"x\"}"));

        let devices = request_text("GET", "/api/qbp/devices", Some("abc"), None, true);
        assert!(devices.contains("X-FTL-SID: abc\r\n") && !devices.contains("Content-"));
        assert!(devices.ends_with("Connection: close\r\n\r\n"), "{devices}");
    }

    #[test]
    fn the_last_line_is_this_processs_resident_memory_in_kb_and_nothing_else() {
        let line = vmrss_line();
        let kb = line
            .strip_prefix("selftest: vmrss: ")
            .and_then(|rest| rest.strip_suffix(" kB"))
            .unwrap_or_else(|| panic!("{line}"));
        assert!(kb.parse::<u64>().is_ok_and(|kb| kb > 0), "{line}");
    }

    #[test]
    fn the_status_code_is_the_second_word_of_the_first_line() {
        assert_eq!(
            status_code_of("HTTP/1.1 200 OK\r\nServer: x\r\n"),
            Some(200)
        );
        assert_eq!(status_code_of("HTTP/1.0 404 not found"), Some(404));
        assert_eq!(status_code_of("200 OK\r\n"), None);
        assert_eq!(status_code_of("HTTP/1.1 ok\r\n"), None);
        assert_eq!(status_code_of(""), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_for_lo_ends_when_it_is_up_or_when_the_time_is_out() {
        let dir = tempfile::tempdir().unwrap();
        let flags = dir.path().join("flags");

        // The file is not there yet, and then holds what a down interface holds.
        assert!(!lo_comes_up(&flags, Duration::from_secs(1)).await);
        std::fs::write(&flags, "0x8\n").unwrap();
        assert!(!lo_comes_up(&flags, Duration::from_secs(1)).await);

        // It comes up half a second in.
        let writer = {
            let flags = flags.clone();
            tokio::spawn(async move {
                sleep(Duration::from_millis(500)).await;
                std::fs::write(&flags, "0x9\n").unwrap();
            })
        };
        let started = tokio::time::Instant::now();
        assert!(lo_comes_up(&flags, LO_WAIT).await);
        let took = started.elapsed();
        assert!(took >= ms(500) && took <= ms(500) + LO_POLL, "{took:?}");
        writer.await.unwrap();

        // And once it is, no time is spent.
        let started = tokio::time::Instant::now();
        assert!(lo_comes_up(&flags, LO_WAIT).await);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
}
