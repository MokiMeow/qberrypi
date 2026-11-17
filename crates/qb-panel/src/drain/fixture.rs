//! Fixed scenes for the goldens and the preview: the stills of the round 6 mockup.
//!
//! Each is a Drain run forward from a fixed start and the figures that go with it, made to look like
//! the still of its name in `round6/` of the mockups (`scenes.js`, with the shot defaults of
//! `common.js`: the clock at 10:21, 4,780 refused, 44 %, and 4:37 of a pause left). Round 8 changes them in
//! one place: the lane's first four cells, x 0 to 96, say `HOME` (spec 3.3), and no name shows there. The
//! scenes are the same moments of the same runs, with what stood in those cells gone from view.
//!
//! The names are public ad domains, `example.org` and `example.com`. Where the mockup shows a name that is
//! not on its public list of ads, the name here is one of the same length, so the lane's geometry is the
//! mockup's.
//!
//! The mockup's funnel in the busy stills is its `fill(146)` and then the landings of its seed-7
//! traffic: `Drain::prefill`, and then the same runs landed one after another. Only the lengths of the
//! runs are kept (`NORMAL_LANDINGS`, `PAUSED_LANDINGS`), found by running the mockup's own code; the
//! names that made them are not reproduced, and a name that is landed later in a still lands as usual.

use super::{Drain, NameKind, Stills};
use crate::format::gate_fill;
use crate::state::{Blocking, LaneWords, Share, TimeOfDay};

/// Which still to make; each is named for the mockup's still it reproduces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// Blocking is on, a 15-letter refused name is melting at the gate, and a plain one has gone on ahead.
    Normal,
    /// Blocking is paused, with 4:37 left, and a listed name crosses the gate outlined.
    Paused,
    /// Wi-Fi not joined: the lane says so, the funnel is empty and the figures are dashes.
    Boot,
    /// Joined, and no query yet: the lane says it is listening and the count is a true 0.
    Listening,
    /// No traffic for 47 minutes: the lane says so, over a full funnel.
    Quiet,
    /// A name longer than the lane has stopped at the gate, fitted to its 15 cells.
    Long,
    /// The normal scene on a busier day: a five-digit count.
    Five,
    /// The normal scene on a busier day: a six-digit count.
    Six,
    /// Nothing refused yet: `0%`, an empty funnel and a name passing.
    Pct0,
    /// Everything refused so far, three names: `ALL`, and three runs in the funnel.
    Pct100,
}

impl Fixture {
    /// Every fixture, in the order of the mockup's page.
    pub const ALL: [Fixture; 10] = [
        Fixture::Normal,
        Fixture::Paused,
        Fixture::Boot,
        Fixture::Listening,
        Fixture::Quiet,
        Fixture::Long,
        Fixture::Five,
        Fixture::Six,
        Fixture::Pct0,
        Fixture::Pct100,
    ];

    /// The mockup's name for the still: `normal`, `quiet`, and so on. A golden is `drain-<name>.png`.
    pub const fn name(self) -> &'static str {
        match self {
            Fixture::Normal => "normal",
            Fixture::Paused => "paused",
            Fixture::Boot => "boot",
            Fixture::Listening => "listening",
            Fixture::Quiet => "quiet",
            Fixture::Long => "long",
            Fixture::Five => "five",
            Fixture::Six => "six",
            Fixture::Pct0 => "pct0",
            Fixture::Pct100 => "pct100",
        }
    }
}

/// How many frames a fixture runs at most before it takes the scene as it is.
const FRAME_LIMIT: usize = 400;

/// The seed of the mockup's traffic, which is also the seed of the funnel's prefill.
const SEED: u32 = 7;

/// The share of today's queries that were blocked, in the stills.
const PERCENT: u8 = 44;

/// The time of day in the stills.
const CLOCK: TimeOfDay = TimeOfDay {
    hour: 10,
    minute: 21,
};

/// The count in the stills.
const COUNT: u32 = 4780;

/// The seconds left of the pause in the paused still.
const PAUSE_LEFT_SECS: u16 = 277;

/// The length of the pause the paused still is in: five minutes, the mockup's.
const PAUSE_TOTAL_SECS: u16 = 300;

/// The minutes the quiet still has been quiet for.
const QUIET_MINUTES: u32 = 47;

/// The refusal step the normal still stops at: the melt has reached the 11th letter, which sags, while
/// the 6th to 10th are puddles and the 5 next to the gate are slumped.
const NORMAL_STEP: u8 = 5;

/// The step at which the long name has just stopped: every tile blue, the door shut, nothing melting yet.
const LONG_STEP: u8 = 2;

/// The lengths of the runs the mockup's traffic landed in its funnel before the normal stills: after the
/// prefill, from dot 61 on. The shade goes on from the prefill's last.
const NORMAL_LANDINGS: [usize; 6] = [12, 15, 15, 10, 9, 15];

/// The lengths of the runs it landed before the paused still, in whose last 180 frames nothing was refused.
const PAUSED_LANDINGS: [usize; 4] = [12, 15, 15, 10];

/// The refused name of the normal stills: `rubiconproject.com` on the lane's 15 cells, `rubiconproject~`.
const REFUSED: &str = "rubiconproject.com";

/// The plain name that has gone on ahead of the refused one: only its last letters, `com`, are still on the screen.
const AHEAD: &str = "example.com";

/// The name of the paused still that has gone on ahead: only its last letters, `rg`, are still on the screen.
const AHEAD_PAUSED: &str = "example.org";

/// The listed name of the paused still, crossing the gate.
const CROSSING: &str = "amazon-adsystem.com";

/// The Drain and the figures of one of the stills.
pub fn drain_fixture(which: Fixture) -> (Drain, Stills) {
    match which {
        Fixture::Normal => refusing(COUNT, Share::Percent(PERCENT)),
        Fixture::Paused => paused(),
        Fixture::Boot => boot(),
        Fixture::Listening => listening(),
        Fixture::Quiet => quiet(),
        Fixture::Long => long(),
        Fixture::Five => refusing(12_480, Share::Percent(PERCENT)),
        Fixture::Six => refusing(104_780, Share::Percent(37)),
        Fixture::Pct0 => pct0(),
        Fixture::Pct100 => pct100(),
    }
}

/// The figures every still shares: the share, the clock, the count and nothing else.
fn stills(percent: Option<Share>, count: Option<u32>) -> Stills {
    Stills {
        percent,
        clock: Some(CLOCK),
        blocking: Blocking::On,
        gate_fill: 0,
        count,
        gate_ask: false,
        words: None,
    }
}

/// A Drain with a full funnel, as the mockup's `fill(146)` and then the runs of its traffic left it, with
/// the landed dots at rest.
fn funnel_of_the_day(runs: &[usize]) -> Drain {
    let mut drain = Drain::new(SEED);
    drain.prefill(SEED);
    for &run in runs {
        drain.land(run);
    }
    // Two frames turn the landed dots over, and they are at rest.
    for _ in 0..2 {
        drain.tick(false);
    }
    drain
}

/// Ticks until the Drain says it is done, or the frame limit runs out.
fn tick_until(drain: &mut Drain, paused: bool, done: impl Fn(&Drain) -> bool) {
    for _ in 0..FRAME_LIMIT {
        if done(drain) {
            break;
        }
        drain.tick(paused);
    }
}

/// A refused name stopped at the gate and melting, with a plain one that has gone on ahead of it (under `HOME`
/// by now), over a full funnel; `count` is what the day's figure and the count show.
fn refusing(count: u32, percent: Share) -> (Drain, Stills) {
    let mut drain = funnel_of_the_day(&NORMAL_LANDINGS);
    drain.offer(AHEAD, NameKind::Plain);
    drain.offer(REFUSED, NameKind::Refused);
    // One refused name is in flight, so the day's figure is one more than the count it holds back.
    drain.set_blocked_today(Some(count + 1));
    tick_until(&mut drain, false, |drain| {
        drain.refusal_step() == Some(NORMAL_STEP)
    });
    (drain, stills(Some(percent), Some(count)))
}

/// A listed name crossing the gate while it is paused, outlined, with a plain one gone on ahead of it.
fn paused() -> (Drain, Stills) {
    let mut drain = funnel_of_the_day(&PAUSED_LANDINGS);
    drain.set_paused(true);
    drain.set_blocked_today(Some(COUNT));
    drain.offer(AHEAD_PAUSED, NameKind::Plain);
    drain.offer(CROSSING, NameKind::Listed);
    // The listed name's first letter has crossed the gate, into the first cell past it: that is under `HOME`
    // now, so the moment is read from the model.
    tick_until(&mut drain, true, |drain| {
        drain
            .names
            .iter()
            .any(|name| name.kind == NameKind::Listed && name.head == 16)
    });
    let blocking = Blocking::Paused {
        left_secs: PAUSE_LEFT_SECS,
    };
    let still = Stills {
        blocking,
        gate_fill: gate_fill(PAUSE_LEFT_SECS, PAUSE_TOTAL_SECS),
        ..stills(Some(Share::Percent(PERCENT)), Some(COUNT))
    };
    (drain, still)
}

/// A name longer than the lane, refused, in the step it has stopped in, over a full funnel.
fn long() -> (Drain, Stills) {
    let mut drain = Drain::new(SEED);
    drain.prefill(SEED);
    drain.offer("pagead2.googlesyndication.com", NameKind::Refused);
    drain.set_blocked_today(Some(COUNT + 1));
    tick_until(&mut drain, false, |drain| {
        drain.refusal_step() == Some(LONG_STEP)
    });
    (drain, stills(Some(Share::Percent(PERCENT)), Some(COUNT)))
}

/// The first second: Wi-Fi is not joined, so nothing is known and the lane says so.
fn boot() -> (Drain, Stills) {
    let still = Stills {
        clock: None,
        words: Some(LaneWords::Joining),
        ..stills(None, None)
    };
    (Drain::new(SEED), still)
}

/// Joined, the clock set, and no query yet: the count is a true 0 and the share is still unknown.
fn listening() -> (Drain, Stills) {
    let mut drain = Drain::new(SEED);
    drain.set_blocked_today(Some(0));
    let still = Stills {
        words: Some(LaneWords::Listening),
        ..stills(None, Some(0))
    };
    (drain, still)
}

/// Queries came today and none for 47 minutes: the lane is idle and says so, over a full funnel.
fn quiet() -> (Drain, Stills) {
    let mut drain = Drain::new(SEED);
    drain.prefill(SEED);
    drain.set_blocked_today(Some(COUNT));
    let still = Stills {
        words: Some(LaneWords::Quiet {
            minutes: QUIET_MINUTES,
        }),
        ..stills(Some(Share::Percent(PERCENT)), Some(COUNT))
    };
    (drain, still)
}

/// Nothing refused yet today: `0%`, an empty funnel, and a plain name that has passed the gate.
fn pct0() -> (Drain, Stills) {
    let mut drain = Drain::new(SEED);
    drain.set_blocked_today(Some(0));
    // 14 letters, as the mockup's `api.github.com`.
    drain.offer("my.example.com", NameKind::Plain);
    // The name's first letter is in cell 17, left of the gate, as the mockup stops it: under `HOME` now, so
    // the moment is read from the model.
    tick_until(&mut drain, false, |drain| {
        drain.names.iter().any(|name| name.head == 17)
    });
    (drain, stills(Some(Share::Zero), Some(0)))
}

/// Three queries so far, all three refused and landed: runs of 9, 15 and 10 dots in the three shades
/// from the tip, the lane idle with nothing to say.
fn pct100() -> (Drain, Stills) {
    let mut drain = Drain::new(SEED);
    for name in ["adnxs.com", "doubleclick.net", "criteo.com"] {
        drain.offer(name, NameKind::Refused);
    }
    drain.set_blocked_today(Some(3));
    tick_until(&mut drain, false, |drain| {
        drain.idle() && !drain.moving() && drain.count() == Some(3)
    });
    (drain, stills(Some(Share::All), Some(3)))
}

#[cfg(test)]
mod tests {
    use super::super::{Melt, Shade, TileStyle};
    use super::*;

    #[test]
    fn the_normal_still_stops_a_refused_name_at_its_melt() {
        let (drain, stills) = drain_fixture(Fixture::Normal);
        assert_eq!(drain.refusal_step(), Some(NORMAL_STEP));
        assert!(drain.door_shut());
        // `rubiconproject~` melts: 5 tiles slump, 5 puddle and 5 sag. The plain name that went ahead of it is
        // under `HOME`, and has left the lane.
        let melt = |melt| drain.lane().filter(|tile| tile.melt == melt).count();
        assert_eq!(
            (melt(Melt::Slump), melt(Melt::Puddle), melt(Melt::Sag)),
            (5, 5, 5)
        );
        assert!(drain.lane().all(|tile| tile.style == TileStyle::Refused));
        assert_eq!(drain.lane().count(), 15);
        assert_eq!(drain.names.len(), 1, "the name ahead has left the lane");
        assert_eq!(drain.count(), Some(COUNT), "one refused name is held back");
        assert_eq!(stills.count, drain.count());
        assert!(!stills.gate_ask);
        assert_eq!(stills.words, None);
    }

    #[test]
    fn the_paused_still_has_a_listed_name_crossing_the_gate_and_none_ahead() {
        let (drain, stills) = drain_fixture(Fixture::Paused);
        assert_eq!(stills.blocking, Blocking::Paused { left_secs: 277 });
        assert_eq!(stills.gate_fill, 1);
        let at = |style| {
            drain
                .lane()
                .filter(|tile| tile.style == style)
                .map(|tile| tile.cell)
                .collect::<Vec<_>>()
        };
        assert!(
            at(TileStyle::Plain).is_empty(),
            "the name ahead is under `HOME`, and gone"
        );
        assert_eq!(
            at(TileStyle::Listed),
            (0..16).collect::<Vec<_>>(),
            "`mazon-adsystem.c` is on the screen, from cell 15 to cell 0; its first letter is under `HOME`"
        );
        assert_eq!(drain.refusal_step(), None);
        assert!(!drain.door_shut());
    }

    #[test]
    fn the_long_still_has_just_stopped_a_name_fitted_to_fifteen_cells() {
        let (drain, _) = drain_fixture(Fixture::Long);
        assert_eq!(drain.refusal_step(), Some(LONG_STEP));
        assert!(drain.door_shut());
        let letters: String = {
            let mut tiles: Vec<_> = drain.lane().collect();
            tiles.sort_by_key(|tile| std::cmp::Reverse(tile.cell));
            tiles.iter().map(|tile| tile.ch).collect()
        };
        assert_eq!(letters, "googlesyndicat~");
        assert!(
            drain
                .lane()
                .all(|tile| tile.style == TileStyle::Refused && tile.melt == Melt::Whole)
        );
    }

    #[test]
    fn boot_listening_and_quiet_say_what_they_are() {
        let (drain, boot) = drain_fixture(Fixture::Boot);
        assert!(drain.idle());
        assert_eq!((boot.percent, boot.clock, boot.count), (None, None, None));
        assert_eq!(boot.words, Some(LaneWords::Joining));
        assert!(drain.funnel().iter().all(|cell| cell.shade.is_none()));
        let (drain, listening) = drain_fixture(Fixture::Listening);
        assert!(drain.idle());
        assert_eq!((listening.percent, listening.count), (None, Some(0)));
        assert_eq!(listening.clock, Some(CLOCK));
        assert_eq!(listening.words, Some(LaneWords::Listening));
        let (drain, quiet) = drain_fixture(Fixture::Quiet);
        assert!(drain.idle());
        assert_eq!(quiet.words, Some(LaneWords::Quiet { minutes: 47 }));
        assert!(drain.funnel().iter().all(|cell| cell.shade.is_some()));
    }

    #[test]
    fn the_busy_stills_have_their_figures() {
        let (_, five) = drain_fixture(Fixture::Five);
        assert_eq!(
            (five.count, five.percent),
            (Some(12_480), Some(Share::Percent(44)))
        );
        let (_, six) = drain_fixture(Fixture::Six);
        assert_eq!(
            (six.count, six.percent),
            (Some(104_780), Some(Share::Percent(37)))
        );
    }

    #[test]
    fn the_percent_stills_show_their_forms() {
        let (drain, zero) = drain_fixture(Fixture::Pct0);
        assert_eq!((zero.percent, zero.count), (Some(Share::Zero), Some(0)));
        assert!(drain.funnel().iter().all(|cell| cell.shade.is_none()));
        assert_eq!(
            drain.lane().count(),
            12,
            "the name has come in whole, 14 letters from cell 17: two are under `HOME`"
        );
        let (drain, all) = drain_fixture(Fixture::Pct100);
        assert_eq!((all.percent, all.count), (Some(Share::All), Some(3)));
        assert_eq!(all.words, None);
        assert!(drain.idle() && !drain.moving());
        // Runs of 9, 15 and 10 dots from the tip, in the three shades in turn.
        let lit = |shade| {
            drain
                .funnel()
                .iter()
                .filter(|cell| cell.shade == Some(shade))
                .count()
        };
        assert_eq!(
            (lit(Shade::Blue), lit(Shade::Mid), lit(Shade::Dark)),
            (9, 15, 10)
        );
        assert_eq!(
            drain
                .funnel()
                .iter()
                .filter(|cell| cell.shade.is_some())
                .count(),
            34
        );
    }
}
