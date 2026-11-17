//! Which forwarded questions go to the upstreams, and when: each device's share of the forwards
//! that are out, and the pace of fresh names to the upstreams, for each device and for all.
//!
//! A public resolver throttles a source that sends it too many fresh names: on 2026-11-12 Quad9
//! stopped answering this household's address for about 30 s after the bench's forwarded ramp (25,
//! 50 and 100 a second, then 80), over plain UDP from Pi-hole as over TLS from qberrypi. What it
//! tolerates is not published. So all the devices together send at most [`UPSTREAM_RATE`] fresh
//! names a second, past a burst of [`UPSTREAM_BURST`] after a quiet while; and each device at most
//! [`CLIENT_RATE`], past its own [`CLIENT_BURST`], always, so that a device that floods (a broken
//! IoT device, a DGA) spends its own patience and never the house's burst: about 350 fresh names in
//! its first 2 s, and 25 a second after. Both Pi-hole and qberrypi held 100 a second to Quad9 and
//! collapsed at 200.
//!
//! A question that finds a bucket empty is **paced, not shed**: it takes its place in the bucket's
//! line and waits for its token, [`PACE_MAX`] at most, and only one that would wait longer gets
//! SERVFAIL. A question that is already out (the same name and type: a client's retransmit, or two
//! devices asking at once) costs no token. Each device may also have at most [`CLIENT_FORWARDS`]
//! forwards out at once.
//!
//! Devices are told apart by their exact address, in a table of [`DEVICES`] entries; a new device
//! takes a free entry or the least recently used idle one near its place in the table (one that
//! finds none is held to the total alone). The board's own `127.0.0.1`, the gateway and the
//! configured shared clients (a router that relays DNS, a mesh that hides its devices behind one
//! address) are held to the total alone. Everything is fixed in size: a question allocates nothing
//! here, and takes the lock once to be admitted and once to be done. Admission looks at the
//! [`MAX_FORWARDS`] questions that may be out, and at 16 entries of the table of devices.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::MAX_FORWARDS;

/// The most forwards that one device may have out at once: a quarter of [`MAX_FORWARDS`]. One
/// more from it gets SERVFAIL at once.
pub const CLIENT_FORWARDS: u16 = (MAX_FORWARDS / 4) as u16;
/// The fresh names a second that one device may send, past its burst.
pub const CLIENT_RATE: u32 = 25;
/// The fresh names that one device may send at once after a quiet while: a heavy page with its
/// A, AAAA and HTTPS questions.
pub const CLIENT_BURST: u32 = 300;
/// The fresh names a second that all the devices together send, sustained.
pub const UPSTREAM_RATE: u32 = 50;
/// The fresh names that all the devices may send at once after a quiet while: a house whose
/// devices come back after an outage with a cold cache.
pub const UPSTREAM_BURST: u32 = 1200;
/// The longest a question waits for its token before it gets SERVFAIL.
pub const PACE_MAX: Duration = Duration::from_millis(600);
/// The devices that are told apart.
pub const DEVICES: usize = 1024;
/// The entries that a device's place in the table looks at.
const PROBE: usize = 16;
/// The questions that are out that are told apart: as many as may be out
/// ([`MAX_FORWARDS`]), each by a 64-bit hash of its name and type.
const ASKING: usize = MAX_FORWARDS;

/// A bucket as the generic cell rate algorithm keeps it: the theoretical arrival time of the next
/// question, in microseconds since the start. Monotonic and 64 bits: it never wraps.
#[derive(Clone, Copy, Debug, Default)]
struct Gcra {
    tat: u64,
}

impl Gcra {
    /// The microseconds between two tokens at `rate` a second.
    fn interval(rate: u32) -> u64 {
        1_000_000 / u64::from(rate.max(1))
    }

    /// The work that is queued at `now`: how far the bucket is from full, in microseconds.
    fn backlog(&self, now: u64) -> u64 {
        self.tat.saturating_sub(now)
    }

    /// Takes a token at `now` from a bucket that earns `rate` a second and holds `burst`, if it
    /// comes within `most` microseconds: how long to wait for it. `None` takes nothing.
    fn reserve(&mut self, now: u64, rate: u32, burst: u32, most: u64) -> Option<u64> {
        let step = Self::interval(rate);
        let tolerance = step * u64::from(burst);
        let backlog = self.backlog(now);
        let wait = (backlog + step).saturating_sub(tolerance);
        if wait > most {
            return None;
        }
        self.tat = now.max(self.tat) + step;
        Some(wait)
    }

    /// Gives back a token that was taken and not used.
    fn refund(&mut self, rate: u32) {
        self.tat = self.tat.saturating_sub(Self::interval(rate));
    }
}

/// One device's entry.
#[derive(Clone, Copy, Debug, Default)]
struct Device {
    /// Its address; `None` for a free entry.
    addr: Option<IpAddr>,
    /// Its forwards out.
    out: u16,
    /// Its bucket.
    gate: Gcra,
    /// When it last asked, in microseconds since the start.
    last: u64,
}

/// What the lock holds.
struct Inner {
    devices: Box<[Device; DEVICES]>,
    /// The questions out: the hash of each and how many of it are out; 0 for a free place.
    asking: Box<[(u64, u16); ASKING]>,
    upstream: Gcra,
    /// The addresses that are held to the total alone, besides `127.0.0.1`.
    shared: Vec<IpAddr>,
    /// Whether the total applies: always, but in the host benchmark of the listener.
    rated: bool,
}

/// The admission of forwarded questions. See the module documentation.
pub(crate) struct Admit {
    inner: Mutex<Inner>,
    /// The key of the hashes, drawn at each start.
    key: u64,
    /// The zero of the clock.
    started: Instant,
}

/// What a question is to do.
pub(crate) enum Verdict<'a> {
    /// Go after this wait (none, mostly), holding the ticket until the exchange is over.
    Go(Duration, Ticket<'a>),
    /// SERVFAIL: the device has its share out, or the wait would be longer than [`PACE_MAX`].
    Shed,
}

/// A question that was admitted: its device's place and its own, given back when dropped.
pub(crate) struct Ticket<'a> {
    admit: &'a Admit,
    device: Option<usize>,
    asking: Option<usize>,
}

impl Drop for Ticket<'_> {
    fn drop(&mut self) {
        let mut inner = self.admit.lock();
        if let Some(index) = self.device {
            let device = &mut inner.devices[index];
            device.out = device.out.saturating_sub(1);
        }
        if let Some(index) = self.asking {
            let asking = &mut inner.asking[index].1;
            *asking = asking.saturating_sub(1);
        }
    }
}

impl Admit {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                devices: Box::new([Device::default(); DEVICES]),
                asking: Box::new([(0, 0); ASKING]),
                upstream: Gcra::default(),
                shared: Vec::new(),
                rated: true,
            }),
            key: crate::sys::random_u64() | 1,
            started: Instant::now(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The addresses that are held to the total alone, besides `127.0.0.1`: the gateway and the
    /// configured shared clients. Replaces what was set.
    pub(crate) fn set_shared(&self, shared: Vec<IpAddr>) {
        self.lock().shared = shared;
    }

    /// Lifts [`UPSTREAM_RATE`], for the host benchmark of the listener.
    #[cfg(test)]
    pub(crate) fn lift_upstream_rate(&self) {
        self.lock().rated = false;
    }

    /// Microseconds since the start.
    pub(crate) fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// A keyed hash of `bits`.
    fn hash(&self, bits: u64) -> u64 {
        let mixed = (bits ^ (bits >> 31)).wrapping_mul(self.key);
        mixed ^ (mixed >> 29)
    }

    /// The hash of the question for `name` and `qtype`.
    fn question_hash(&self, name: &str, qtype: u16) -> u64 {
        // FNV-1a over the name, then the type.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.bytes().chain(qtype.to_be_bytes()) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        self.hash(hash)
    }

    /// The place of the question `hash` among those out, and whether it is out already: its own,
    /// or a free one. `None` only if more than [`ASKING`] are out, which the permits prevent.
    fn asking_place(asking: &[(u64, u16); ASKING], hash: u64) -> Option<(usize, bool)> {
        let mut free = None;
        for (index, &(held, out)) in asking.iter().enumerate() {
            if out == 0 {
                free.get_or_insert(index);
            } else if held == hash {
                return Some((index, true));
            }
        }
        free.map(|index| (index, false))
    }

    /// The entry of `client` in `devices`, taking a free entry or the least recently used idle
    /// one of its window for a device that has none; `None` if every entry of the window is busy.
    fn device(&self, devices: &mut [Device; DEVICES], client: IpAddr, now: u64) -> Option<usize> {
        let bits = match client {
            IpAddr::V4(v4) => u64::from(u32::from(v4)),
            IpAddr::V6(v6) => {
                let bits = u128::from(v6);
                (bits as u64) ^ ((bits >> 64) as u64)
            }
        };
        let start = (self.hash(bits) % DEVICES as u64) as usize;
        let mut free: Option<usize> = None;
        let mut idle: Option<usize> = None;
        for step in 0..PROBE {
            let index = (start + step) % DEVICES;
            let entry = &devices[index];
            match entry.addr {
                Some(addr) if addr == client => return Some(index),
                None => {
                    free.get_or_insert(index);
                }
                Some(_) if entry.out == 0 => {
                    if idle.is_none_or(|old| devices[old].last > entry.last) {
                        idle = Some(index);
                    }
                }
                Some(_) => {}
            }
        }
        let index = free.or(idle)?;
        devices[index] = Device {
            addr: Some(client),
            out: 0,
            gate: Gcra::default(),
            last: now,
        };
        Some(index)
    }

    /// Admits a question from `client` for `name` and `qtype` at `now` (microseconds since the
    /// start). See the module documentation.
    pub(crate) fn admit(&self, client: IpAddr, name: &str, qtype: u16, now: u64) -> Verdict<'_> {
        let most = u64::try_from(PACE_MAX.as_micros()).unwrap_or(u64::MAX);
        let hash = self.question_hash(name, qtype);
        let mut inner = self.lock();
        let inner = &mut *inner;
        let shared = client == IpAddr::V4(Ipv4Addr::LOCALHOST) || inner.shared.contains(&client);
        let device = if shared {
            None
        } else {
            self.device(&mut inner.devices, client, now)
        };
        if let Some(index) = device {
            let entry = &mut inner.devices[index];
            entry.last = now;
            if entry.out >= CLIENT_FORWARDS {
                return Verdict::Shed;
            }
        }
        // A question that is out already costs no token: it is the same one again.
        let place = Self::asking_place(&inner.asking, hash);
        let again = place.is_some_and(|(_, out)| out);
        let mut wait = 0;
        if !again && inner.rated {
            // The total first, and the device's last, so that a device's token is not spent on a
            // question that the total refuses.
            let Some(total) = inner
                .upstream
                .reserve(now, UPSTREAM_RATE, UPSTREAM_BURST, most)
            else {
                return Verdict::Shed;
            };
            wait = total;
            if let Some(index) = device {
                match inner.devices[index]
                    .gate
                    .reserve(now, CLIENT_RATE, CLIENT_BURST, most)
                {
                    Some(own) => wait = wait.max(own),
                    None => {
                        inner.upstream.refund(UPSTREAM_RATE);
                        return Verdict::Shed;
                    }
                }
            }
        }
        if let Some(index) = device {
            inner.devices[index].out += 1;
        }
        let asking = place.map(|(index, _)| index);
        if let Some(index) = asking {
            let entry = &mut inner.asking[index];
            *entry = (hash, entry.1.saturating_add(1));
        }
        Verdict::Go(
            Duration::from_micros(wait),
            Ticket {
                admit: self,
                device,
                asking,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 1_000_000;
    const DAY: u64 = 86_400 * SECOND;

    fn device(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, n))
    }

    /// Admits a question, and gives the wait, or `None` for a SERVFAIL; the ticket is kept in
    /// `held` until the caller drops it.
    fn ask<'a>(
        admit: &'a Admit,
        held: &mut Vec<Ticket<'a>>,
        client: IpAddr,
        name: &str,
        now: u64,
    ) -> Option<Duration> {
        match admit.admit(client, name, 1, now) {
            Verdict::Go(wait, ticket) => {
                held.push(ticket);
                Some(wait)
            }
            Verdict::Shed => None,
        }
    }

    #[test]
    fn a_bucket_lets_its_burst_through_then_paces_and_refuses_past_the_wait() {
        let mut gate = Gcra::default();
        let now = 5 * SECOND;
        // Ten a second, three at once: the burst at once, then a wait of a token's time each.
        for _ in 0..3 {
            assert_eq!(gate.reserve(now, 10, 3, 300_000), Some(0));
        }
        assert_eq!(gate.reserve(now, 10, 3, 300_000), Some(100_000));
        assert_eq!(gate.reserve(now, 10, 3, 300_000), Some(200_000));
        assert_eq!(gate.reserve(now, 10, 3, 300_000), Some(300_000));
        // Past the longest wait: refused, and nothing taken.
        assert_eq!(gate.reserve(now, 10, 3, 300_000), None);
        assert_eq!(gate.reserve(now + 100_000, 10, 3, 300_000), Some(300_000));
    }

    #[test]
    fn a_bucket_that_was_idle_for_weeks_is_full() {
        for idle in [30 * DAY, 60 * DAY, 400 * DAY] {
            let mut gate = Gcra::default();
            // Emptied at the start, and then left alone.
            while gate.reserve(SECOND, 25, 300, 0).is_some() {}
            let later = SECOND + idle;
            for n in 0..300 {
                assert_eq!(
                    gate.reserve(later, 25, 300, 0),
                    Some(0),
                    "token {n} after {} days",
                    idle / DAY
                );
            }
            assert_eq!(gate.reserve(later, 25, 300, 0), None);
        }
        // A time before the last take (another thread read the clock first) is no refill.
        let mut gate = Gcra::default();
        while gate.reserve(10 * SECOND, 25, 300, 0).is_some() {}
        assert_eq!(gate.reserve(9 * SECOND, 25, 300, 0), None);
    }

    #[test]
    fn a_device_has_a_share_of_the_forwards_and_gets_it_back() {
        let admit = Admit::new();
        let mut held = Vec::new();
        for n in 0..CLIENT_FORWARDS {
            let name = format!("n{n}.example.com");
            assert!(ask(&admit, &mut held, device(7), &name, SECOND).is_some());
        }
        assert!(ask(&admit, &mut held, device(7), "more.example.com", SECOND).is_none());
        // Another device, with its own entry, is not held back by it.
        let mut other = Vec::new();
        assert!(ask(&admit, &mut other, device(8), "x.example.com", SECOND).is_some());
        held.clear();
        assert!(ask(&admit, &mut held, device(7), "more.example.com", SECOND).is_some());
    }

    #[test]
    fn devices_are_told_apart_by_their_exact_address() {
        let admit = Admit::new();
        let mut held = Vec::new();
        // 254 devices with one question each, and one that fills its share: no other is held.
        for n in 0..CLIENT_FORWARDS {
            ask(
                &admit,
                &mut held,
                device(1),
                &format!("f{n}.example.com"),
                SECOND,
            )
            .unwrap();
        }
        for n in 2..=254 {
            let name = format!("d{n}.example.com");
            assert!(
                ask(&admit, &mut held, device(n), &name, SECOND).is_some(),
                "device {n}"
            );
        }
    }

    /// Asks `count` fresh names, `per_batch` at once each `every`, from `client`, starting at
    /// `start`: how many got a SERVFAIL, and the longest wait. Each ticket is held for `answer`.
    fn load(
        admit: &Admit,
        clients: &[IpAddr],
        count: usize,
        per_batch: usize,
        every: u64,
        start: u64,
    ) -> (usize, Duration) {
        let mut shed = 0;
        let mut longest = Duration::ZERO;
        let mut held: Vec<(u64, Ticket<'_>)> = Vec::new();
        for n in 0..count {
            let now = start + (n / per_batch) as u64 * every;
            // An answer comes 30 ms after the question went out.
            held.retain(|(done, _)| *done > now);
            let client = clients[n % clients.len()];
            match admit.admit(client, &format!("q{n}.load{start}.example.com"), 1, now) {
                Verdict::Go(wait, ticket) => {
                    longest = longest.max(wait);
                    held.push((
                        now + u64::try_from(wait.as_micros()).unwrap() + 30_000,
                        ticket,
                    ));
                }
                Verdict::Shed => shed += 1,
            }
        }
        (shed, longest)
    }

    #[test]
    fn a_household_with_a_cold_cache_is_paced_and_never_shed() {
        let admit = Admit::new();
        // 25 devices come back at once and ask 2,000 fresh names in 30 s, front-loaded: 1,000 in
        // the first 2 s, then the rest over 28 s.
        let devices: Vec<IpAddr> = (10..35).map(device).collect();
        let (shed, longest) = load(&admit, &devices, 1000, 25, 50_000, SECOND);
        assert_eq!(shed, 0, "longest wait {longest:?}");
        let (shed, longest) = load(&admit, &devices, 1000, 5, 140_000, 3 * SECOND);
        assert_eq!(shed, 0, "longest wait {longest:?}");
        assert!(longest <= PACE_MAX);
    }

    #[test]
    fn a_heavy_page_from_one_device_is_never_shed() {
        let admit = Admit::new();
        // 300 fresh names in 2 s: 30 every 200 ms.
        let (shed, longest) = load(&admit, &[device(7)], 300, 30, 200_000, SECOND);
        assert_eq!((shed, longest), (0, Duration::ZERO));
        // And again 10 s later, on a bucket that has had time to fill.
        let (shed, _) = load(&admit, &[device(7)], 300, 30, 200_000, 12 * SECOND);
        assert_eq!(shed, 0);
    }

    #[test]
    fn a_flood_from_one_device_is_contained_and_the_others_are_not_held() {
        let admit = Admit::new();
        let flood = device(7);
        let quiet = device(8);
        let mut held = Vec::new();
        let mut through = 0;
        let mut quiet_waits = Duration::ZERO;
        // 500 fresh names a second for 10 s, one each 2 ms; the other device asks 10 a second.
        for step in 0..5_000u64 {
            let now = SECOND + step * 2_000;
            // Each one is answered at once: only the rates hold it.
            held.clear();
            let name = format!("f{step}.flood.example.com");
            through += usize::from(ask(&admit, &mut held, flood, &name, now).is_some());
            if step % 50 == 0 {
                let name = format!("q{step}.quiet.example.com");
                let wait = ask(&admit, &mut held, quiet, &name, now)
                    .unwrap_or_else(|| panic!("the quiet device shed at {step}"));
                quiet_waits = quiet_waits.max(wait);
            }
        }
        // Its own burst, its rate for the 10 s, and the 600 ms of tokens it may wait for: never
        // the house's burst, and far under the 5,000.
        let most = (CLIENT_BURST + 10 * CLIENT_RATE + CLIENT_RATE * 6 / 10 + 1) as usize;
        assert!(
            through >= (CLIENT_BURST + 9 * CLIENT_RATE) as usize,
            "{through} of the flood went: it is paced, not starved"
        );
        assert!(through <= most, "{through} of the flood went (most {most})");
        assert!(quiet_waits <= Duration::from_millis(100), "{quiet_waits:?}");
    }

    #[test]
    fn the_same_question_out_twice_costs_one_token() {
        let admit = Admit::new();
        let mut held = Vec::new();
        // Empty the house's bucket with one device's distinct names... from the board, which no
        // device rate holds.
        let board = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut n = 0;
        while ask(
            &admit,
            &mut held,
            board,
            &format!("b{n}.example.com"),
            SECOND,
        ) == Some(Duration::ZERO)
        {
            n += 1;
        }
        assert_eq!(n, UPSTREAM_BURST as usize);
        // A retransmit of a question that is out goes at once, and takes nothing.
        let again = ask(&admit, &mut held, device(9), "b0.example.com", SECOND);
        assert_eq!(again, Some(Duration::ZERO));
    }

    #[test]
    fn the_gateway_and_the_shared_clients_are_held_to_the_total_alone() {
        let admit = Admit::new();
        let gateway = device(1);
        admit.set_shared(vec![gateway, device(2)]);
        let mut held = Vec::new();
        // Half the house's bucket spent by the board, which no device rate holds.
        for n in 0..UPSTREAM_BURST / 2 + 1 {
            held.clear();
            ask(
                &admit,
                &mut held,
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                &format!("w{n}.example"),
                SECOND,
            )
            .unwrap();
        }
        let mut through = |client: IpAddr| {
            let mut through = 0;
            for n in 0..400 {
                held.clear();
                let name = format!("{client}-{n}.example");
                through += usize::from(
                    ask(&admit, &mut held, client, &name, SECOND) == Some(Duration::ZERO),
                );
            }
            through
        };
        // A device's own burst, and then waits; the gateway, all that is left of the total.
        assert_eq!(through(device(30)), CLIENT_BURST as usize);
        assert!(through(gateway) > 0);
        // No share either: more than one device's worth out at once.
        let mut out = Vec::new();
        for n in 0..u32::from(CLIENT_FORWARDS) + 10 {
            let name = format!("s{n}.example");
            assert!(ask(&admit, &mut out, device(2), &name, 100 * SECOND).is_some());
        }
    }
}
