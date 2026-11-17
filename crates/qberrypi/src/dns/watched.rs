//! The devices that asked to see their own queries: an opt-in table, small and short-lived.
//!
//! qberrypi keeps no link between a name that was asked for and the device that asked it, except
//! here, and here only for an address that opted in: a device that asks the Pi about itself
//! ([`Watched::touch`]) is watched, and from then on its answered queries are kept, so that it can
//! be shown them. The table holds [`MAX_WATCHED`] addresses at most, keeps [`RING`] queries and
//! the counts of the last 24 hours for each, and forgets an address, with all it holds, that has
//! not been refreshed for [`IDLE_SECS`]: by a refresh, and by every query that DNS answers, so that
//! a device that went quiet is forgotten on time whoever else is about. A query of an address that
//! is not watched is not kept, and neither is a reverse lookup of any, as its name is an address.
//!
//! The table has no `Debug`, as its addresses must not reach a log line, and nothing here logs.

use std::collections::VecDeque;
use std::net::Ipv4Addr;

/// The most addresses the table watches at once.
pub const MAX_WATCHED: usize = 8;
/// The most queries kept for each address: the newest.
pub const RING: usize = 32;
/// How long an address stays watched without being refreshed.
pub const IDLE_SECS: u64 = 15 * 60;

/// The seconds of an hour, which the counts are kept in.
const HOUR_SECS: u64 = 3_600;
/// The hours that the counts of an address cover.
const WINDOW_HOURS: usize = 24;

/// One query that a watched device asked.
///
/// Its `Debug` shows the name: it must never reach a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    /// The wall-clock time of the query, in seconds since the Unix epoch.
    pub unix_secs: u64,
    /// The name asked for, as the resolver recorded it.
    pub name: Box<str>,
    /// Whether the query was blocked.
    pub refused: bool,
    /// Whether the block was of a threat list.
    pub threat: bool,
}

/// What the table holds of one watched device, as it is shown to that device.
///
/// Its `Debug` shows the names: it must never reach a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mine {
    /// The wall-clock time at which the device began to be watched, in seconds since the Unix
    /// epoch, as the clock reads when this is made: the seconds watched are counted on the
    /// monotonic clock, so it is right once the clock is set, whenever the device opted in. Not
    /// moved by a refresh, but by a return after the address was forgotten.
    pub watching_since_unix: u64,
    /// The queries in the current monotonic hour and the 23 before it.
    pub asked_24h: u32,
    /// How many of them were blocked.
    pub refused_24h: u32,
    /// How many of them were blocked by a threat list.
    pub threats_24h: u32,
    /// The latest queries, newest first: [`RING`] at most.
    pub recent: Vec<Seen>,
}

/// The queries of one hour, by what became of them.
#[derive(Debug, Clone, Copy, Default)]
struct Hits {
    /// The queries.
    asked: u32,
    /// The queries that were blocked.
    refused: u32,
    /// The queries that were blocked by a threat list.
    threats: u32,
}

/// One watched address and what is kept for it.
struct Entry {
    addr: Ipv4Addr,
    /// The monotonic second at which the address began to be watched. The wall clock is not kept:
    /// the Pi has no clock of its own, and one that was read before it was set is wrong for as
    /// long as it is kept, so the start on the wall clock is worked out when it is asked for.
    since_mono: u64,
    /// The monotonic second of the latest touch.
    last_touch: u64,
    /// The latest hour in which a query was counted.
    hour: u64,
    /// The counts of hour `h` are in bucket `h % 24`, for the 24 hours ending at `hour`; each
    /// count stays at `u32::MAX` once full. The same plan as the hourly block counts of the
    /// resolver's statistics.
    buckets: [Hits; WINDOW_HOURS],
    /// The latest queries, oldest first.
    ring: VecDeque<Seen>,
}

impl Entry {
    /// An address that begins to be watched at `mono_secs`, with nothing kept.
    fn new(addr: Ipv4Addr, mono_secs: u64) -> Self {
        Self {
            addr,
            since_mono: mono_secs,
            last_touch: mono_secs,
            hour: mono_secs / HOUR_SECS,
            buckets: [Hits::default(); WINDOW_HOURS],
            ring: VecDeque::new(),
        }
    }

    /// Whether the address was last touched [`IDLE_SECS`] ago or more at `mono_secs`.
    fn idle(&self, mono_secs: u64) -> bool {
        mono_secs.saturating_sub(self.last_touch) >= IDLE_SECS
    }

    /// The bucket that holds the counts of `hour`.
    fn bucket_of(hour: u64) -> usize {
        // Under 24, so the conversion loses nothing.
        (hour % WINDOW_HOURS as u64) as usize
    }

    /// The buckets as they stand once the clock has reached `hour`: every hour that has passed
    /// since the latest count took over the bucket of the hour 24 before it, which has left the
    /// window, so that bucket is empty. An `hour` that is not after the latest count, which takes
    /// a clock going back, leaves them as they are.
    fn buckets_at(&self, hour: u64) -> [Hits; WINDOW_HOURS] {
        let mut buckets = self.buckets;
        let passed = hour.saturating_sub(self.hour).min(WINDOW_HOURS as u64);
        for step in 1..=passed {
            // `self.hour % 24 + step` is at most 47. The bucket of the hour `step` after the
            // latest count is the one that held the hour 24 before it.
            let left = Self::bucket_of(self.hour % WINDOW_HOURS as u64 + step);
            if let Some(bucket) = buckets.get_mut(left) {
                *bucket = Hits::default();
            }
        }
        buckets
    }

    /// Keeps one answered query at `mono_secs`: counted at its hour, or in the latest hour seen
    /// if that is before it, and put at the head of the ring, which drops its oldest if full.
    fn record(&mut self, mono_secs: u64, unix_secs: u64, name: &str, refused: bool, threat: bool) {
        self.buckets = self.buckets_at(mono_secs / HOUR_SECS);
        self.hour = self.hour.max(mono_secs / HOUR_SECS);
        if let Some(hits) = self.buckets.get_mut(Self::bucket_of(self.hour)) {
            hits.asked = hits.asked.saturating_add(1);
            hits.refused = hits.refused.saturating_add(u32::from(refused));
            hits.threats = hits.threats.saturating_add(u32::from(threat));
        }
        if self.ring.len() == RING {
            self.ring.pop_front();
        }
        self.ring.push_back(Seen {
            unix_secs,
            name: name.into(),
            refused,
            threat,
        });
    }

    /// What is kept for the address, as it stands at `mono_secs`, which is `unix_secs` on the wall
    /// clock as it reads now: the start of the watch is that time less the seconds watched since,
    /// so a clock that was stepped after the watch began does not leave the start behind.
    fn mine(&self, mono_secs: u64, unix_secs: u64) -> Mine {
        let (mut asked, mut refused, mut threats) = (0u32, 0u32, 0u32);
        for hits in self.buckets_at(mono_secs / HOUR_SECS) {
            asked = asked.saturating_add(hits.asked);
            refused = refused.saturating_add(hits.refused);
            threats = threats.saturating_add(hits.threats);
        }
        Mine {
            watching_since_unix: unix_secs
                .saturating_sub(mono_secs.saturating_sub(self.since_mono)),
            asked_24h: asked,
            refused_24h: refused,
            threats_24h: threats,
            recent: self.ring.iter().rev().cloned().collect(),
        }
    }
}

/// The reverse zones, whose names are addresses written backwards: `5.2.0.192.in-addr.arpa`.
const REVERSE_ZONES: [&[u8]; 2] = [b".in-addr.arpa", b".ip6.arpa"];

/// Whether `name` is a name under `in-addr.arpa` or `ip6.arpa`: a reverse lookup, which asks who
/// an address is and so holds the address. The zone is matched as DNS does, without regard to
/// case, and the root dot at the end of the name does not matter. Allocates nothing.
fn is_reverse_lookup(name: &str) -> bool {
    let name = name.strip_suffix('.').unwrap_or(name).as_bytes();
    REVERSE_ZONES.iter().any(|zone| {
        name.len()
            .checked_sub(zone.len())
            .and_then(|start| name.get(start..))
            .is_some_and(|tail| tail.eq_ignore_ascii_case(zone))
    })
}

/// The devices that are watched: [`MAX_WATCHED`] at most, the least recently refreshed forgotten
/// first.
///
/// It reads no clock: the caller says what time it is, in monotonic and wall-clock seconds. It
/// has no `Debug`, as its addresses must not reach a log line.
#[derive(Default)]
pub struct Watched {
    /// Ordered by when each was last touched, the least recent first: one that is touched again
    /// moves to the end.
    entries: Vec<Entry>,
}

impl Watched {
    /// The caller opts in or is refreshed. Every address that was last touched [`IDLE_SECS`]
    /// ago or more is forgotten first, with all that is kept for it, `addr` too if it is one: it
    /// then begins again. A new address that finds the table full takes the place of the one
    /// that was refreshed least recently. Returns what is kept for `addr`.
    pub fn touch(&mut self, addr: Ipv4Addr, mono_secs: u64, unix_secs: u64) -> Mine {
        self.entries.retain(|entry| !entry.idle(mono_secs));
        let mut entry = match self.entries.iter().position(|e| e.addr == addr) {
            Some(at) => self.entries.remove(at),
            None => {
                if self.entries.len() == MAX_WATCHED {
                    self.entries.remove(0);
                }
                Entry::new(addr, mono_secs)
            }
        };
        entry.last_touch = mono_secs;
        let mine = entry.mine(mono_secs, unix_secs);
        self.entries.push(entry);
        mine
    }

    /// Whether `addr` is in the table at `mono_secs`: for the caller that would copy a name out
    /// only for an address that is. First every address that was last touched [`IDLE_SECS`] ago
    /// or more is forgotten, with all that is kept for it, whoever `addr` is: this is what the
    /// DNS path calls for every query, so a device that was switched off does not keep its
    /// queries in memory until another device happens to call [`Watched::touch`]. With nothing
    /// watched it is one look at an empty table; with devices, a scan of at most
    /// [`MAX_WATCHED`].
    pub fn is_watching(&mut self, addr: Ipv4Addr, mono_secs: u64) -> bool {
        self.entries.retain(|entry| !entry.idle(mono_secs));
        self.entries.iter().any(|entry| entry.addr == addr)
    }

    /// Records one answered query, only if `addr` is watched: for any other address it does
    /// nothing and allocates nothing. An address that has been idle for [`IDLE_SECS`] is no longer
    /// watched: it is forgotten here, with what was kept for it, and the query is not kept.
    ///
    /// A reverse lookup, a name under `in-addr.arpa` or `ip6.arpa` in any case and with or without
    /// the root dot, is left out altogether: it is not kept and not counted, as the name is an
    /// address, and no address may be in what the device is shown.
    pub fn note(
        &mut self,
        addr: Ipv4Addr,
        mono_secs: u64,
        unix_secs: u64,
        name: &str,
        refused: bool,
        threat: bool,
    ) {
        if is_reverse_lookup(name) {
            return;
        }
        let Some(at) = self.entries.iter().position(|entry| entry.addr == addr) else {
            return;
        };
        let Some(entry) = self.entries.get_mut(at) else {
            return;
        };
        if entry.idle(mono_secs) {
            self.entries.remove(at);
            return;
        }
        entry.record(mono_secs, unix_secs, name, refused, threat);
    }

    /// How many addresses the table watches.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table watches none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 198.51.100.n, a documentation range.
    fn v4(n: u8) -> Ipv4Addr {
        Ipv4Addr::new(198, 51, 100, n)
    }

    /// The names of `mine.recent`, newest first.
    fn names(mine: &Mine) -> Vec<&str> {
        mine.recent.iter().map(|seen| &*seen.name).collect()
    }

    /// The wall-clock second that goes with the monotonic second `mono`, in these tests.
    fn unix(mono: u64) -> u64 {
        1_800_000_000 + mono
    }

    /// `touch` at `mono`, with its wall-clock second.
    fn touch(w: &mut Watched, addr: Ipv4Addr, mono: u64) -> Mine {
        w.touch(addr, mono, unix(mono))
    }

    /// An answered query that was let through, noted at `mono`.
    fn note(w: &mut Watched, addr: Ipv4Addr, mono: u64, name: &str) {
        w.note(addr, mono, unix(mono), name, false, false);
    }

    #[test]
    fn an_unwatched_address_records_nothing() {
        let mut w = Watched::default();
        note(&mut w, v4(1), 10, "ads.example.com");
        assert_eq!(
            (w.len(), w.is_empty(), w.is_watching(v4(1), 10)),
            (0, true, false)
        );
        // Once it opts in, nothing of what it asked before is there.
        let mine = touch(&mut w, v4(1), 20);
        assert_eq!(mine.asked_24h, 0);
        assert!(mine.recent.is_empty());
        // A watched address does not make its neighbour watched.
        note(&mut w, v4(2), 21, "ads.example.com");
        assert_eq!(touch(&mut w, v4(2), 22).asked_24h, 0);
    }

    #[test]
    fn a_watched_address_sees_only_its_own_queries() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 10);
        touch(&mut w, v4(2), 11);
        note(&mut w, v4(1), 12, "a1.example.com");
        note(&mut w, v4(2), 13, "b.example.com");
        note(&mut w, v4(1), 14, "a2.example.com");
        note(&mut w, v4(3), 15, "stranger.example.com");
        let a = touch(&mut w, v4(1), 20);
        let b = touch(&mut w, v4(2), 20);
        assert_eq!((a.asked_24h, b.asked_24h), (2, 1));
        assert_eq!(names(&a), ["a2.example.com", "a1.example.com"]);
        assert_eq!(names(&b), ["b.example.com"]);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn a_query_is_kept_as_it_was_asked() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 10);
        w.note(v4(1), 12, 1_800_000_123, "x.bad.example.net", true, true);
        let mine = touch(&mut w, v4(1), 13);
        assert_eq!(
            mine.recent,
            [Seen {
                unix_secs: 1_800_000_123,
                name: "x.bad.example.net".into(),
                refused: true,
                threat: true,
            }]
        );
    }

    #[test]
    fn the_ring_keeps_the_newest_32_newest_first() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        for n in 0..40u64 {
            note(&mut w, v4(1), n, &format!("n{n}.example.com"));
        }
        let mine = touch(&mut w, v4(1), 40);
        assert_eq!(mine.recent.len(), RING);
        assert_eq!(
            mine.recent.first().map(|s| &*s.name),
            Some("n39.example.com")
        );
        assert_eq!(mine.recent.last().map(|s| &*s.name), Some("n8.example.com"));
        assert_eq!(mine.recent.first().map(|s| s.unix_secs), Some(unix(39)));
        // The counts are of every query, not of the ring.
        assert_eq!(mine.asked_24h, 40);
    }

    #[test]
    fn counts_cover_a_rolling_24_hours() {
        let mut w = Watched::default();
        // The watcher is refreshed every ten minutes, as a page that stays open does.
        let at = |hours: u64| hours * HOUR_SECS;
        for mono in (0..=at(25)).step_by(600) {
            touch(&mut w, v4(1), mono);
            if mono == at(0) || mono == at(25) {
                note(&mut w, v4(1), mono, "example.com");
            }
        }
        // Hour 0 is 25 hours back: out of the window. Only the query of hour 25 is in.
        assert_eq!(touch(&mut w, v4(1), at(25)).asked_24h, 1);

        // The window's edge: a query counts for the hour it was asked in and the 23 after it.
        let mut w = Watched::default();
        let mut mono = 0;
        touch(&mut w, v4(1), mono);
        note(&mut w, v4(1), mono, "example.com");
        let mut counted = Vec::new();
        for hour in [1, 23, 24] {
            while mono < at(hour) {
                mono += 600;
                touch(&mut w, v4(1), mono);
            }
            counted.push(touch(&mut w, v4(1), mono).asked_24h);
        }
        assert_eq!(counted, [1, 1, 0]);
    }

    #[test]
    fn idle_for_15_minutes_is_forgotten_with_its_ring() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        note(&mut w, v4(1), 5, "example.com");
        // Just under the limit it is still watched, and its refresh keeps its start.
        let mine = touch(&mut w, v4(1), IDLE_SECS - 1);
        assert_eq!(mine.watching_since_unix, unix(0));
        assert_eq!(names(&mine), ["example.com"]);
        // 15 minutes after that refresh it is not.
        let mine = touch(&mut w, v4(1), IDLE_SECS - 1 + IDLE_SECS);
        assert_eq!(mine.watching_since_unix, unix(IDLE_SECS - 1 + IDLE_SECS));
        assert!(mine.recent.is_empty());
        assert_eq!(mine.asked_24h, 0);
    }

    #[test]
    fn a_touch_forgets_every_idle_address_not_only_its_own() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        touch(&mut w, v4(2), 100);
        touch(&mut w, v4(3), 950);
        // At 950 the first has been idle for 950 s and is gone; the second, for 850 s, is not.
        assert_eq!(w.len(), 2);
        assert!(
            !w.is_watching(v4(1), 950) && w.is_watching(v4(2), 950) && w.is_watching(v4(3), 950)
        );
        // At 1,849 the second has been idle for 1,749 s, and the third for 899.
        touch(&mut w, v4(3), 950 + IDLE_SECS - 1);
        assert_eq!(w.len(), 1);
        assert!(w.is_watching(v4(3), 950 + IDLE_SECS - 1));
    }

    #[test]
    fn a_query_after_the_idle_limit_is_not_kept_and_ends_the_watch() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        note(&mut w, v4(1), 5, "kept.example.com");
        // The page was closed: nobody touches again, and the query that comes after the limit
        // is not recorded, though no touch has swept the table.
        note(&mut w, v4(1), IDLE_SECS, "late.example.com");
        assert_eq!(w.len(), 0);
        let mine = touch(&mut w, v4(1), IDLE_SECS + 1);
        assert!(mine.recent.is_empty());
        assert_eq!(mine.watching_since_unix, unix(IDLE_SECS + 1));
    }

    #[test]
    fn the_ninth_watcher_evicts_the_least_recently_refreshed() {
        let mut w = Watched::default();
        for n in 1..=MAX_WATCHED as u8 {
            touch(&mut w, v4(n), u64::from(n));
        }
        assert_eq!(w.len(), MAX_WATCHED);
        // The first is refreshed: the second is now the one refreshed least recently.
        touch(&mut w, v4(1), 20);
        touch(&mut w, v4(9), 21);
        assert_eq!(w.len(), MAX_WATCHED);
        assert!(!w.is_watching(v4(2), 21));
        assert!(
            (1..=9)
                .filter(|&n| n != 2)
                .all(|n| w.is_watching(v4(n), 21))
        );
        // A query of the evicted one is not kept, and a refresh of one that is watched adds none.
        note(&mut w, v4(2), 22, "example.com");
        assert_eq!(w.len(), MAX_WATCHED);
        touch(&mut w, v4(5), 23);
        assert_eq!(w.len(), MAX_WATCHED);
        // The next one in line is the third, not the first.
        touch(&mut w, v4(10), 24);
        assert!(w.is_watching(v4(1), 24) && !w.is_watching(v4(3), 24));
    }

    #[test]
    fn refused_and_threat_are_counted_apart() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        w.note(v4(1), 1, unix(1), "fine.example.com", false, false);
        w.note(v4(1), 2, unix(2), "ads.example.com", true, false);
        w.note(v4(1), 3, unix(3), "ads2.example.com", true, false);
        w.note(v4(1), 4, unix(4), "x.bad.example.net", true, true);
        let mine = touch(&mut w, v4(1), 5);
        assert_eq!(
            (mine.asked_24h, mine.refused_24h, mine.threats_24h),
            (4, 3, 1)
        );
        let flags: Vec<_> = mine.recent.iter().map(|s| (s.refused, s.threat)).collect();
        assert_eq!(
            flags,
            [(true, true), (true, false), (true, false), (false, false)]
        );
    }

    #[test]
    fn the_start_is_corrected_when_the_wall_clock_is_set_after_it() {
        // The Pi has no clock of its own: until SNTP has stepped it, the wall clock reads a build
        // epoch, and a device that opts in then is told a start that is far in the past.
        const BUILD_EPOCH: u64 = 1_700_000_000;
        const REAL: u64 = 1_800_000_000;
        let mut w = Watched::default();
        let first = w.touch(v4(1), 5, BUILD_EPOCH);
        assert_eq!(first.watching_since_unix, BUILD_EPOCH);
        // 60 monotonic seconds later the clock has been set: the start moves with it, to where the
        // device opted in on the clock that is right now.
        let later = w.touch(v4(1), 65, REAL);
        assert_eq!(later.watching_since_unix, REAL - 60);
        // And it stays there as the device goes on refreshing.
        let later = w.touch(v4(1), 65 + 600, REAL + 600);
        assert_eq!(later.watching_since_unix, REAL - 60);
    }

    #[test]
    fn an_idle_address_is_forgotten_when_another_asks_dns() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        touch(&mut w, v4(2), 600);
        note(&mut w, v4(1), 10, "private.example.com");
        // Nobody calls `touch` again: the second keeps asking DNS, and by then the first has been
        // idle for 15 minutes. It is gone, with its ring, though it never asked again itself.
        assert!(w.is_watching(v4(2), IDLE_SECS));
        assert_eq!(w.len(), 1);
        assert!(!w.is_watching(v4(1), IDLE_SECS));
        // The one that is still fresh is kept, and so is what it is asked for.
        note(&mut w, v4(2), IDLE_SECS, "kept.example.com");
        assert_eq!(touch(&mut w, v4(2), IDLE_SECS).asked_24h, 1);
        // A client that is no one's, asking once the second has gone idle too, empties the table.
        assert!(!w.is_watching(v4(3), IDLE_SECS + IDLE_SECS));
        assert!(w.is_empty());
    }

    #[test]
    fn reverse_lookups_are_neither_kept_nor_counted() {
        let mut w = Watched::default();
        touch(&mut w, v4(1), 0);
        // A reverse lookup names an address, which no answer may hold: in either zone, in any
        // case, with or without the root dot.
        for name in [
            "5.2.0.192.in-addr.arpa",
            "5.2.0.192.IN-ADDR.ARPA.",
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa",
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.B.D.0.1.0.0.2.Ip6.Arpa.",
        ] {
            w.note(v4(1), 1, unix(1), name, true, false);
        }
        note(&mut w, v4(1), 2, "example.com");
        let mine = touch(&mut w, v4(1), 3);
        assert_eq!(mine.asked_24h, 1);
        assert_eq!((mine.refused_24h, mine.threats_24h), (0, 0));
        assert_eq!(names(&mine), ["example.com"]);
        // A name that only looks like the zone is a name like any other.
        note(&mut w, v4(1), 4, "notin-addr.arpa");
        note(&mut w, v4(1), 5, "in-addr.arpa.example.com");
        assert_eq!(touch(&mut w, v4(1), 6).asked_24h, 3);
    }
}
