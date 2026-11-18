//! What the front panel and the status page show: query counters for today and since start, the
//! most recently blocked name with how often it was blocked in the last 24 hours, and a log of
//! recent queries.
//!
//! Two clocks are kept apart. "Today", the times in the log and the ten-minute history follow
//! the wall clock, which steps at every boot and may step back. The 24-hour count of a blocked
//! name follows the monotonic clock, so a step of the wall clock does not move it.
//!
//! The history is 144 buckets of ten minutes, in a ring keyed by the wall-clock time. Each slot
//! remembers which bucket it counts: a query for another bucket starts the slot over, and a
//! bucket is only ever read at its own place in the window. So after a step of the wall clock in
//! either direction, counts from outside the 24 hours before the time asked for are not shown,
//! and new queries are counted at once.
//!
//! Each tracked name also has its count for the local day, and the [`TOP_NAMES`] names blocked
//! most often that day are kept in a list of their own, [`Stats::top_today`]. A block updates
//! both with a few comparisons and no scan. A change of the local day, in either direction, starts
//! the day's counts over: the list at once, and each name's count when the name is next blocked.
//!
//! Everything is bounded: the log never holds more entries than its capacity, the tracker never
//! more names than its own, each with 24 hourly counts and one for the day, the day's list
//! [`TOP_NAMES`] names, and the history exactly 144 buckets of 40 bytes. Nothing but the history
//! is reserved up front; the rest grows with what is recorded. All arithmetic on counts and times
//! is saturating, or cannot leave its range: no time, offset, name or count makes a call panic or
//! wrap.
//!
//! The names are stored as given, so the bounds in bytes follow the longest name. With the
//! capacities [`Resolver`](crate::Resolver) uses and the names it records, which are up to 1,003
//! bytes in the log and up to 253 in the tracker, the log is about 50 MB at worst (50,000 names,
//! and a few MB for the entries) and the tracker under 1 MB (each name is held twice).

use std::collections::VecDeque;

use crate::Now;
use crate::lru::Lru;

/// The number of queries the log keeps for the status page.
pub const QUERY_LOG_CAPACITY: usize = 50_000;
/// The number of blocked names whose 24-hour counts are kept; the least recently blocked goes.
pub const BLOCKED_NAMES_TRACKED: usize = 1_024;
/// The number of names [`Stats::top_today`] lists.
pub const TOP_NAMES: usize = 8;

/// The seconds one history bucket covers.
pub const BUCKET_SECS: u64 = 600;
/// The number of history buckets: 24 hours of ten-minute ones.
pub const HISTORY_BUCKETS: usize = 144;

/// The seconds in a day of the wall clock.
const DAY_SECS: i64 = 86_400;
/// The seconds in an hour of the monotonic clock.
const HOUR_SECS: u64 = 3_600;
/// The hours a blocked name's count covers: the current one and the 23 before it.
const WINDOW_HOURS: usize = 24;

/// How many queries were answered, and how.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every query recorded, whatever its outcome.
    pub queries: u64,
    /// Queries for a blocked name.
    pub blocked: u64,
    /// Queries for a name that is blocked as a threat: a part of `blocked`, never more than it.
    /// Only [`Stats::record_threat`] counts them, and only in today's counts and the total; a
    /// history bucket leaves it at 0.
    pub blocked_threat: u64,
    /// Queries answered from the cache, fresh or stale.
    pub cache_hits: u64,
    /// Queries sent upstream.
    pub forwarded: u64,
}

impl Counts {
    /// Counts one query with `outcome`. A counter that is full stays full.
    fn count(&mut self, outcome: Outcome) {
        let counter = match outcome {
            Outcome::Blocked => Some(&mut self.blocked),
            Outcome::Cached | Outcome::Stale => Some(&mut self.cache_hits),
            Outcome::Forwarded => Some(&mut self.forwarded),
            Outcome::Local => None,
        };
        if let Some(counter) = counter {
            *counter = counter.saturating_add(1);
        }
        self.queries = self.queries.saturating_add(1);
    }

    /// The sum of `self` and `other`, each counter full at the most.
    fn saturating_add(self, other: Counts) -> Counts {
        Counts {
            queries: self.queries.saturating_add(other.queries),
            blocked: self.blocked.saturating_add(other.blocked),
            blocked_threat: self.blocked_threat.saturating_add(other.blocked_threat),
            cache_hits: self.cache_hits.saturating_add(other.cache_hits),
            forwarded: self.forwarded.saturating_add(other.forwarded),
        }
    }
}

/// The queries of one ten-minute stretch of wall-clock time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryBucket {
    /// The first second of the stretch, since the Unix epoch: a multiple of [`BUCKET_SECS`].
    pub start_unix: u64,
    /// The queries recorded in the stretch.
    pub counts: Counts,
}

/// How a query was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The name is blocked: answered with a null address or an empty success.
    Blocked,
    /// Answered here by a fixed rule, such as `localhost` or a private reverse zone.
    Local,
    /// Answered from the cache, within the answer's lifetime.
    Cached,
    /// Answered from the cache past the answer's lifetime, while upstream is asked again.
    Stale,
    /// Sent upstream.
    Forwarded,
}

/// One query in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// The wall-clock time of the query, in seconds since the Unix epoch.
    pub unix_secs: u64,
    /// The name asked for, as it was recorded.
    pub name: Box<str>,
    /// The record type asked for, as its number.
    pub qtype: u16,
    /// How the query was answered.
    pub outcome: Outcome,
}

/// The most recently blocked name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastBlocked {
    /// The name, as it was recorded.
    pub name: String,
    /// The wall-clock time of its latest block, in seconds since the Unix epoch.
    pub unix_secs: u64,
    /// How often it was blocked in the current monotonic hour and the 23 before it.
    pub count_24h: u32,
}

/// How often one name was blocked in each of the 24 monotonic hours ending at `hour`, and how
/// often in the local day it was last blocked in.
struct HourlyBlocks {
    /// The latest hour in which the name was blocked.
    hour: u64,
    /// The blocks of hour `h` are in bucket `h % 24`, for the 24 hours ending at `hour`. Each
    /// stays at `u16::MAX` once full.
    buckets: [u16; WINDOW_HOURS],
    /// The [`Stats::epoch`] that `today` counts: the local day it was last blocked in.
    epoch: u32,
    /// The blocks in that day. Stays at `u32::MAX` once full.
    today: u32,
}

impl HourlyBlocks {
    /// No blocks yet, as of `hour`, in the day numbered `epoch`.
    fn new(hour: u64, epoch: u32) -> Self {
        Self {
            hour,
            buckets: [0; WINDOW_HOURS],
            epoch,
            today: 0,
        }
    }

    /// Counts one block in the day numbered `epoch` and returns the day's count: the count starts
    /// again at 1 if the name was last blocked in another day.
    fn add_today(&mut self, epoch: u32) -> u32 {
        if self.epoch != epoch {
            (self.epoch, self.today) = (epoch, 0);
        }
        self.today = self.today.saturating_add(1);
        self.today
    }

    /// The bucket that holds the blocks of `hour`.
    fn bucket_of(hour: u64) -> usize {
        // Under 24, so the conversion loses nothing.
        (hour % WINDOW_HOURS as u64) as usize
    }

    /// The buckets as they stand once the clock has reached `hour`: every hour that has passed
    /// since the latest block took over the bucket of the hour 24 before it, which has left the
    /// window, so that bucket is zero. An `hour` that is not after the latest block, which takes
    /// a monotonic clock going back, leaves them as they are.
    fn buckets_at(&self, hour: u64) -> [u16; WINDOW_HOURS] {
        let mut buckets = self.buckets;
        let passed = hour.saturating_sub(self.hour).min(WINDOW_HOURS as u64);
        for step in 1..=passed {
            // `self.hour % 24 + step` is at most 47. The bucket of the hour `step` after the
            // latest block is the one that held the hour 24 before it.
            let left = Self::bucket_of(self.hour % WINDOW_HOURS as u64 + step);
            if let Some(bucket) = buckets.get_mut(left) {
                *bucket = 0;
            }
        }
        buckets
    }

    /// Counts one block at `hour`, or in the latest hour seen if `hour` is before it.
    fn add(&mut self, hour: u64) {
        self.buckets = self.buckets_at(hour);
        self.hour = self.hour.max(hour);
        if let Some(bucket) = self.buckets.get_mut(Self::bucket_of(self.hour)) {
            *bucket = bucket.saturating_add(1);
        }
    }

    /// The blocks in `hour` and the 23 hours before it.
    fn count_at(&self, hour: u64) -> u32 {
        // 24 buckets of at most 65,535 each: far below `u32::MAX`.
        self.buckets_at(hour).iter().map(|&n| u32::from(n)).sum()
    }
}

/// One name of the most blocked names of the local day.
#[derive(Clone, PartialEq, Eq)]
pub struct TopName {
    /// The name, as it was recorded.
    pub name: Box<str>,
    /// How often it was blocked today.
    pub count: u32,
}

impl core::fmt::Debug for TopName {
    /// The count only: the name is something the house asked for.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TopName")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

/// The counters, the most recently blocked name, the ten-minute history and the log of recent
/// queries.
pub struct Stats {
    /// Local time minus UTC, in seconds.
    utc_offset: i32,
    /// The local day of the latest record, as days since the epoch; `None` before the first.
    day: Option<i64>,
    /// Which stretch of one local day `today` and the blocked names' day counts belong to: it
    /// goes up every time `day` changes, so that a wall clock that steps back to a day it has
    /// left does not bring that day's old counts back.
    epoch: u32,
    /// The counts of `day`.
    today: Counts,
    /// The counts since start.
    total: Counts,
    /// The ten-minute history. The bucket that starts at `s` has the slot `s / 600 % 144`, and the
    /// slot's `start_unix` says which bucket it holds; a slot that has never been used holds the
    /// empty bucket that starts at 0.
    history: [HistoryBucket; HISTORY_BUCKETS],
    /// The most entries `log` may hold.
    log_capacity: usize,
    /// The latest queries, oldest first.
    log: VecDeque<LogEntry>,
    /// The hourly blocks of the names blocked most recently.
    blocked: Lru<Box<str>, HourlyBlocks>,
    /// The name blocked last and the wall-clock time of that block.
    last_blocked: Option<(Box<str>, u64)>,
    /// The names blocked most often in the local day of the latest record, most often first; of
    /// two with the same count, the one that reached it first. At most [`TOP_NAMES`].
    top: Vec<TopName>,
}

impl Stats {
    /// Empty statistics that log the last `log_capacity` queries and keep 24-hour counts for
    /// `tracked_names` blocked names. With a `log_capacity` of 0 no query is logged; a
    /// `tracked_names` of 0 is treated as 1. Only the history's 144 buckets, about 6 KB, are
    /// reserved up front.
    pub fn new(log_capacity: usize, tracked_names: usize) -> Self {
        Self {
            utc_offset: 0,
            day: None,
            epoch: 0,
            today: Counts::default(),
            total: Counts::default(),
            history: [HistoryBucket::default(); HISTORY_BUCKETS],
            log_capacity,
            log: VecDeque::new(),
            blocked: Lru::new(tracked_names),
            last_blocked: None,
            top: Vec::new(),
        }
    }

    /// Sets local time minus UTC in seconds, which places the midnight where "today" starts
    /// again. It is 0 until set, and applies from the next call on: the day of the latest
    /// record is not worked out again.
    pub fn set_utc_offset(&mut self, secs: i32) {
        self.utc_offset = secs;
    }

    /// Records one answered query: counts it, logs it and, if it was blocked, makes `name` the
    /// most recently blocked one. `name` is stored as given. The history counts it in the bucket
    /// of `now.unix_secs`; if the slot of that bucket holds another bucket, an earlier one or a
    /// later one that a wall clock now gone back had reached, the slot starts over first.
    pub fn record(&mut self, name: &str, qtype: u16, outcome: Outcome, now: Now) {
        self.roll_day(now);
        self.today.count(outcome);
        self.total.count(outcome);
        self.record_history(outcome, now);

        if outcome == Outcome::Blocked {
            let hour = now.mono_secs / HOUR_SECS;
            let key: Box<str> = name.into();
            let epoch = self.epoch;
            let count = match self.blocked.get_mut(&key) {
                Some(blocks) => {
                    blocks.add(hour);
                    blocks.add_today(epoch)
                }
                None => {
                    let mut blocks = HourlyBlocks::new(hour, epoch);
                    blocks.add(hour);
                    let count = blocks.add_today(epoch);
                    // Whichever name this evicts was blocked least recently.
                    self.blocked.insert(key.clone(), blocks);
                    count
                }
            };
            self.note_top(&key, count);
            self.last_blocked = Some((key, now.unix_secs));
        }

        if self.log_capacity == 0 {
            return;
        }
        if self.log.len() >= self.log_capacity {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            unix_secs: now.unix_secs,
            name: name.into(),
            qtype,
            outcome,
        });
    }

    /// Puts one more block of `name` in the list of the most blocked names of the day. `count` is
    /// the name's count for the day, which the tracker has just worked out.
    ///
    /// A few comparisons and, only when `name` enters the list, one copy of it: the per-block
    /// copy of `record` already allocates more than this. A name that is on the list is counted
    /// there, so one that the tracker lost to a newer name and began again does not fall back.
    fn note_top(&mut self, name: &str, count: u32) {
        let mut at = if let Some(at) = self.top.iter().position(|top| &*top.name == name) {
            if let Some(top) = self.top.get_mut(at) {
                top.count = top.count.saturating_add(1).max(count);
            }
            at
        } else if self.top.len() < TOP_NAMES {
            self.top.push(TopName {
                name: name.into(),
                count,
            });
            self.top.len().saturating_sub(1)
        } else {
            let last = self.top.len().saturating_sub(1);
            match self.top.last_mut() {
                // Only a name that beats the least of them takes its place.
                Some(least) if count > least.count => {
                    *least = TopName {
                        name: name.into(),
                        count,
                    };
                    last
                }
                _ => return,
            }
        };
        // Up past every name that it now beats; a name it ties stays ahead of it.
        while let Some(before) = at.checked_sub(1) {
            let beats = matches!(
                (self.top.get(before), self.top.get(at)),
                (Some(ahead), Some(this)) if ahead.count < this.count
            );
            if !beats {
                break;
            }
            self.top.swap(before, at);
            at = before;
        }
    }

    /// The names blocked most often in the local day of `now`, most often first, at most
    /// [`TOP_NAMES`] of them: empty if that is another day than the latest record's, as
    /// [`today`](Self::today) is. A copy of eight names at most, with no scan.
    ///
    /// The counts are those of the names the tracker holds ([`BLOCKED_NAMES_TRACKED`]): a name
    /// that the tracker dropped for newer ones and that was then blocked again counts only from
    /// that block, unless it was on the list, which counts it itself.
    pub fn top_today(&self, now: Now) -> Vec<TopName> {
        if self.day == Some(self.local_day(now)) {
            self.top.clone()
        } else {
            Vec::new()
        }
    }

    /// Counts the query that [`record`](Self::record) has just recorded as a block of a threat
    /// name too, in today's counts and the total; a counter that is full stays full. The
    /// history, the log and the blocked-name counts are left as they are.
    ///
    /// Call it right after `record`, with the same `now`: the day has been rolled over by then,
    /// so the count lands in the day of the query, and rolling it over again changes nothing.
    pub fn record_threat(&mut self, now: Now) {
        self.roll_day(now);
        self.today.blocked_threat = self.today.blocked_threat.saturating_add(1);
        self.total.blocked_threat = self.total.blocked_threat.saturating_add(1);
    }

    /// The counts of the local day of `now`: zero if that is another day than the latest
    /// record's.
    pub fn today(&self, now: Now) -> Counts {
        if self.day == Some(self.local_day(now)) {
            self.today
        } else {
            Counts::default()
        }
    }

    /// The counts since start.
    pub fn total(&self) -> Counts {
        self.total
    }

    /// The most recently blocked name, or `None` if nothing was blocked yet. Its count is as
    /// of `now` on the monotonic clock.
    pub fn last_blocked(&self, now: Now) -> Option<LastBlocked> {
        let (name, unix_secs) = self.last_blocked.as_ref()?;
        // The name blocked last is the most recently used one, so it is still tracked.
        let count_24h = self
            .blocked
            .peek(name)
            .map_or(0, |blocks| blocks.count_at(now.mono_secs / HOUR_SECS));
        Some(LastBlocked {
            name: String::from(&**name),
            unix_secs: *unix_secs,
            count_24h,
        })
    }

    /// The 24 hours before `now` as [`HISTORY_BUCKETS`] buckets of ten minutes, oldest first: the
    /// last is the one that holds `now.unix_secs`. A bucket shows what was recorded in it and
    /// nothing else, so one nothing was recorded in, or whose slot has since been used for
    /// another bucket, is empty. A bucket that would start before the epoch is empty too, and
    /// starts at 0.
    pub fn history(&self, now: Now) -> Vec<HistoryBucket> {
        self.window(now).collect()
    }

    /// The sum of [`history`](Self::history): the queries in the bucket that holds `now.unix_secs`
    /// and the 143 before it.
    pub fn last_24h(&self, now: Now) -> Counts {
        self.window(now).fold(Counts::default(), |sum, bucket| {
            sum.saturating_add(bucket.counts)
        })
    }

    /// The buckets of the 24-hour window ending with the one that holds `now.unix_secs`, oldest
    /// first, as [`history`](Self::history) describes them.
    fn window(&self, now: Now) -> impl Iterator<Item = HistoryBucket> + '_ {
        let current = now.unix_secs / BUCKET_SECS;
        (0..HISTORY_BUCKETS as u64).rev().map(move |back| {
            let Some(bucket) = current.checked_sub(back) else {
                return HistoryBucket::default();
            };
            let start_unix = Self::start_of(bucket);
            self.history
                .get(Self::slot_of(bucket))
                .filter(|slot| slot.start_unix == start_unix)
                .copied()
                .unwrap_or(HistoryBucket {
                    start_unix,
                    counts: Counts::default(),
                })
        })
    }

    /// The first second of the bucket numbered `bucket`, counted from the epoch.
    fn start_of(bucket: u64) -> u64 {
        // Every bucket used here is a `unix_secs / 600` or below one, so its start is at most
        // `u64::MAX` and the product never saturates; this only rules out a panic or a wrap.
        bucket.saturating_mul(BUCKET_SECS)
    }

    /// The slot of the history that the bucket numbered `bucket`, counted from the epoch, uses.
    fn slot_of(bucket: u64) -> usize {
        // Under 144, so the conversion loses nothing.
        (bucket % HISTORY_BUCKETS as u64) as usize
    }

    /// Counts one query with `outcome` in the bucket of `now.unix_secs`. A slot that holds any
    /// other bucket starts over for this one. Queries are recorded at the current time, so a slot
    /// that holds a later bucket holds counts from the clock's future, which no window ending at
    /// `now` shows: keeping them would hold the history at zero until the clock caught up.
    fn record_history(&mut self, outcome: Outcome, now: Now) {
        let bucket = now.unix_secs / BUCKET_SECS;
        let start_unix = Self::start_of(bucket);
        let Some(slot) = self.history.get_mut(Self::slot_of(bucket)) else {
            return;
        };
        if slot.start_unix != start_unix {
            *slot = HistoryBucket {
                start_unix,
                counts: Counts::default(),
            };
        }
        slot.counts.count(outcome);
    }

    /// The latest logged queries, newest first: at most `n` of them.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &LogEntry> {
        self.log.iter().rev().take(n)
    }

    /// Starts today's counts over if the local day of `now` is not the day they are of. A step
    /// of the wall clock can land on an earlier day as well as a later one.
    fn roll_day(&mut self, now: Now) {
        let day = self.local_day(now);
        if self.day != Some(day) {
            self.day = Some(day);
            self.today = Counts::default();
            // The names' day counts are of the day that has ended, and the list with them.
            self.epoch = self.epoch.wrapping_add(1);
            self.top.clear();
        }
    }

    /// The day `now` falls on in local time, as days since the epoch; negative for a local time
    /// before it. A wall clock beyond `i64::MAX` seconds counts as that many.
    fn local_day(&self, now: Now) -> i64 {
        i64::try_from(now.unix_secs)
            .unwrap_or(i64::MAX)
            .saturating_add(i64::from(self.utc_offset))
            .div_euclid(DAY_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    fn at(mono_secs: u64, unix_secs: u64) -> Now {
        Now {
            mono_secs,
            unix_secs,
        }
    }

    /// 20,833 days after the epoch.
    const MIDNIGHT_UTC: u64 = 1_799_971_200;

    const ALL_OUTCOMES: [Outcome; 5] = [
        Outcome::Blocked,
        Outcome::Local,
        Outcome::Cached,
        Outcome::Stale,
        Outcome::Forwarded,
    ];

    #[test]
    fn counts_follow_outcomes() {
        let mut s = Stats::new(100, 10);
        for o in ALL_OUTCOMES {
            s.record("example.com", 1, o, at(5, MIDNIGHT_UTC));
        }
        let expected = Counts {
            queries: 5,
            blocked: 1,
            blocked_threat: 0,
            cache_hits: 2,
            forwarded: 1,
        };
        assert_eq!(
            (s.today(at(6, MIDNIGHT_UTC + 1)), s.total()),
            (expected, expected)
        );
    }

    #[test]
    fn blocked_threat_rolls_over_with_the_day() {
        let mut s = Stats::new(100, 10);
        s.record("bad.example.net", 1, Outcome::Blocked, at(1, MIDNIGHT_UTC));
        s.record_threat(at(1, MIDNIGHT_UTC));
        s.record("ads.example.com", 1, Outcome::Blocked, at(2, MIDNIGHT_UTC));
        let counted = Counts {
            queries: 2,
            blocked: 2,
            blocked_threat: 1,
            ..Counts::default()
        };
        assert_eq!(
            (s.today(at(2, MIDNIGHT_UTC)), s.total()),
            (counted, counted)
        );
        // The next local day: today starts over, the total keeps its threat.
        let next_day = MIDNIGHT_UTC + 86_400;
        s.record("example.com", 1, Outcome::Forwarded, at(3, next_day));
        assert_eq!(s.today(at(3, next_day)).blocked_threat, 0);
        assert_eq!(s.total().blocked_threat, 1);
        // The history never holds the threat count.
        assert_eq!(s.last_24h(at(2, MIDNIGHT_UTC)).blocked_threat, 0);
    }

    #[test]
    fn the_threat_counter_saturates() {
        let mut s = Stats::new(1, 1);
        s.record("bad.example.net", 1, Outcome::Blocked, at(1, 1));
        s.today.blocked_threat = u64::MAX;
        s.total.blocked_threat = u64::MAX;
        s.record_threat(at(1, 1));
        assert_eq!(s.today(at(1, 1)).blocked_threat, u64::MAX);
        assert_eq!(s.total().blocked_threat, u64::MAX);
    }

    #[test]
    fn today_rolls_over_at_local_midnight() {
        for (offset, boundary) in [
            (0, MIDNIGHT_UTC),
            (3600, MIDNIGHT_UTC - 3600),
            (-18_000, MIDNIGHT_UTC + 18_000),
        ] {
            let mut s = Stats::new(100, 10);
            s.set_utc_offset(offset);
            for _ in 0..3 {
                s.record("example.com", 1, Outcome::Forwarded, at(1, boundary - 1));
            }
            assert_eq!(s.today(at(1, boundary - 1)).queries, 3);
            // Before any query of the new day.
            assert_eq!(s.today(at(2, boundary)), Counts::default());
            s.record("example.com", 1, Outcome::Blocked, at(2, boundary));
            assert_eq!(
                s.today(at(2, boundary)),
                Counts {
                    queries: 1,
                    blocked: 1,
                    blocked_threat: 0,
                    cache_hits: 0,
                    forwarded: 0
                }
            );
            assert_eq!(s.total().queries, 4);
        }
    }

    /// Review Focus 4.
    #[test]
    fn clock_steps_reset_today_without_underflow() {
        let mut s = Stats::new(100, 10);
        s.set_utc_offset(-39_600);
        // The image's build time, then the time after SNTP.
        s.record(
            "ads.example.com",
            1,
            Outcome::Blocked,
            at(10, 1_700_000_000),
        );
        s.record(
            "ads.example.com",
            1,
            Outcome::Blocked,
            at(11, 1_800_000_000),
        );
        assert_eq!(
            (s.today(at(11, 1_800_000_000)).queries, s.total().queries),
            (1, 2)
        );
        // A step back.
        s.record(
            "ads.example.com",
            1,
            Outcome::Blocked,
            at(12, 1_700_000_000),
        );
        assert_eq!(
            (s.today(at(12, 1_700_000_000)).queries, s.total().queries),
            (1, 3)
        );
        // The monotonic clock did not move.
        assert_eq!(s.last_blocked(at(12, 0)).unwrap().count_24h, 3);
        // The epoch, with a negative offset.
        s.record("ads.example.com", 1, Outcome::Blocked, at(13, 0));
        assert_eq!(s.today(at(13, 0)).queries, 1);
    }

    #[test]
    fn last_blocked_counts_the_last_24_hours() {
        let mut s = Stats::new(100, 10);
        assert_eq!(s.last_blocked(at(0, 0)), None);
        for _ in 0..3 {
            s.record("ads.example.com", 1, Outcome::Blocked, at(0, 5_000));
        }
        for _ in 0..2 {
            s.record("ads.example.com", 28, Outcome::Blocked, at(18_000, 23_000));
        }
        s.record("example.com", 1, Outcome::Forwarded, at(18_001, 23_001));
        let last = |s: &Stats, mono| s.last_blocked(at(mono, 0)).unwrap();
        assert_eq!(
            last(&s, 18_001),
            LastBlocked {
                name: "ads.example.com".into(),
                unix_secs: 23_000,
                count_24h: 5
            }
        );
        // Hour 23: hours 0..=23.
        assert_eq!(last(&s, 86_399).count_24h, 5);
        // Hour 24: hours 1..=24.
        assert_eq!(last(&s, 86_400).count_24h, 2);
        // Hour 29.
        assert_eq!(last(&s, 104_400).count_24h, 0);
        s.record("ads.example.com", 1, Outcome::Blocked, at(104_400, 99_000));
        // Hour 5's bucket is reused, not added to.
        assert_eq!(last(&s, 104_400).count_24h, 1);
        s.record(
            "tracker.example.net",
            1,
            Outcome::Blocked,
            at(104_401, 99_001),
        );
        assert_eq!(
            last(&s, 104_401),
            LastBlocked {
                name: "tracker.example.net".into(),
                unix_secs: 99_001,
                count_24h: 1
            }
        );
    }

    #[test]
    fn tracked_names_are_bounded() {
        let mut s = Stats::new(100, 2);
        for name in [
            "a.example.com",
            "b.example.com",
            "c.example.com",
            "a.example.com",
        ] {
            s.record(name, 1, Outcome::Blocked, at(1, 1));
        }
        assert_eq!(
            s.last_blocked(at(1, 1)).unwrap(),
            LastBlocked {
                name: "a.example.com".into(),
                unix_secs: 1,
                count_24h: 1
            }
        );
        assert_eq!(s.blocked.len(), 2);
    }

    /// The most blocked names of the day of `unix_secs`, as `(name, count)`.
    fn top(s: &Stats, unix_secs: u64) -> Vec<(String, u32)> {
        s.top_today(at(1, unix_secs))
            .into_iter()
            .map(|top| (top.name.into(), top.count))
            .collect()
    }

    /// Blocks `name` `times` times at the wall clock `unix_secs`.
    fn block(s: &mut Stats, name: &str, times: u32, unix_secs: u64) {
        for _ in 0..times {
            s.record(name, 1, Outcome::Blocked, at(1, unix_secs));
        }
    }

    #[test]
    fn the_top_is_ordered_by_count_and_a_tie_by_who_got_there_first() {
        let mut s = Stats::new(100, 10);
        assert!(top(&s, MIDNIGHT_UTC).is_empty());
        block(&mut s, "a.example.com", 2, MIDNIGHT_UTC);
        block(&mut s, "b.example.com", 1, MIDNIGHT_UTC);
        s.record("c.example.com", 1, Outcome::Forwarded, at(1, MIDNIGHT_UTC));
        assert_eq!(
            top(&s, MIDNIGHT_UTC),
            [("a.example.com".into(), 2), ("b.example.com".into(), 1)]
        );
        // b catches up with a, and stays behind it: a was at 2 first.
        block(&mut s, "b.example.com", 1, MIDNIGHT_UTC);
        assert_eq!(
            top(&s, MIDNIGHT_UTC),
            [("a.example.com".into(), 2), ("b.example.com".into(), 2)]
        );
        // And passes it by going on.
        block(&mut s, "b.example.com", 1, MIDNIGHT_UTC);
        block(&mut s, "c.example.com", 3, MIDNIGHT_UTC);
        assert_eq!(
            top(&s, MIDNIGHT_UTC),
            [
                ("b.example.com".into(), 3),
                ("c.example.com".into(), 3),
                ("a.example.com".into(), 2)
            ]
        );
        // The names are as recorded and the count is the day's, however it is spelled elsewhere.
        assert_eq!(s.top_today(at(1, MIDNIGHT_UTC))[0].count, 3);
    }

    #[test]
    fn the_top_holds_eight_and_a_name_must_beat_the_last_to_enter() {
        let mut s = Stats::new(100, 100);
        for (i, count) in [5, 4, 4, 3, 3, 3, 2, 2].into_iter().enumerate() {
            block(&mut s, &format!("n{i}.example.com"), count, MIDNIGHT_UTC);
        }
        let full = top(&s, MIDNIGHT_UTC);
        assert_eq!(full.len(), TOP_NAMES);
        // A ninth name that only ties the last stays out, however often it is blocked up to that.
        block(&mut s, "late.example.com", 2, MIDNIGHT_UTC);
        assert_eq!(top(&s, MIDNIGHT_UTC), full);
        // One more block beats it: the name that held the last place goes, and the newcomer takes
        // its place in the order, behind the names that were at 3 before it and ahead of the 2.
        block(&mut s, "late.example.com", 1, MIDNIGHT_UTC);
        let after = top(&s, MIDNIGHT_UTC);
        assert_eq!(after.len(), TOP_NAMES);
        assert_eq!(after[5], ("n5.example.com".into(), 3));
        assert_eq!(after[6], ("late.example.com".into(), 3));
        assert_eq!(after[7], ("n6.example.com".into(), 2));
        assert!(!after.iter().any(|(name, _)| name == "n7.example.com"));
        // A name that came in under the least of them never makes the list.
        block(&mut s, "low.example.com", 1, MIDNIGHT_UTC);
        assert_eq!(top(&s, MIDNIGHT_UTC), after);
    }

    #[test]
    fn the_top_starts_over_with_the_day_in_either_direction() {
        let next = MIDNIGHT_UTC + 86_400;
        let mut s = Stats::new(100, 10);
        block(&mut s, "a.example.com", 3, MIDNIGHT_UTC);
        assert_eq!(top(&s, MIDNIGHT_UTC), [("a.example.com".into(), 3)]);
        // Before any query of the new day the list is the old day's, and not today's.
        assert!(top(&s, next).is_empty());
        block(&mut s, "b.example.com", 1, next);
        block(&mut s, "a.example.com", 1, next);
        assert_eq!(
            top(&s, next),
            [("b.example.com".into(), 1), ("a.example.com".into(), 1)]
        );
        // A name that was blocked on the day before counts from 1 again, not from 3.
        assert_eq!(s.total().blocked, 5);
        // The wall clock steps back to the first day: that day's counts are not brought back.
        block(&mut s, "a.example.com", 1, MIDNIGHT_UTC);
        assert_eq!(top(&s, MIDNIGHT_UTC), [("a.example.com".into(), 1)]);
        assert!(top(&s, next).is_empty());
        // The 24-hour count follows the monotonic clock and was not touched by any of it.
        assert_eq!(s.last_blocked(at(1, 0)).unwrap().count_24h, 5);
    }

    #[test]
    fn the_top_follows_the_local_day() {
        let mut s = Stats::new(100, 10);
        s.set_utc_offset(-18_000);
        // Five hours behind UTC: 05:00 UTC is midnight where the clock is.
        let midnight_local = MIDNIGHT_UTC + 18_000;
        block(&mut s, "a.example.com", 1, midnight_local - 1);
        block(&mut s, "a.example.com", 1, midnight_local - 100);
        assert_eq!(top(&s, midnight_local - 1), [("a.example.com".into(), 2)]);
        block(&mut s, "a.example.com", 1, midnight_local);
        assert_eq!(top(&s, midnight_local), [("a.example.com".into(), 1)]);
    }

    #[test]
    fn a_name_the_tracker_dropped_does_not_fall_back_on_the_list() {
        // The tracker holds two names; the list holds up to eight.
        let mut s = Stats::new(100, 2);
        block(&mut s, "a.example.com", 5, MIDNIGHT_UTC);
        block(&mut s, "b.example.com", 1, MIDNIGHT_UTC);
        // Evicts a, whose count the tracker loses.
        block(&mut s, "c.example.com", 2, MIDNIGHT_UTC);
        assert_eq!(s.blocked.len(), 2);
        assert!(s.blocked.peek(&"a.example.com".into()).is_none());
        // The list still has a with its 5; blocked again it is 6, not 1.
        block(&mut s, "a.example.com", 1, MIDNIGHT_UTC);
        assert_eq!(
            top(&s, MIDNIGHT_UTC),
            [
                ("a.example.com".into(), 6),
                ("c.example.com".into(), 2),
                ("b.example.com".into(), 1)
            ]
        );
        // The tracker holds a again, from this block.
        assert_eq!(
            s.blocked.peek(&"a.example.com".into()).map(|b| b.today),
            Some(1)
        );
    }

    #[test]
    fn day_counts_saturate() {
        let mut blocks = HourlyBlocks::new(0, 7);
        blocks.today = u32::MAX - 1;
        assert_eq!(blocks.add_today(7), u32::MAX);
        assert_eq!(blocks.add_today(7), u32::MAX);
        // Another day starts it over.
        assert_eq!(blocks.add_today(8), 1);
    }

    #[test]
    fn a_top_name_shows_its_count_and_not_its_name_in_debug() {
        let top = TopName {
            name: "secret.example.org".into(),
            count: 12,
        };
        let shown = format!("{top:?}");
        assert!(shown.contains("12") && !shown.contains("secret"), "{shown}");
    }

    /// What the day's top adds to each blocked answer, which is `note_top` and one more counter:
    /// the list full, for a name that is not on it, and for the names that are. Run in release:
    /// `cargo test -p qb-dns --release the_days_top_costs -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing test for release builds; see the comment above it"]
    fn the_days_top_costs_little_per_blocked_answer() {
        use std::hint::black_box;
        use std::time::Instant;

        const CALLS: usize = 20_000_000;
        const ROUNDS: usize = 5;
        const OTHERS: usize = 4_096;

        let mut s = Stats::new(0, BLOCKED_NAMES_TRACKED);
        let on_list: Vec<String> = (0..TOP_NAMES)
            .map(|i| format!("top{i}.ads.example.com"))
            .collect();
        for (i, name) in on_list.iter().enumerate() {
            block(&mut s, name, 1_000 + i as u32, MIDNIGHT_UTC);
        }
        assert_eq!(s.top.len(), TOP_NAMES);
        let others: Vec<String> = (0..OTHERS)
            .map(|i| format!("t{i}.tracker.example.net"))
            .collect();
        let per_call = |s: &mut Stats, names: &[String], count: u32| {
            let mut fastest = f64::MAX;
            for _ in 0..ROUNDS {
                let started = Instant::now();
                for i in 0..CALLS / ROUNDS {
                    s.note_top(black_box(&names[i % names.len()]), black_box(count));
                }
                fastest =
                    fastest.min(started.elapsed().as_nanos() as f64 / (CALLS / ROUNDS) as f64);
            }
            fastest
        };
        let full = s.top.clone();
        let miss = per_call(&mut s, &others, 1);
        // Nothing changed: a name under the last of the list never enters it.
        assert_eq!(s.top, full);
        // The name with the most blocks is the first of the list.
        let leader = [on_list[TOP_NAMES - 1].clone()];
        let first = per_call(&mut s, &leader, 1);
        let each = per_call(&mut s, &on_list, 1);
        println!(
            "note_top, list full: a name that is not on it {miss:.1} ns, the first of the list {first:.1} ns, \
             each of the list in turn {each:.1} ns"
        );
    }

    /// Random blocks of a dozen names over three local days, with a clock that steps back now and
    /// then, checked against plain counts: the list is the eight names with the most blocks that
    /// day, a tie going to the one that got there first. The tracker is wide enough to lose no
    /// name.
    #[test]
    fn the_top_matches_plain_counts_over_random_blocks() {
        const NAMES: usize = 12;
        let mut rng = Rng::new(11);
        let mut s = Stats::new(8, 64);
        // For each name: its blocks in the current day and the step of the last of them.
        let mut model: Vec<(u32, u64)> = vec![(0, 0); NAMES];
        let mut day = 0u64;
        let (mut entered, mut rolled, mut full) = (0, 0, 0);
        for step in 1..=30_000u64 {
            // Mostly the same day, now and then the next, and once in a while one before.
            match rng.below(1_000) {
                0..=2 => day += 1,
                3 => day = day.saturating_sub(1),
                _ => {}
            }
            let unix = MIDNIGHT_UTC + day * 86_400 + 100;
            let before = top(&s, unix);
            // The counts start over when a record is made in another day than the last one's.
            if s.day != Some(i64::try_from(unix / 86_400).unwrap()) {
                model = vec![(0, 0); NAMES];
                rolled += 1;
            }
            // The first names are blocked most often.
            let i = rng.below(NAMES).min(rng.below(NAMES));
            let name = format!("n{i}.example.com");
            if rng.below(5) == 0 {
                s.record(&name, 1, Outcome::Forwarded, at(1, unix));
            } else {
                s.record(&name, 1, Outcome::Blocked, at(1, unix));
                model[i] = (model[i].0 + 1, step);
            }
            let mut want: Vec<(usize, (u32, u64))> = model
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, m)| m.0 > 0)
                .collect();
            want.sort_by_key(|&(_, (count, at))| (std::cmp::Reverse(count), at));
            let want: Vec<(String, u32)> = want
                .into_iter()
                .take(TOP_NAMES)
                .map(|(i, (count, _))| (format!("n{i}.example.com"), count))
                .collect();
            let got = top(&s, unix);
            entered += usize::from(before != got);
            assert_eq!(got, want, "step {step}");
            full += usize::from(got.len() == TOP_NAMES);
        }
        assert!(
            entered > 1_000 && rolled > 50 && full > 10_000,
            "{entered} changes, {rolled} resets, {full} full lists"
        );
    }

    #[test]
    fn log_is_a_bounded_ring_newest_first() {
        let mut s = Stats::new(3, 2);
        for i in 1..=5u64 {
            s.record(
                &format!("n{i}.example.com"),
                1,
                Outcome::Forwarded,
                at(i, 100 + i),
            );
        }
        let names = |n| s.recent(n).map(|e| &*e.name).collect::<Vec<_>>();
        assert_eq!(
            names(10),
            ["n5.example.com", "n4.example.com", "n3.example.com"]
        );
        assert_eq!(names(2), ["n5.example.com", "n4.example.com"]);
        assert!(names(0).is_empty());
        assert_eq!(
            s.recent(1).next().unwrap(),
            &LogEntry {
                unix_secs: 105,
                name: "n5.example.com".into(),
                qtype: 1,
                outcome: Outcome::Forwarded
            }
        );
        assert_eq!(s.log.len(), 3);
    }

    #[test]
    fn zero_capacities_keep_no_log_and_one_name() {
        let mut s = Stats::new(0, 0);
        s.record("a.example.com", 1, Outcome::Blocked, at(1, 1));
        s.record("b.example.com", 1, Outcome::Blocked, at(2, 2));
        s.record("example.com", 1, Outcome::Forwarded, at(3, 3));
        assert_eq!((s.recent(10).count(), s.log.capacity()), (0, 0));
        assert_eq!((s.total().queries, s.today(at(3, 3)).blocked), (3, 2));
        let last = |name: &str, unix_secs| LastBlocked {
            name: name.into(),
            unix_secs,
            count_24h: 1,
        };
        assert_eq!(s.last_blocked(at(3, 3)), Some(last("b.example.com", 2)));
        // The one tracked name was `b`, so `a` starts again from nothing.
        s.record("a.example.com", 1, Outcome::Blocked, at(4, 4));
        assert_eq!(s.last_blocked(at(4, 4)), Some(last("a.example.com", 4)));
        assert_eq!(s.blocked.len(), 1);
    }

    #[test]
    fn huge_capacities_reserve_nothing_up_front() {
        let mut s = Stats::new(usize::MAX, usize::MAX);
        assert_eq!(s.log.capacity(), 0);
        for o in ALL_OUTCOMES {
            s.record("example.com", 1, o, at(1, 1));
        }
        assert_eq!(s.recent(usize::MAX).count(), 5);
        assert_eq!(s.last_blocked(at(1, 1)).unwrap().count_24h, 1);
    }

    #[test]
    fn the_log_keeps_names_as_given() {
        let long = "x".repeat(5_000);
        let mut s = Stats::new(2, 2);
        for name in ["", "Ads.Example.COM.", "a\\.b.example.com", &long] {
            s.record(name, 65_535, Outcome::Blocked, at(1, 1));
            assert_eq!(&*s.recent(1).next().unwrap().name, name);
            assert_eq!(s.last_blocked(at(1, 1)).unwrap().name, name);
        }
    }

    #[test]
    fn extreme_times_and_offsets_do_not_panic() {
        let far = i64::MAX as u64;
        let times = [
            at(0, 0),
            at(u64::MAX, u64::MAX),
            // The monotonic clock going back, which it should never do.
            at(0, far),
            at(u64::MAX, far + 1),
            at(86_400, 1),
            at(u64::MAX - 1, 0),
        ];
        for offset in [i32::MIN, -86_400, -1, 0, 1, 86_400, i32::MAX] {
            let mut s = Stats::new(4, 2);
            s.set_utc_offset(offset);
            for (i, now) in times.into_iter().enumerate() {
                s.record("ads.example.com", 1, Outcome::Blocked, now);
                assert!(s.today(now).queries >= 1);
                assert_eq!(s.total().blocked, i as u64 + 1);
                assert_eq!(s.last_blocked(now).unwrap().unix_secs, now.unix_secs);
            }
            // The first block is over 24 hours before the second. Every later one, whatever its
            // own time, is counted in the latest hour seen, and reading at an earlier hour
            // takes nothing away.
            for now in times {
                assert_eq!(s.last_blocked(now).unwrap().count_24h, 5);
            }
        }
    }

    #[test]
    fn a_monotonic_clock_going_back_keeps_the_buckets() {
        let hour = |h: u64| at(h * 3_600, 0);
        let mut s = Stats::new(4, 2);
        let block = |s: &mut Stats, h| s.record("ads.example.com", 1, Outcome::Blocked, hour(h));
        block(&mut s, 30);
        block(&mut s, 40);
        // Hour 2 is before both: it counts in hour 40, the latest seen.
        block(&mut s, 2);
        let count = |s: &Stats, h| s.last_blocked(hour(h)).unwrap().count_24h;
        assert_eq!((count(&s, 2), count(&s, 40), count(&s, 53)), (3, 3, 3));
        // Hour 30 leaves the window at hour 54, hour 40 with its two blocks at hour 64.
        assert_eq!((count(&s, 54), count(&s, 63), count(&s, 64)), (2, 2, 0));
    }

    #[test]
    fn counters_and_buckets_saturate() {
        const FULL: Counts = Counts {
            queries: u64::MAX,
            blocked: u64::MAX,
            blocked_threat: u64::MAX,
            cache_hits: u64::MAX,
            forwarded: u64::MAX,
        };
        let mut s = Stats::new(1, 1);
        s.record("ads.example.com", 1, Outcome::Blocked, at(1, 1));
        (s.today, s.total) = (FULL, FULL);
        for o in ALL_OUTCOMES {
            s.record("ads.example.com", 1, o, at(1, 1));
        }
        assert_eq!((s.today(at(1, 1)), s.total()), (FULL, FULL));
        for _ in 0..70_000 {
            s.record("ads.example.com", 1, Outcome::Blocked, at(1, 1));
        }
        assert_eq!(
            s.last_blocked(at(1, 1)).unwrap().count_24h,
            u32::from(u16::MAX)
        );
        assert_eq!(s.log.len(), 1);
    }

    /// Random blocks of a few names on a clock that only moves forward, checked against a plain
    /// list of the hours at which each tracked name was blocked.
    #[test]
    fn count_24h_matches_a_list_of_block_hours() {
        const NAMES: [&str; 6] = [
            "a.example.com",
            "b.example.com",
            "c.example.com",
            "a.example.net",
            "b.example.net",
            "c.example.net",
        ];
        const TRACKED: usize = 4;
        let mut rng = Rng::new(9);
        let mut s = Stats::new(8, TRACKED);
        // The tracked names, least recently blocked first, each with its block hours.
        let mut model: Vec<(&str, Vec<u64>)> = Vec::new();
        // The clock starts anywhere.
        let mut mono = rng.next() >> 16;
        let (mut evictions, mut lapsed, mut highest) = (0, 0, 0);
        for _ in 0..20_000 {
            // Mostly under ten minutes between queries, now and then up to 30 hours.
            mono += match rng.below(100) {
                0 => rng.below(108_000) as u64,
                _ => rng.below(600) as u64,
            };
            // The first names are asked for most often, so the last ones come and go.
            let name = NAMES[rng.below(NAMES.len()).min(rng.below(NAMES.len()))];
            if rng.below(4) == 0 {
                s.record(name, 1, Outcome::Forwarded, at(mono, 7));
            } else {
                s.record(name, 1, Outcome::Blocked, at(mono, mono ^ 1));
                let mut hours = match model.iter().position(|&(n, _)| n == name) {
                    Some(i) => model.remove(i).1,
                    None if model.len() == TRACKED => {
                        model.remove(0);
                        evictions += 1;
                        Vec::new()
                    }
                    None => Vec::new(),
                };
                hours.push(mono / 3_600);
                model.push((name, hours));
            }
            // Read now, or up to 30 hours later.
            let read = mono
                + match rng.below(3) {
                    0 => 0,
                    _ => rng.below(108_000) as u64,
                };
            let Some((name, hours)) = model.last() else {
                assert_eq!(s.last_blocked(at(read, 0)), None);
                continue;
            };
            let read_hour = read / 3_600;
            let expected = hours.iter().filter(|&&h| read_hour - h < 24).count() as u32;
            let got = s.last_blocked(at(read, 0)).unwrap();
            assert_eq!((&*got.name, got.count_24h), (*name, expected));
            lapsed += usize::from(expected == 0);
            highest = highest.max(expected);
        }
        // The run must have evicted names, read counts that had lapsed, and read high ones.
        assert!(
            evictions > 500 && lapsed > 100 && highest > 20,
            "{evictions} evictions, {lapsed} lapsed, highest {highest}"
        );
    }

    /// The ten-minute history, with a clock that is given as wall-clock seconds alone.
    mod history {
        use super::*;

        fn at(unix_secs: u64) -> Now {
            Now {
                mono_secs: 1,
                unix_secs,
            }
        }

        /// A multiple of 600.
        const T: u64 = 1_800_000_000;

        #[test]
        fn buckets_are_ten_minutes_oldest_first_and_end_with_the_current_one() {
            let mut s = Stats::new(16, 16);
            s.record("a.example.com", 1, Outcome::Forwarded, at(T + 5));
            s.record("b.example.com", 1, Outcome::Blocked, at(T + 599));
            s.record("c.example.com", 1, Outcome::Cached, at(T + 600));
            let h = s.history(at(T + 700));
            assert_eq!(h.len(), HISTORY_BUCKETS);
            assert_eq!(
                (h[143].start_unix, h[142].start_unix, h[0].start_unix),
                (T + 600, T, T + 600 - 143 * 600)
            );
            assert_eq!(
                h[142].counts,
                Counts {
                    queries: 2,
                    blocked: 1,
                    blocked_threat: 0,
                    cache_hits: 0,
                    forwarded: 1
                }
            );
            assert_eq!(
                h[143].counts,
                Counts {
                    queries: 1,
                    blocked: 0,
                    blocked_threat: 0,
                    cache_hits: 1,
                    forwarded: 0
                }
            );
            assert_eq!(
                s.last_24h(at(T + 700)),
                Counts {
                    queries: 3,
                    blocked: 1,
                    blocked_threat: 0,
                    cache_hits: 1,
                    forwarded: 1
                }
            );
        }

        #[test]
        fn a_bucket_leaves_the_window_after_144_more() {
            let mut s = Stats::new(16, 16);
            s.record("a.example.com", 1, Outcome::Forwarded, at(T));
            assert_eq!(s.last_24h(at(T + 143 * 600)).queries, 1);
            assert_eq!(s.last_24h(at(T + 144 * 600)).queries, 0);
        }

        /// Review Focus 4.
        #[test]
        fn the_clock_stepping_forward_or_back_never_counts_outside_the_window() {
            let mut s = Stats::new(16, 16);
            // At the build time.
            s.record("a.example.com", 1, Outcome::Forwarded, at(T));
            // SNTP stepped the clock a year on.
            let later = T + 365 * 86_400;
            s.record("b.example.com", 1, Outcome::Forwarded, at(later));
            assert_eq!(s.last_24h(at(later)).queries, 1);
            // And back an hour.
            s.record("c.example.com", 1, Outcome::Forwarded, at(later - 3_600));
            assert_eq!(s.last_24h(at(later)).queries, 2);
            // Asked from before them all: nothing from its future.
            assert_eq!(s.last_24h(at(later - 7_200)).queries, 0);
            // A clock near zero does not underflow.
            assert_eq!(s.history(at(0)).len(), HISTORY_BUCKETS);
        }

        #[test]
        fn a_slot_starts_over_for_any_other_bucket() {
            let mut s = Stats::new(16, 16);
            let same_slot = T + HISTORY_BUCKETS as u64 * BUCKET_SECS;
            s.record("a.example.com", 1, Outcome::Forwarded, at(T));
            s.record("b.example.com", 1, Outcome::Blocked, at(same_slot));
            let h = s.history(at(same_slot));
            assert_eq!(h[143].start_unix, same_slot);
            assert_eq!(h[143].counts.blocked, 1);
            assert_eq!(s.last_24h(at(same_slot)).queries, 1);
            // The clock goes back a day, to the earlier bucket whose slot holds the later one:
            // the slot starts over for the earlier bucket, and its new traffic is counted.
            s.record("c.example.com", 1, Outcome::Forwarded, at(T + 1));
            assert_eq!(s.last_24h(at(T + 1)).queries, 1);
            // What the later bucket had counted was from the clock's future, and is gone.
            assert_eq!(s.last_24h(at(same_slot)).queries, 0);
            assert_eq!(s.total().queries, 3);
        }

        #[test]
        fn after_a_step_back_new_traffic_is_counted_at_once() {
            // From a day to a year, with every slot holding a bucket from the future.
            for back in [86_400, 90_000, 365 * 86_400] {
                let mut s = Stats::new(16, 16);
                for i in 0..HISTORY_BUCKETS as u64 {
                    s.record(
                        "a.example.com",
                        1,
                        Outcome::Forwarded,
                        at(T + back + i * BUCKET_SECS),
                    );
                }
                let end = T + (HISTORY_BUCKETS as u64 - 1) * BUCKET_SECS;
                // None of it is in the window ending at `end`.
                assert_eq!(s.last_24h(at(end)).queries, 0, "back {back}");
                for i in 0..HISTORY_BUCKETS as u64 {
                    s.record(
                        "b.example.com",
                        1,
                        Outcome::Blocked,
                        at(T + i * BUCKET_SECS),
                    );
                    assert_eq!(
                        s.last_24h(at(T + i * BUCKET_SECS)).queries,
                        i + 1,
                        "back {back}"
                    );
                }
                let h = s.history(at(end));
                assert!(h.iter().all(|b| b.counts.blocked == 1), "back {back}");
                assert_eq!(s.last_24h(at(end)).queries, HISTORY_BUCKETS as u64);
            }
        }

        #[test]
        fn a_clock_near_the_epoch_reports_empty_buckets_before_it() {
            let mut s = Stats::new(16, 16);
            s.record("a.example.com", 1, Outcome::Forwarded, at(0));
            s.record("b.example.com", 1, Outcome::Forwarded, at(599));
            s.record("c.example.com", 1, Outcome::Forwarded, at(600));
            let h = s.history(at(599));
            assert_eq!(h.len(), HISTORY_BUCKETS);
            assert!(h[..143].iter().all(|b| *b == HistoryBucket::default()));
            assert_eq!((h[143].start_unix, h[143].counts.queries), (0, 2));
            let h = s.history(at(600));
            assert!(h[..142].iter().all(|b| *b == HistoryBucket::default()));
            assert_eq!(h[142], s.history(at(599))[143]);
            assert_eq!((h[143].start_unix, h[143].counts.queries), (600, 1));
            assert_eq!(s.last_24h(at(600)).queries, 3);
            // Queries never recorded: empty, and every bucket still where it belongs.
            let h = Stats::new(0, 0).history(at(1_000));
            let starts: Vec<u64> = h[141..].iter().map(|b| b.start_unix).collect();
            assert_eq!(starts, [0, 0, 600]);
        }

        #[test]
        fn the_far_end_of_the_clock_does_not_overflow() {
            let last = u64::MAX / BUCKET_SECS * BUCKET_SECS;
            let mut s = Stats::new(16, 16);
            s.record("a.example.com", 1, Outcome::Forwarded, at(0));
            s.record("b.example.com", 1, Outcome::Blocked, at(u64::MAX));
            let h = s.history(at(u64::MAX));
            assert_eq!(h.len(), HISTORY_BUCKETS);
            assert_eq!((h[143].start_unix, h[143].counts.blocked), (last, 1));
            assert_eq!(h[0].start_unix, last - 143 * BUCKET_SECS);
            assert_eq!(s.last_24h(at(u64::MAX)).queries, 1);
            assert_eq!(s.last_24h(at(0)).queries, 1);
            // Asked from the epoch: the record at the far end is nowhere in the window.
            assert_eq!(s.history(at(0))[143].counts.blocked, 0);
        }

        #[test]
        fn counts_saturate_in_the_sum() {
            const FULL: Counts = Counts {
                queries: u64::MAX,
                blocked: u64::MAX,
                blocked_threat: u64::MAX,
                cache_hits: u64::MAX,
                forwarded: u64::MAX,
            };
            let mut s = Stats::new(0, 0);
            for (i, unix_secs) in [T, T + 600].into_iter().enumerate() {
                s.record("a.example.com", 1, Outcome::Forwarded, at(unix_secs));
                let slot = Stats::slot_of(unix_secs / BUCKET_SECS);
                s.history[slot].counts = FULL;
                assert_eq!(s.last_24h(at(T + 600)).queries, u64::MAX, "bucket {i}");
            }
            s.record("b.example.com", 1, Outcome::Blocked, at(T + 600));
            assert_eq!(s.last_24h(at(T + 600)), FULL);
        }
    }
}
