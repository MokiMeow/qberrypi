//! LIVE (spec §3.3): the Drain as one section of the OS. The Drain draws itself, with its own painter, its
//! dirty rows and its budget; the lane's first four cells say `HOME` there, and the gate ask, the gate's
//! choices unfolded over its field, is one of its layers (`Stills::gate_ask`). What is here is the glue the
//! OS needs of it: where its taps are, which is the one entry point `nav::decide` calls for LIVE.
//!
//! The taps are the mockup's `tapOS` live branch (`screens.js`), in its order:
//!
//! - **With the gate ask open** any press ends it. A press in the keys' band, `y >= 212` and `x >= 124` (the
//!   three 110 by 80 keys at `128 + 117 i`, y 224, with a margin), opens the gate for that key's time as it
//!   ends it: 5, 30 or 60 minutes, by `floor((x - 124) / 117)`, the last going on to the edge. Anywhere else
//!   only ends it. It also folds away by itself after 8 s (`nav::age_ask`).
//! - **The key** (`KEY_TARGET`): with the gate open it shuts it at once; shut, it unfolds the ask.
//! - **The lane**, `y < 72`: `x < 108` goes HOME; the rest is the Drain's lane of names, which is no door.
//! - **The left column** under the lane: the share (`y < 148`) is a door to NAMES, and the clock, `y >= 148`
//!   down to the key, to SYSTEM. **The funnel and the count** (`x >= 120`) are doors to NAMES too.
//!
//! NAMES does not exist in Phase 1 (ruling R1), so its three doors are inert; SYSTEM is the ABOUT reading.
//! Strong actions are unguarded, and the gate is not one (spec §7.1).

use crate::geometry::Rect;
use crate::os::intent::{GateMove, Intent};
use crate::os::nav::{Change, Nav, Section, Tap};
use crate::os::screens::parts::GATE_CHOICES;
use crate::os::world::World;
use crate::ui::look::{LANE_HOME_X, LANE_TARGET_Y};

/// Where a press works the Drain's key: the key and a margin round it, `(0, 212, 96, 108)` (the mockup's
/// `KEY_TARGET`).
pub const KEY_TARGET: Rect = Rect {
    x: 0,
    y: 212,
    w: 96,
    h: 108,
};

/// The first row of the gate ask's keys' band: the keys' top, 224, less a margin.
pub const ASK_HIT_Y: u16 = 212;

/// The first column of the gate ask's keys' band: the first key's left edge, 128, less a margin.
pub const ASK_HIT_X: u16 = 124;

/// The width of one key's band: the distance between two keys' left edges.
pub const ASK_HIT_PITCH: u16 = 117;

/// The share's rows end here, in the left column: from this row the clock is the column's.
pub const SHARE_END_Y: u16 = 148;

/// The left column's width: `x < 96` is the share, the clock and the key, and the gate stands from there.
pub const LEFT_COLUMN_W: u16 = 96;

/// The first column of the field, right of the gate: the funnel and the count are doors from here.
pub const FIELD_X: u16 = 120;

/// For how long the key at column `x` of the ask's band opens the gate, in seconds, for a press at `(x, y)`;
/// `None` outside the band. The third key's band goes on to the screen's edge.
pub fn ask_secs(x: u16, y: u16) -> Option<u16> {
    if y < ASK_HIT_Y || x < ASK_HIT_X {
        return None;
    }
    let band = usize::from((x - ASK_HIT_X) / ASK_HIT_PITCH);
    let last = GATE_CHOICES.len().saturating_sub(1);
    GATE_CHOICES.get(band.min(last)).map(|&(_, secs)| secs)
}

/// What a press at `(x, y)` would do while LIVE shows, with nothing above it. `nav::decide` calls it.
pub fn decide(nav: &Nav, world: &World, x: u16, y: u16) -> Tap {
    if nav.gate_ask() {
        // The ask takes every press: a key picks, the rest only fold it away.
        return match ask_secs(x, y) {
            Some(secs) => Tap::Act(
                Intent::Gate(GateMove::Open(secs)),
                Some(Change::CloseGateAsk),
            ),
            None => Tap::Nav(Change::CloseGateAsk),
        };
    }
    if KEY_TARGET.contains(x, y) {
        return if world.gate_open() {
            Tap::Act(Intent::Gate(GateMove::Shut), None)
        } else {
            Tap::Nav(Change::OpenGateAsk)
        };
    }
    if y < LANE_TARGET_Y {
        return if x < LANE_HOME_X {
            Tap::Nav(Change::Go(Section::Home))
        } else {
            Tap::None
        };
    }
    if x < LEFT_COLUMN_W {
        // The share is a door to NAMES, which Phase 1 has not; the clock is SYSTEM's.
        return if y < SHARE_END_Y {
            names_door()
        } else {
            Tap::Nav(Change::Go(Section::System))
        };
    }
    if x >= FIELD_X {
        return names_door();
    }
    // The gate's own column, between the two.
    Tap::None
}

/// The share's, the funnel's and the count's door to NAMES: inert until NAMES exists (ruling R1).
fn names_door() -> Tap {
    Tap::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drain::look::{
        ASK_FIELD, ASK_KEY_H, ASK_KEY_PITCH, ASK_KEY_W, ASK_KEY_X0, ASK_KEY_Y, ASK_KEYS,
    };
    use crate::os::fixture::{Still, world};
    use crate::os::nav::{GATE_ASK_MS, age_ask, tap};
    use crate::os::world::Pause;

    /// LIVE with the gate shut, and its world.
    fn live() -> (Nav, World) {
        let world = world(Still::Live);
        (Still::Live.nav(&world), world)
    }

    /// LIVE with the gate ask open.
    fn asking() -> (Nav, World) {
        let world = world(Still::LiveGate);
        (Still::LiveGate.nav(&world), world)
    }

    /// The world with the main gate open, 277 s of 300 left.
    fn open_gate(mut world: World) -> World {
        world.blocking = Some(Pause {
            left_secs: 277,
            total_secs: 300,
        });
        world
    }

    #[test]
    fn the_key_geometry_is_the_drains_and_the_mockups() {
        // The key's target holds the key (8, 224, 80, 80) with a margin, and reaches the screen's foot.
        let key = (8_u16, 224_u16, 80_u16, 80_u16);
        assert!(KEY_TARGET.x == 0 && KEY_TARGET.y <= key.1);
        assert!(KEY_TARGET.x + KEY_TARGET.w >= key.0 + key.2);
        assert_eq!(KEY_TARGET.y + KEY_TARGET.h, 320);
        // The ask's bands are the keys with a margin of 4 left and 3 right, three across.
        assert_eq!(f32::from(ASK_HIT_X), ASK_KEY_X0 - 4.0);
        assert_eq!(f32::from(ASK_HIT_PITCH), ASK_KEY_PITCH);
        assert_eq!(f32::from(ASK_HIT_Y), ASK_KEY_Y - 12.0);
        for i in 0..3_u16 {
            let (left, right) = (
                ASK_KEY_X0 as u16 + i * ASK_HIT_PITCH,
                ASK_KEY_X0 as u16 + i * ASK_HIT_PITCH + ASK_KEY_W as u16 - 1,
            );
            for x in [left, right] {
                assert_eq!(
                    ask_secs(x, ASK_KEY_Y as u16),
                    Some(GATE_CHOICES[usize::from(i)].1),
                    "key {i} at {x}"
                );
                assert_eq!(
                    ask_secs(x, ASK_KEY_Y as u16 + ASK_KEY_H as u16 - 1),
                    Some(GATE_CHOICES[usize::from(i)].1)
                );
            }
        }
        // The keys stand inside the field the ask claims, down to the floor.
        const { assert!(ASK_KEY_Y + ASK_KEY_H <= ASK_FIELD.y + ASK_FIELD.h) };
        assert_eq!(
            ASK_KEYS.map(String::from),
            GATE_CHOICES.map(|(word, _)| word.to_string()),
            "the keys the Drain draws are the gate's choices"
        );
    }

    #[test]
    fn the_ask_bands_are_the_mockups_floor_of_the_x_minus_124_over_117() {
        // `y >= 212 && x >= 124`, band `min(2, floor((x - 124) / 117))`.
        for (x, secs) in [
            (124, 300),
            (240, 300),
            (241, 1_800),
            (357, 1_800),
            (358, 3_600),
            (479, 3_600),
        ] {
            for y in [212, 224, 263, 319] {
                assert_eq!(ask_secs(x, y), Some(secs), "({x}, {y})");
            }
        }
        for (x, y) in [
            (123, 280),
            (0, 280),
            (300, 211),
            (300, 100),
            (479, 0),
            (95, 300),
        ] {
            assert_eq!(ask_secs(x, y), None, "({x}, {y})");
        }
    }

    #[test]
    fn the_key_unfolds_the_ask_when_the_gate_is_shut_and_shuts_the_gate_when_it_is_open() {
        let (mut nav, world) = live();
        assert!(!nav.gate_ask());
        for (x, y) in [(0, 212), (50, 260), (95, 319), (8, 224)] {
            assert_eq!(
                decide(&nav, &world, x, y),
                Tap::Nav(Change::OpenGateAsk),
                "({x}, {y})"
            );
        }
        assert_eq!(tap(&mut nav, &world, 50, 260), None);
        assert!(nav.gate_ask());
        assert_eq!(nav.gate_ask_left_ms(), Some(GATE_ASK_MS));
        assert_eq!(nav.root(), Section::Live);
        // Open, the key shuts it at once, and the panel stays on LIVE with no ask.
        let (mut nav, world) = live();
        let world = open_gate(world);
        assert_eq!(
            tap(&mut nav, &world, 50, 260),
            Some(Intent::Gate(GateMove::Shut))
        );
        assert!(!nav.gate_ask());
        assert_eq!(nav.root(), Section::Live);
    }

    #[test]
    fn a_choice_opens_the_gate_for_its_time_and_folds_the_ask_away() {
        for (x, secs) in [(180, 300), (300, 1_800), (420, 3_600), (479, 3_600)] {
            let (mut nav, world) = asking();
            assert!(nav.gate_ask());
            assert_eq!(
                tap(&mut nav, &world, x, 270),
                Some(Intent::Gate(GateMove::Open(secs))),
                "{x}"
            );
            assert!(!nav.gate_ask(), "{x}");
            assert_eq!(nav.root(), Section::Live);
        }
    }

    #[test]
    fn any_other_press_only_folds_the_ask_away() {
        // The lane's HOME, the key, the readings, the field above the keys, the gap and the screen's edges:
        // none of them acts while the ask is up.
        for (x, y) in [
            (10, 40),
            (50, 260),
            (50, 160),
            (50, 100),
            (200, 150),
            (300, 100),
            (479, 0),
            (130, 211),
            (123, 270),
            (400, 40),
            (110, 270),
        ] {
            let (mut nav, world) = asking();
            assert_eq!(
                decide(&nav, &world, x, y),
                Tap::Nav(Change::CloseGateAsk),
                "({x}, {y})"
            );
            assert_eq!(tap(&mut nav, &world, x, y), None, "({x}, {y})");
            assert!(!nav.gate_ask(), "({x}, {y})");
            assert_eq!(nav.root(), Section::Live, "({x}, {y}): it did not go HOME");
        }
    }

    #[test]
    fn the_ask_folds_away_by_itself_after_eight_seconds() {
        let (mut nav, world) = live();
        tap(&mut nav, &world, 50, 260);
        assert!(!age_ask(&mut nav, 0));
        assert!(!age_ask(&mut nav, 7_000));
        assert_eq!(nav.gate_ask_left_ms(), Some(1_000));
        assert!(!age_ask(&mut nav, 999));
        assert!(nav.gate_ask());
        assert!(age_ask(&mut nav, 1), "8 s have passed");
        assert!(!nav.gate_ask());
        assert!(!age_ask(&mut nav, 1_000), "nothing left to fold away");
        // One long turn ends it too, and a folded ask stays folded.
        tap(&mut nav, &world, 50, 260);
        assert!(age_ask(&mut nav, u64::MAX));
        assert!(!nav.gate_ask());
        // It starts over when it is opened again.
        tap(&mut nav, &world, 50, 260);
        assert_eq!(nav.gate_ask_left_ms(), Some(GATE_ASK_MS));
    }

    #[test]
    fn a_touch_does_not_start_the_ask_over_but_leaving_live_ends_it() {
        let (mut nav, world) = live();
        tap(&mut nav, &world, 50, 260);
        age_ask(&mut nav, 5_000);
        // Only a press ends it, and the press is the ask's; time is all that runs it down.
        assert_eq!(nav.gate_ask_left_ms(), Some(3_000));
        // The panel goes HOME by itself: the ask goes with LIVE.
        nav.go(Section::Home);
        assert!(!nav.gate_ask());
        // Another screen opened over LIVE, or a whole-system state, folds it away too.
        let (mut nav, world) = live();
        tap(&mut nav, &world, 50, 260);
        nav.set_whole(Some(crate::os::nav::Whole::Night));
        assert!(!nav.gate_ask());
        // And the ask opens only on LIVE.
        let mut nav = Nav::new();
        nav.apply(Change::OpenGateAsk);
        assert!(!nav.gate_ask());
        let mut nav = Nav::at(Section::Devices);
        nav.apply(Change::OpenGateAsk);
        assert!(!nav.gate_ask());
    }

    #[test]
    fn the_lane_on_live_is_home_on_its_first_cells_and_nothing_else() {
        let (nav, world) = live();
        let home = Tap::Nav(Change::Go(Section::Home));
        for (x, y) in [(0, 0), (50, 40), (107, 71), (107, 0), (0, 71)] {
            assert_eq!(decide(&nav, &world, x, y), home, "({x}, {y})");
        }
        // From x 108 the lane carries names: no door, up to the clock and the screen's edge, the gate included.
        for (x, y) in [
            (108, 40),
            (200, 40),
            (347, 40),
            (348, 40),
            (479, 71),
            (110, 0),
        ] {
            assert_eq!(decide(&nav, &world, x, y), Tap::None, "({x}, {y})");
        }
    }

    #[test]
    fn the_clock_column_is_system_and_the_share_and_the_field_are_the_inert_doors_to_names() {
        let (nav, world) = live();
        let system = Tap::Nav(Change::Go(Section::System));
        // The clock: x < 96, from y 148 down to the key's target.
        for (x, y) in [(0, 148), (50, 190), (95, 211), (10, 160)] {
            assert_eq!(decide(&nav, &world, x, y), system, "({x}, {y})");
        }
        // The share, above it: NAMES, which Phase 1 does not have.
        for (x, y) in [(0, 72), (50, 114), (95, 147)] {
            assert_eq!(decide(&nav, &world, x, y), Tap::None, "({x}, {y})");
        }
        // The funnel and the count: NAMES too, inert.
        for (x, y) in [(120, 72), (300, 150), (300, 280), (479, 319), (200, 236)] {
            assert_eq!(decide(&nav, &world, x, y), Tap::None, "({x}, {y})");
        }
        // The gate's own column between the left column and the field, and the rails above the field.
        for (x, y) in [(96, 100), (119, 300), (100, 230)] {
            assert_eq!(decide(&nav, &world, x, y), Tap::None, "({x}, {y})");
        }
        // The system tap goes to ABOUT, the root of SYSTEM.
        let (mut nav, world) = live();
        assert_eq!(tap(&mut nav, &world, 50, 190), None);
        assert_eq!(nav.root(), Section::System);
    }

    #[test]
    fn the_key_wins_over_the_clock_column_and_a_press_target_is_the_tap_itself() {
        let (nav, world) = live();
        // The row where the clock column ends and the key's begins.
        assert_eq!(
            decide(&nav, &world, 50, 211),
            Tap::Nav(Change::Go(Section::System))
        );
        assert_eq!(decide(&nav, &world, 50, 212), Tap::Nav(Change::OpenGateAsk));
        // The gate open or shut makes the key's press a different tap, so a press that slides from one to
        // the other (the world changed under the finger) is not a tap.
        let open = open_gate(world.clone());
        assert_ne!(decide(&nav, &world, 50, 260), decide(&nav, &open, 50, 260));
        // And with the ask up a press anywhere is the same tap: the finger may move about on a closing press.
        let (asking, world) = asking();
        assert_eq!(
            decide(&asking, &world, 10, 10),
            decide(&asking, &world, 300, 100)
        );
    }

    #[test]
    fn live_taps_do_nothing_when_something_covers_it() {
        // A state of the whole system answers nothing (P3's own keys), whatever LIVE would have done.
        let (mut nav, world) = live();
        nav.set_whole(Some(crate::os::nav::Whole::Update));
        for (x, y) in [(50, 260), (10, 40), (50, 190)] {
            assert_eq!(crate::os::nav::decide(&nav, &world, x, y), Tap::None);
        }
    }
}
