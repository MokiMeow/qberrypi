//! A DNS server on the loopback for tests, of this crate and of the ones that use it: UDP and TCP
//! on one port of `127.0.0.1`, each question answered as a closure of the name says. It never
//! asks anyone else, so a test that loads it sends nothing off this computer.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::wire::{HEADER_LEN, RCODE_NXDOMAIN, RCODE_SERVFAIL};

/// How often the server's threads look at whether it is to stop.
const TICK: Duration = Duration::from_millis(10);

/// What the server answers a question with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// NOERROR with one A record.
    Address(Ipv4Addr),
    NxDomain,
    Servfail,
    /// Nothing.
    Silent,
}

/// How the server answers.
#[derive(Clone)]
pub enum Behaviour {
    /// `192.0.2.1` for every name.
    Answer,
    /// `0.0.0.0` for every name.
    Blocked,
    /// SERVFAIL for every name.
    Servfail,
    /// No answer to anything.
    Silent,
    /// What the closure says for the name asked, in lower case and without its trailing dot.
    By(Arc<dyn Fn(&str) -> Reply + Send + Sync>),
}

impl Behaviour {
    fn reply(&self, name: &str) -> Reply {
        match self {
            Behaviour::Answer => Reply::Address(Ipv4Addr::new(192, 0, 2, 1)),
            Behaviour::Blocked => Reply::Address(Ipv4Addr::UNSPECIFIED),
            Behaviour::Servfail => Reply::Servfail,
            Behaviour::Silent => Reply::Silent,
            Behaviour::By(by) => by(name),
        }
    }
}

/// The name of the question in `query`, in lower case, and where the question ends.
fn question(query: &[u8]) -> Option<(String, usize)> {
    let mut at = HEADER_LEN;
    let mut labels = Vec::new();
    loop {
        let len = usize::from(*query.get(at)?);
        if len == 0 {
            break;
        }
        let label = query.get(at + 1..at + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        at += 1 + len;
    }
    // The root's 0, the type and the class.
    let end = at + 5;
    (end <= query.len()).then(|| (labels.join("."), end))
}

/// The answer to `query` as `behaviour` has it, or `None` for none.
fn answer(query: &[u8], behaviour: &Behaviour) -> Option<Vec<u8>> {
    let (name, end) = question(query)?;
    let (code, addr) = match behaviour.reply(&name) {
        Reply::Silent => return None,
        Reply::Address(addr) => (0, Some(addr)),
        Reply::NxDomain => (RCODE_NXDOMAIN, None),
        Reply::Servfail => (RCODE_SERVFAIL, None),
    };
    let mut out = query[..end].to_vec();
    // QR and RA; one question; the answer, if there is one.
    out[2] |= 0x80;
    out[3] = 0x80 | code;
    out[4..12].copy_from_slice(&[0, 1, 0, u8::from(addr.is_some()), 0, 0, 0, 0]);
    if let Some(addr) = addr {
        out.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
        out.extend_from_slice(&addr.octets());
    }
    Some(out)
}

/// A running server; it stops when dropped.
pub struct TestServer {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    questions: Arc<AtomicU64>,
    threads: Vec<JoinHandle<()>>,
}

impl TestServer {
    /// Starts a server on a free port of `127.0.0.1`, UDP and TCP.
    pub fn start(behaviour: Behaviour) -> TestServer {
        let (udp, tcp) = loop {
            let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("a UDP port");
            let port = udp.local_addr().expect("a bound address").port();
            if let Ok(tcp) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                break (udp, tcp);
            }
        };
        let addr = udp.local_addr().expect("a bound address");
        let stop = Arc::new(AtomicBool::new(false));
        let questions = Arc::new(AtomicU64::new(0));
        udp.set_read_timeout(Some(TICK)).expect("a timeout");
        tcp.set_nonblocking(true).expect("a non-blocking listener");
        let udp_thread = {
            let (stop, questions, behaviour) = (stop.clone(), questions.clone(), behaviour.clone());
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while !stop.load(Ordering::Relaxed) {
                    let Ok((n, from)) = udp.recv_from(&mut buf) else {
                        continue;
                    };
                    questions.fetch_add(1, Ordering::Relaxed);
                    if let Some(reply) = answer(&buf[..n], &behaviour) {
                        let _ = udp.send_to(&reply, from);
                    }
                }
            })
        };
        let tcp_thread = {
            let (stop, questions) = (stop.clone(), questions.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match tcp.accept() {
                        Ok((stream, _)) => {
                            let (stop, questions, behaviour) =
                                (stop.clone(), questions.clone(), behaviour.clone());
                            thread::spawn(move || serve_tcp(stream, &stop, &questions, &behaviour));
                        }
                        Err(_) => thread::sleep(TICK),
                    }
                }
            })
        };
        TestServer {
            addr,
            stop,
            questions,
            threads: vec![udp_thread, tcp_thread],
        }
    }

    /// How many questions it has had.
    pub fn questions(&self) -> u64 {
        self.questions.load(Ordering::Relaxed)
    }

    /// A port of `127.0.0.1` that nothing listens on, as far as can be told: it was bound and let
    /// go of.
    pub fn closed_port() -> SocketAddr {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("a UDP port");
        socket.local_addr().expect("a bound address")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// One TCP connection: frames in, frames out, until the client closes or the server stops.
fn serve_tcp(
    mut stream: TcpStream,
    stop: &AtomicBool,
    questions: &AtomicU64,
    behaviour: &Behaviour,
) {
    if stream.set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let mut pending = Vec::new();
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
        let mut used = 0;
        while let [a, b, ..] = pending[used..] {
            let len = usize::from(u16::from_be_bytes([a, b]));
            let Some(query) = pending.get(used + 2..used + 2 + len) else {
                break;
            };
            questions.fetch_add(1, Ordering::Relaxed);
            if let Some(reply) = answer(query, behaviour) {
                let framed = [&(reply.len() as u16).to_be_bytes()[..], &reply].concat();
                if stream.write_all(&framed).is_err() {
                    return;
                }
            }
            used += 2 + len;
        }
        pending.drain(..used);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{TYPE_A, encode_query};

    #[test]
    fn it_answers_as_its_behaviour_says() {
        let by = Behaviour::By(Arc::new(|name: &str| {
            if name == "ads.example" {
                Reply::Address(Ipv4Addr::UNSPECIFIED)
            } else {
                Reply::NxDomain
            }
        }));
        let mut query = Vec::new();
        encode_query(&mut query, 3, "Ads.Example.", TYPE_A).unwrap();
        let reply = answer(&query, &by).unwrap();
        let read = crate::wire::answer(&reply).unwrap();
        assert!(read.is_blocked() && read.rcode == 0);
        encode_query(&mut query, 3, "other.example", TYPE_A).unwrap();
        assert_eq!(
            crate::wire::answer(&answer(&query, &by).unwrap())
                .unwrap()
                .rcode,
            3
        );
        assert_eq!(answer(&query, &Behaviour::Silent), None);
        assert_eq!(answer(&query[..14], &Behaviour::Answer), None);
    }
}
