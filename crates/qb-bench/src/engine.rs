//! The load itself: questions sent at a set rate, open loop, with many in flight, each matched to
//! its answer by ID and given up after a wait.
//!
//! One thread does all of it, over one socket (UDP) or one connection (TCP), and sleeps in
//! `poll` between a send that is due and an answer that arrives, so the processor time that a
//! report gives for the generator is that of one thread. Question `k` of a step is due at
//! `k / rate` after the step began, whatever came of the ones before it: the rate does not slow
//! down for a slow server, as a household's devices do not. Only when [`Limits::max_in_flight`]
//! questions are out does the next one wait for an answer, and a step that falls behind its rate
//! that way stops sending at its end and says how many it sent.
//!
//! The IDs are handed out in turn from a queue of all 65,536, and one that comes back goes to the
//! end of it: an ID is used again only after every other free one has been, so an answer that
//! comes after its question was given up is not taken for the answer to the next question under
//! that ID. Such an answer is counted as late.

use std::collections::VecDeque;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::names::Names;
use crate::wire::{self, TYPE_A, encode_query, is_failure};

/// The most questions that may be out at once, whatever is asked: half the IDs, so that an ID
/// that comes back is used again only after at least 32,768 others.
pub const MAX_IN_FLIGHT: usize = 32_768;
/// The receive buffer the generator asks for, so that answers that come in a burst are not lost
/// on its side of the socket before it reads them.
const RECV_BUFFER: usize = 4 << 20;
/// The send buffer the generator asks for.
const SEND_BUFFER: usize = 1 << 20;
/// The longest UDP answer read; a longer one is cut, which changes nothing that is read of it.
const UDP_READ: usize = 4096;
/// How many bytes may wait to be written on a TCP connection before the next question waits.
const TCP_BACKLOG: usize = 1 << 20;
/// How far past the end of its step a load that is behind its rate may still send what is due.
const BEHIND_GRACE: Duration = Duration::from_millis(50);
/// How long a TCP connect may take.
const CONNECT_WAIT: Duration = Duration::from_secs(5);

/// How the questions travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Udp,
    /// One connection, the questions pipelined on it, each framed with its length.
    Tcp,
}

impl Transport {
    /// Its name in reports.
    pub fn word(self) -> &'static str {
        match self {
            Transport::Udp => "udp",
            Transport::Tcp => "tcp",
        }
    }
}

/// What bounds the questions of a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long a question waits for its answer before it counts as lost.
    pub timeout: Duration,
    /// The most that are out at once; at most [`MAX_IN_FLIGHT`].
    pub max_in_flight: usize,
}

/// What came of the questions of one step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepCounts {
    /// Questions sent, a send that failed included.
    pub sent: u64,
    /// Answered with anything but SERVFAIL or REFUSED.
    pub answered: u64,
    /// Answered SERVFAIL or REFUSED.
    pub failed: u64,
    /// Not answered within the wait, or lost to a failed send.
    pub lost: u64,
    /// Answers to no question that was out: after their question was given up, or with an ID
    /// that was never sent. Not counted in `answered`.
    pub late: u64,
    /// The sends and receives that failed (the server's port closed, say).
    pub socket_errors: u64,
    /// The first of those, in words.
    pub first_error: Option<String>,
    /// The time of each answer counted in `answered`, in microseconds, in the order they came.
    pub latencies_us: Vec<u32>,
    /// From the start of the step to its last send.
    pub sending: Duration,
}

impl StepCounts {
    /// Notes a failed send or receive.
    fn error(&mut self, error: &io::Error) {
        self.socket_errors += 1;
        self.first_error.get_or_insert_with(|| error.to_string());
    }
}

/// The questions that are out, by ID.
struct Book {
    /// When the question under each ID was sent; `None` for an ID that is free.
    sent_at: Vec<Option<Instant>>,
    /// The free IDs, the next to use first.
    free: VecDeque<u16>,
    /// The IDs in the order they were sent, with the time; an entry whose ID has since been
    /// answered, or used again, is left to be skipped.
    order: VecDeque<(u16, Instant)>,
    in_flight: usize,
    timeout: Duration,
}

/// What an answer was to the book.
enum Settled {
    /// The answer to a question that was out, after this long.
    Answered(Duration),
    /// No question was out under its ID, or it came after the wait.
    Late,
}

impl Book {
    fn new(timeout: Duration) -> Self {
        Self {
            sent_at: vec![None; usize::from(u16::MAX) + 1],
            free: (0..=u16::MAX).collect(),
            order: VecDeque::new(),
            in_flight: 0,
            timeout,
        }
    }

    /// The next free ID.
    fn take(&mut self) -> Option<u16> {
        self.free.pop_front()
    }

    /// Gives back an ID that was taken and not sent; it is the next to use again.
    fn untake(&mut self, id: u16) {
        self.free.push_front(id);
    }

    /// Notes that the question under `id` was sent at `at`.
    fn sent(&mut self, id: u16, at: Instant) {
        self.sent_at[usize::from(id)] = Some(at);
        self.order.push_back((id, at));
        self.in_flight += 1;
    }

    /// Takes in an answer under `id` that arrived at `at`.
    fn settle(&mut self, id: u16, at: Instant) -> Settled {
        let Some(sent) = self.sent_at[usize::from(id)] else {
            return Settled::Late;
        };
        let took = at.saturating_duration_since(sent);
        if took > self.timeout {
            // Given up on already, though not yet expired: the expiry counts it as lost.
            return Settled::Late;
        }
        self.release(id);
        Settled::Answered(took)
    }

    fn release(&mut self, id: u16) {
        self.sent_at[usize::from(id)] = None;
        self.free.push_back(id);
        self.in_flight -= 1;
    }

    /// The oldest question still out, with the time it was sent; the entries before it, which are
    /// answered or reused, are dropped on the way.
    fn oldest(&mut self) -> Option<(u16, Instant)> {
        while let Some(&(id, at)) = self.order.front() {
            if self.sent_at[usize::from(id)] == Some(at) {
                return Some((id, at));
            }
            self.order.pop_front();
        }
        None
    }

    /// Gives up every question sent longer than the wait before `now`; how many.
    fn expire(&mut self, now: Instant) -> u64 {
        let mut expired = 0;
        while let Some((id, at)) = self.oldest() {
            if now.saturating_duration_since(at) < self.timeout {
                break;
            }
            self.order.pop_front();
            self.release(id);
            expired += 1;
        }
        expired
    }

    /// When the oldest question still out will be given up.
    fn next_expiry(&mut self) -> Option<Instant> {
        self.oldest().map(|(_, at)| at + self.timeout)
    }

    /// Gives up every question still out; how many.
    fn expire_all(&mut self) -> u64 {
        let mut expired = 0;
        while let Some((id, _)) = self.oldest() {
            self.order.pop_front();
            self.release(id);
            expired += 1;
        }
        expired
    }
}

/// A TCP connection with what is still to be written on it and what has been read of a frame.
struct Tcp {
    addr: SocketAddr,
    stream: Option<TcpStream>,
    read: Vec<u8>,
    write: Vec<u8>,
    written: usize,
}

impl Tcp {
    fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&addr, CONNECT_WAIT)?;
        stream.set_nodelay(true)?;
        stream.set_nonblocking(true)?;
        Ok(stream)
    }

    /// The connection, made again if the server closed the last one.
    fn stream(&mut self) -> io::Result<&mut TcpStream> {
        if self.stream.is_none() {
            self.read.clear();
            self.write.clear();
            self.written = 0;
            self.stream = Some(Self::connect(self.addr)?);
        }
        self.stream
            .as_mut()
            .ok_or_else(|| io::Error::from(ErrorKind::NotConnected))
    }

    /// Writes what it can of what is waiting.
    fn flush(&mut self) -> io::Result<()> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        while self.written < self.write.len() {
            match stream.write(&self.write[self.written..]) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => self.written += n,
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => {
                    self.stream = None;
                    return Err(error);
                }
            }
        }
        if self.written == self.write.len() {
            self.write.clear();
            self.written = 0;
        }
        Ok(())
    }
}

/// The socket or connection to the server.
enum Link {
    Udp(UdpSocket),
    Tcp(Tcp),
}

/// Sets a socket buffer as large as the kernel allows: with the privilege to force it, the size
/// asked for; without, the kernel's maximum. A failure leaves the default, which is no error.
fn grow_buffers(fd: impl AsFd) {
    use rustix::net::sockopt;
    if sockopt::set_socket_recv_buffer_size_force(&fd, RECV_BUFFER).is_err() {
        let _ = sockopt::set_socket_recv_buffer_size(&fd, RECV_BUFFER);
    }
    if sockopt::set_socket_send_buffer_size_force(&fd, SEND_BUFFER).is_err() {
        let _ = sockopt::set_socket_send_buffer_size(&fd, SEND_BUFFER);
    }
}

impl Link {
    fn open(server: SocketAddr, transport: Transport) -> io::Result<Link> {
        match transport {
            Transport::Udp => {
                let local: IpAddr = if server.ip().is_loopback() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    Ipv4Addr::UNSPECIFIED.into()
                };
                let socket = UdpSocket::bind((local, 0))?;
                socket.connect(server)?;
                socket.set_nonblocking(true)?;
                grow_buffers(&socket);
                Ok(Link::Udp(socket))
            }
            Transport::Tcp => {
                let stream = Tcp::connect(server)?;
                grow_buffers(&stream);
                Ok(Link::Tcp(Tcp {
                    addr: server,
                    stream: Some(stream),
                    read: Vec::new(),
                    write: Vec::new(),
                    written: 0,
                }))
            }
        }
    }

    /// Sends one question: `Ok(false)` when it must wait for room.
    fn send(&mut self, packet: &[u8]) -> io::Result<bool> {
        match self {
            Link::Udp(socket) => match socket.send(packet) {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(false),
                Err(error) => Err(error),
            },
            Link::Tcp(tcp) => {
                if tcp.write.len() - tcp.written >= TCP_BACKLOG {
                    tcp.flush()?;
                    if tcp.write.len() - tcp.written >= TCP_BACKLOG {
                        return Ok(false);
                    }
                }
                tcp.stream()?;
                let len = u16::try_from(packet.len()).map_err(|_| ErrorKind::InvalidInput)?;
                tcp.write.extend_from_slice(&len.to_be_bytes());
                tcp.write.extend_from_slice(packet);
                tcp.flush()?;
                Ok(true)
            }
        }
    }

    /// Reads every answer that is there, giving each to `each` with the time it was read; a
    /// failed read is given to `error` and ends the reading.
    fn receive(
        &mut self,
        buf: &mut [u8],
        mut each: impl FnMut(&[u8], Instant),
        mut error: impl FnMut(&io::Error),
    ) {
        match self {
            Link::Udp(socket) => loop {
                match socket.recv(buf) {
                    Ok(n) => each(&buf[..n], Instant::now()),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => {
                        error(&e);
                        return;
                    }
                }
            },
            Link::Tcp(tcp) => {
                let Some(stream) = tcp.stream.as_mut() else {
                    return;
                };
                loop {
                    match stream.read(buf) {
                        Ok(0) => {
                            // The server closed it: what was out on it is lost, and the next
                            // send connects again.
                            tcp.stream = None;
                            break;
                        }
                        Ok(n) => tcp.read.extend_from_slice(&buf[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        Err(e) => {
                            tcp.stream = None;
                            error(&e);
                            break;
                        }
                    }
                }
                let at = Instant::now();
                let mut used = 0;
                while let [a, b, ..] = tcp.read[used..] {
                    let len = usize::from(u16::from_be_bytes([a, b]));
                    let Some(frame) = tcp.read.get(used + 2..used + 2 + len) else {
                        break;
                    };
                    each(frame, at);
                    used += 2 + len;
                }
                tcp.read.drain(..used);
                if let Err(e) = tcp.flush() {
                    error(&e);
                }
            }
        }
    }

    /// Waits until an answer may be read, or room to write is wanted and there, or `wait` has
    /// passed. A wait of zero only looks.
    fn wait(&self, wait: Duration) -> io::Result<()> {
        let (fd, write) = match self {
            Link::Udp(socket) => (socket.as_fd(), false),
            Link::Tcp(tcp) => match &tcp.stream {
                Some(stream) => (stream.as_fd(), tcp.written < tcp.write.len()),
                None => return Ok(()),
            },
        };
        let mut flags = PollFlags::IN;
        if write {
            flags |= PollFlags::OUT;
        }
        let mut fds = [PollFd::from_borrowed_fd(fd, flags)];
        let timeout = Timespec::try_from(wait).unwrap_or(Timespec {
            tv_sec: i64::MAX,
            tv_nsec: 0,
        });
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// The generator for one server: the link, the questions out and the buffers.
pub struct Engine {
    link: Link,
    book: Book,
    limits: Limits,
    transport: Transport,
    packet: Vec<u8>,
    buf: Vec<u8>,
}

impl Engine {
    /// A generator that asks `server` over `transport` within `limits`. The error is that of the
    /// socket, or of the connection.
    pub fn connect(server: SocketAddr, transport: Transport, limits: Limits) -> io::Result<Engine> {
        Ok(Engine {
            link: Link::open(server, transport)?,
            book: Book::new(limits.timeout),
            limits: Limits {
                max_in_flight: limits.max_in_flight.clamp(1, MAX_IN_FLIGHT),
                ..limits
            },
            transport,
            packet: Vec::with_capacity(512),
            buf: vec![0; UDP_READ],
        })
    }

    /// How the questions travel.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// The bounds it works within.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Asks `name` once and waits up to `wait` for its answer, which is returned whole; `None`
    /// when none came. Answers to other questions are read and dropped. For the checks before a
    /// load, with nothing else out.
    pub fn ask(&mut self, name: &str, wait: Duration) -> io::Result<Option<Vec<u8>>> {
        let id = self
            .book
            .take()
            .ok_or_else(|| io::Error::other("no free ID"))?;
        if let Err(error) = encode_query(&mut self.packet, id, name, TYPE_A) {
            self.book.untake(id);
            return Err(io::Error::new(ErrorKind::InvalidInput, error));
        }
        let started = Instant::now();
        let deadline = started + wait;
        loop {
            match self.link.send(&self.packet) {
                Ok(true) => break,
                Ok(false) if Instant::now() < deadline => {
                    self.link.wait(Duration::from_millis(1))?
                }
                Ok(false) => {
                    self.book.untake(id);
                    return Ok(None);
                }
                Err(error) => {
                    self.book.untake(id);
                    return Err(error);
                }
            }
        }
        self.book.sent(id, started);
        let mut found = None;
        let mut failure = None;
        loop {
            let book = &mut self.book;
            self.link.receive(
                &mut self.buf,
                |reply, at| {
                    if let Some(head) = wire::header(reply)
                        && head.response
                        && head.id == id
                        && found.is_none()
                        && matches!(book.settle(id, at), Settled::Answered(_))
                    {
                        found = Some(reply.to_vec());
                    }
                },
                |error| {
                    failure.get_or_insert_with(|| io::Error::new(error.kind(), error.to_string()));
                },
            );
            if let Some(reply) = found {
                return Ok(Some(reply));
            }
            if let Some(error) = failure {
                self.book.expire_all();
                return Err(error);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.book.expire_all();
                return Ok(None);
            }
            self.link.wait(left)?;
        }
    }

    /// Sends `qps` questions a second for `length`, each the next of `names`, and waits for what
    /// is still out once the last is sent, up to the wait. Nothing is out when it returns.
    pub fn step(&mut self, qps: u32, length: Duration, names: &mut Names) -> StepCounts {
        let mut counts = StepCounts::default();
        let qps = u64::from(qps.max(1));
        let total = (u128::from(qps) * length.as_nanos() / 1_000_000_000) as u64;
        let started = Instant::now();
        let send_end = started + length;
        let due = |k: u64| {
            started + Duration::from_nanos((u128::from(k) * 1_000_000_000 / u128::from(qps)) as u64)
        };
        let mut next = 0u64;
        let mut stopped = false;
        let mut last_send = started;
        loop {
            let now = Instant::now();
            // The sends that are due.
            while !stopped && next < total && self.book.in_flight < self.limits.max_in_flight {
                if due(next) > now {
                    break;
                }
                let Some(id) = self.book.take() else {
                    break;
                };
                names.write_next(id, &mut self.packet);
                let at = Instant::now();
                match self.link.send(&self.packet) {
                    Ok(true) => {
                        self.book.sent(id, at);
                        counts.sent += 1;
                        next += 1;
                        last_send = at;
                    }
                    Ok(false) => {
                        self.book.untake(id);
                        break;
                    }
                    Err(error) => {
                        self.book.untake(id);
                        counts.error(&error);
                        counts.sent += 1;
                        counts.lost += 1;
                        next += 1;
                        last_send = at;
                    }
                }
            }
            if !stopped && next < total && now >= send_end + BEHIND_GRACE {
                // Behind the rate at the end of the step: the rest is not sent.
                stopped = true;
            }
            let sending = !stopped && next < total;

            let book = &mut self.book;
            let mut errors = Vec::new();
            self.link.receive(
                &mut self.buf,
                |reply, at| {
                    let Some(head) = wire::header(reply).filter(|head| head.response) else {
                        counts.late += 1;
                        return;
                    };
                    match book.settle(head.id, at) {
                        Settled::Answered(_) if is_failure(head.rcode) => counts.failed += 1,
                        Settled::Answered(took) => {
                            counts.answered += 1;
                            counts
                                .latencies_us
                                .push(u32::try_from(took.as_micros()).unwrap_or(u32::MAX));
                        }
                        Settled::Late => counts.late += 1,
                    }
                },
                |error| errors.push(io::Error::new(error.kind(), error.to_string())),
            );
            for error in &errors {
                counts.error(error);
            }
            let now = Instant::now();
            counts.lost += self.book.expire(now);
            if !sending && self.book.in_flight == 0 {
                break;
            }

            // Sleep until the next send is due, or the oldest question is given up, or an answer
            // comes, whichever is first.
            let mut wake = self.book.next_expiry();
            if sending && self.book.in_flight < self.limits.max_in_flight {
                let at = due(next);
                wake = Some(wake.map_or(at, |w| w.min(at)));
            }
            let wait = wake.map_or(self.limits.timeout, |w| w.saturating_duration_since(now));
            if let Err(error) = self.link.wait(wait) {
                counts.error(&error);
                counts.lost += self.book.expire_all();
                break;
            }
        }
        counts.sending = last_send.saturating_duration_since(started);
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Behaviour, TestServer};

    fn limits() -> Limits {
        Limits {
            timeout: Duration::from_millis(300),
            max_in_flight: 1000,
        }
    }

    fn names(list: &[&str]) -> Names {
        Names::new(&list.iter().map(|&n| n.to_owned()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn the_book_hands_out_ids_in_turn_and_reuses_one_last() {
        let mut book = Book::new(Duration::from_secs(1));
        let now = Instant::now();
        let first = book.take().unwrap();
        book.sent(first, now);
        assert!(matches!(book.settle(first, now), Settled::Answered(_)));
        // The answered ID goes to the back: the next is another.
        assert_ne!(book.take(), Some(first));
        // A second answer under it is late.
        assert!(matches!(book.settle(first, now), Settled::Late));
        assert_eq!(book.in_flight, 0);
    }

    #[test]
    fn the_book_gives_up_what_is_older_than_the_wait() {
        let mut book = Book::new(Duration::from_millis(10));
        let t0 = Instant::now();
        for i in 0..3 {
            let id = book.take().unwrap();
            book.sent(id, t0 + Duration::from_millis(i * 5));
        }
        assert_eq!(book.next_expiry(), Some(t0 + Duration::from_millis(10)));
        assert_eq!(book.expire(t0 + Duration::from_millis(16)), 2);
        assert_eq!(book.in_flight, 1);
        // An answer that comes after the wait is late, though not yet expired.
        let last = book.oldest().unwrap().0;
        assert!(matches!(
            book.settle(last, t0 + Duration::from_millis(30)),
            Settled::Late
        ));
        assert_eq!(book.expire_all(), 1);
        assert_eq!(book.next_expiry(), None);
    }

    #[test]
    fn a_step_sends_at_the_rate_and_every_answer_is_counted() {
        for transport in [Transport::Udp, Transport::Tcp] {
            let server = TestServer::start(Behaviour::Answer);
            let mut engine = Engine::connect(server.addr, transport, limits()).unwrap();
            let started = Instant::now();
            let counts = engine.step(500, Duration::from_millis(400), &mut names(&["a.example"]));
            assert_eq!(counts.sent, 200, "{transport:?}");
            assert_eq!(counts.answered, 200, "{transport:?}");
            assert_eq!((counts.lost, counts.failed, counts.late), (0, 0, 0));
            assert_eq!(counts.latencies_us.len(), 200);
            // Paced, not sent at once: the last question went out near the end of the step.
            assert!(
                counts.sending >= Duration::from_millis(350),
                "{:?}",
                counts.sending
            );
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(server.questions(), 200);
        }
    }

    #[test]
    fn unanswered_questions_are_lost_after_the_wait_and_failures_are_counted_apart() {
        let silent = TestServer::start(Behaviour::Silent);
        let mut engine = Engine::connect(silent.addr, Transport::Udp, limits()).unwrap();
        let started = Instant::now();
        let counts = engine.step(100, Duration::from_millis(100), &mut names(&["a.example"]));
        assert_eq!((counts.sent, counts.answered, counts.lost), (10, 0, 10));
        // The step waits out the last question's wait, and no longer.
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(390) && took < Duration::from_secs(1),
            "{took:?}"
        );

        let failing = TestServer::start(Behaviour::Servfail);
        let mut engine = Engine::connect(failing.addr, Transport::Udp, limits()).unwrap();
        let counts = engine.step(100, Duration::from_millis(100), &mut names(&["a.example"]));
        assert_eq!((counts.sent, counts.failed, counts.answered), (10, 10, 0));
        assert!(counts.latencies_us.is_empty());
    }

    #[test]
    fn the_in_flight_cap_holds_the_rate_back_and_the_step_says_how_many_it_sent() {
        let silent = TestServer::start(Behaviour::Silent);
        let capped = Limits {
            timeout: Duration::from_millis(200),
            max_in_flight: 5,
        };
        let mut engine = Engine::connect(silent.addr, Transport::Udp, capped).unwrap();
        let counts = engine.step(1000, Duration::from_millis(100), &mut names(&["a.example"]));
        // Five go out, nothing comes back for 200 ms, and the step is over by then.
        assert_eq!(counts.sent, 5);
        assert_eq!(counts.lost, 5);
    }

    #[test]
    fn a_closed_port_is_counted_and_does_not_stop_the_step() {
        let closed = TestServer::closed_port();
        let mut engine = Engine::connect(closed, Transport::Udp, limits()).unwrap();
        let counts = engine.step(100, Duration::from_millis(100), &mut names(&["a.example"]));
        assert_eq!(counts.sent, 10);
        assert_eq!(counts.answered, 0);
        assert_eq!(counts.lost, 10);
        assert!(counts.socket_errors > 0);
        assert!(counts.first_error.is_some());
    }

    #[test]
    fn ask_gives_the_answer_to_its_own_question() {
        let server = TestServer::start(Behaviour::Answer);
        let mut engine = Engine::connect(server.addr, Transport::Udp, limits()).unwrap();
        let reply = engine
            .ask("a.example", Duration::from_millis(500))
            .unwrap()
            .unwrap();
        let answer = wire::answer(&reply).unwrap();
        assert_eq!(answer.v4, [Ipv4Addr::new(192, 0, 2, 1)]);
        let silent = TestServer::start(Behaviour::Silent);
        let mut engine = Engine::connect(silent.addr, Transport::Udp, limits()).unwrap();
        assert_eq!(
            engine.ask("a.example", Duration::from_millis(50)).unwrap(),
            None
        );
        // Nothing is left out after it.
        assert_eq!(engine.book.in_flight, 0);
    }
}
