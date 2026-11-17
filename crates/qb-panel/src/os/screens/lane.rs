//! The lane (spec §3.1): the status bar every screen shares except LIVE (whose Drain draws its own,
//! with `HOME` in its first four cells) and the states of the whole system.
//!
//! It is a drawing of `(root, World)`: `LaneView::of` makes the view of exactly what the lane puts on
//! the glass, with the words already made, and `draw` paints it. A screen that has the lane holds a
//! `LaneView` in its own view, so that a change in the lane is a change of the screen's view.
//!
//! `HOME` is cells 0 to 3, white on HOME and `G2` elsewhere, with the lit rail under it on HOME and
//! under the section's word (from x 120) elsewhere. Cells 5 to 11 say the system in two words on HOME
//! and the section's word elsewhere. A white tile in cell 13 counts the alerts that wait, on every
//! screen but HOME (which says it in words) and ALERTS (which lists them). The clock is cells 15 to 19.

use crate::format;
use crate::geometry::{Rect, Rgb};
use crate::os::nav::Section;
use crate::os::world::World;
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    COUNT_TILE_DIGIT_BASELINE, COUNT_TILE_DIGIT_X, COUNT_TILE_H, COUNT_TILE_W, COUNT_TILE_X,
    COUNT_TILE_Y, G1, G2, G3, GROUND, LANE_CLOCK_CELL, LANE_WORD_CELL, LIT_HOME_W, LIT_WORD_X,
    RAIL_BOTTOM_Y, RAIL_H, WHITE,
};

/// How many alerts the lane's words count before they say `99+`: the words must fit cells 5 to 14.
const WORDS_COUNT_MAX: u32 = 99;

/// The system in two words, on HOME: the first of these that is true, which is also the door the words are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Says {
    /// Alerts wait: `n ALERT(S)`, white; a tap goes to ALERTS.
    Alerts(u32),
    /// The main gate is open: `GATE OPEN`, white; a tap shuts it.
    GateOpen,
    /// Nothing to say: `ALL WELL`, `G1`; a tap goes to LIVE.
    AllWell,
}

impl Says {
    /// What the system says now: waiting alerts first, then an open gate, else all well.
    pub fn of(world: &World) -> Self {
        if world.alerts_waiting > 0 {
            Self::Alerts(world.alerts_waiting)
        } else if world.gate_open() {
            Self::GateOpen
        } else {
            Self::AllWell
        }
    }

    /// The words, in capitals: `4 ALERTS`, `1 ALERT`, `GATE OPEN`, `ALL WELL`.
    pub fn text(self) -> String {
        match self {
            Self::Alerts(1) => "1 ALERT".to_string(),
            Self::Alerts(n) if n > WORDS_COUNT_MAX => format!("{WORDS_COUNT_MAX}+ ALERTS"),
            Self::Alerts(n) => format!("{n} ALERTS"),
            Self::GateOpen => "GATE OPEN".to_string(),
            Self::AllWell => "ALL WELL".to_string(),
        }
    }

    /// The colour of the words: `G1` when all is well, white when something wants a look.
    pub const fn ink(self) -> Rgb {
        match self {
            Self::AllWell => G1,
            Self::Alerts(_) | Self::GateOpen => WHITE,
        }
    }
}

/// What the lane puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneView {
    /// Where the panel is: it decides `HOME`'s colour, the lit rail and the words.
    pub root: Section,
    /// The words in cells 5 onward: the system's on HOME, the section's elsewhere.
    pub words: String,
    /// The colour of the words.
    pub words_ink: Rgb,
    /// The alert count's digit (`+` above 9) in the white tile, if the tile shows.
    pub tile: Option<char>,
    /// The clock: `hh:mm`, or `--:--` while it is not set.
    pub clock: String,
    /// The clock's colour: `G2`, or `G3` while it is not set.
    pub clock_ink: Rgb,
}

impl LaneView {
    /// The lane of a screen under `root`, from the world.
    pub fn of(root: Section, world: &World) -> Self {
        let (words, words_ink) = if root == Section::Home {
            let says = Says::of(world);
            (says.text(), says.ink())
        } else {
            (root.word().to_string(), WHITE)
        };
        let tile = (root != Section::Home && root != Section::Alerts && world.alerts_waiting > 0)
            .then(|| match world.alerts_waiting {
                n @ 1..=9 => char::from_digit(n, 10).unwrap_or('+'),
                _ => '+',
            });
        let clock = format::clock(world.time);
        let clock_ink = if world.time.is_some() && !clock.starts_with('-') {
            G2
        } else {
            G3
        };
        Self {
            root,
            words,
            words_ink,
            tile,
            clock,
            clock_ink,
        }
    }
}

/// Draws the lane on a canvas: the rails, `HOME`, the words, the lit rail, the count tile and the clock.
pub fn draw(canvas: &mut Canvas, view: &LaneView) {
    let home = view.root == Section::Home;
    canvas.lane_rails();
    canvas.lane_say("HOME", 0.0, if home { WHITE } else { G2 });
    if home {
        canvas.rect(Rect::new(0, RAIL_BOTTOM_Y, LIT_HOME_W, RAIL_H), WHITE);
    } else {
        let letters = u16::try_from(view.words.chars().count()).unwrap_or(0);
        canvas.rect(
            Rect::new(LIT_WORD_X, RAIL_BOTTOM_Y, letters * 24, RAIL_H),
            WHITE,
        );
    }
    canvas.lane_say(&view.words, f32::from(LANE_WORD_CELL), view.words_ink);
    if let Some(digit) = view.tile {
        canvas.rect(
            Rect::new(COUNT_TILE_X, COUNT_TILE_Y, COUNT_TILE_W, COUNT_TILE_H),
            WHITE,
        );
        let mut utf8 = [0; 4];
        canvas.text(
            digit.encode_utf8(&mut utf8),
            COUNT_TILE_DIGIT_X,
            COUNT_TILE_DIGIT_BASELINE,
            Face::bold(24.0, GROUND).mid(),
        );
    }
    canvas.lane_say(&view.clock, f32::from(LANE_CLOCK_CELL), view.clock_ink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{HEIGHT, WIDTH};
    use crate::os::fixture::{Still, world};
    use crate::os::world::Pause;
    use crate::ui::look::{G4, GROUND};
    use crate::ui::painter::ScreenPainter;

    fn lane_of(root: Section, world: &World) -> LaneView {
        LaneView::of(root, world)
    }

    fn painted(view: LaneView) -> Canvas {
        let mut canvas = Canvas::new();
        draw(&mut canvas, &view);
        canvas
    }

    /// Whether any pixel of the box is not the colour `ground`.
    fn has_ink(
        canvas: &Canvas,
        (x0, y0, x1, y1): (usize, usize, usize, usize),
        ground: Rgb,
    ) -> bool {
        (y0..y1).any(|y| (x0..x1).any(|x| canvas.pixel(x, y) != Some(ground)))
    }

    #[test]
    fn home_says_all_well_unless_alerts_wait_or_the_gate_is_open() {
        let mut w = World::default();
        assert_eq!(Says::of(&w), Says::AllWell);
        assert_eq!(Says::AllWell.text(), "ALL WELL");
        assert_eq!(Says::AllWell.ink(), G1);
        w.blocking = Some(Pause {
            left_secs: 100,
            total_secs: 300,
        });
        assert_eq!(Says::of(&w), Says::GateOpen);
        assert_eq!(Says::GateOpen.text(), "GATE OPEN");
        assert_eq!(Says::GateOpen.ink(), WHITE);
        // Alerts come first: the mockup's `sysWord` asks them before the gate.
        w.alerts_waiting = 1;
        assert_eq!(Says::of(&w), Says::Alerts(1));
        assert_eq!(Says::Alerts(1).text(), "1 ALERT");
        w.alerts_waiting = 4;
        assert_eq!(Says::Alerts(4).text(), "4 ALERTS");
        assert_eq!(Says::Alerts(4).ink(), WHITE);
        assert_eq!(Says::Alerts(99).text(), "99 ALERTS");
        assert_eq!(Says::Alerts(100).text(), "99+ ALERTS");
        assert_eq!(Says::Alerts(u32::MAX).text(), "99+ ALERTS");
        // The longest fits the cells between `HOME` and the clock: 5 to 14.
        assert!(Says::Alerts(u32::MAX).text().chars().count() <= 10);
    }

    #[test]
    fn the_view_is_what_the_lane_says() {
        let home = lane_of(Section::Home, &world(Still::Home));
        assert_eq!(home.words, "ALL WELL");
        assert_eq!(home.words_ink, G1);
        assert_eq!(home.tile, None, "HOME says the alerts in words");
        assert_eq!((home.clock.as_str(), home.clock_ink), ("10:21", G2));
        let open = lane_of(Section::Home, &world(Still::HomeOpen));
        assert_eq!((open.words.as_str(), open.words_ink), ("GATE OPEN", WHITE));
        let alerts = lane_of(Section::Home, &world(Still::HomeAlert));
        assert_eq!(
            (alerts.words.as_str(), alerts.words_ink),
            ("4 ALERTS", WHITE)
        );
        let devices = lane_of(Section::Devices, &world(Still::Devices));
        assert_eq!(
            (devices.words.as_str(), devices.words_ink),
            ("DEVICES", WHITE)
        );
        assert_eq!(devices.tile, None, "none wait");
    }

    #[test]
    fn the_count_tile_shows_everywhere_but_home_and_alerts() {
        let waiting = world(Still::HomeAlert);
        for (root, shown) in [
            (Section::Home, false),
            (Section::Alerts, false),
            (Section::Live, true),
            (Section::Devices, true),
            (Section::Names, true),
            (Section::System, true),
            (Section::Gate, true),
        ] {
            assert_eq!(lane_of(root, &waiting).tile.is_some(), shown, "{root:?}");
        }
        assert_eq!(lane_of(Section::Devices, &waiting).tile, Some('4'));
        let mut many = World {
            alerts_waiting: 9,
            ..World::default()
        };
        assert_eq!(lane_of(Section::Devices, &many).tile, Some('9'));
        many.alerts_waiting = 10;
        assert_eq!(
            lane_of(Section::Devices, &many).tile,
            Some('+'),
            "`+` above 9"
        );
        many.alerts_waiting = 0;
        assert_eq!(lane_of(Section::Devices, &many).tile, None);
    }

    #[test]
    fn an_unset_clock_is_dashes_in_a_dim_grey() {
        let mut w = World::default();
        let view = lane_of(Section::Home, &w);
        assert_eq!((view.clock.as_str(), view.clock_ink), ("--:--", G3));
        w.time = Some(crate::state::TimeOfDay { hour: 7, minute: 5 });
        assert_eq!(lane_of(Section::Home, &w).clock, "07:05");
    }

    #[test]
    fn home_is_lit_white_with_its_rail_under_it_and_elsewhere_it_is_grey() {
        let home = painted(lane_of(Section::Home, &world(Still::Home)));
        // The lit rail is x 0 to 96 of rows 64 to 68, white; the rest of the rail is G4.
        for y in 64..68 {
            assert_eq!(home.pixel(0, y), Some(WHITE));
            assert_eq!(home.pixel(95, y), Some(WHITE));
            assert_eq!(home.pixel(96, y), Some(G4));
            assert_eq!(home.pixel(479, y), Some(G4));
        }
        for y in 4..8 {
            assert_eq!(home.pixel(0, y), Some(G4), "the top rail is never lit");
        }
        // Under the section's word elsewhere: from x 120, 24 px a letter.
        let devices = painted(lane_of(Section::Devices, &world(Still::Devices)));
        for y in 64..68 {
            assert_eq!(devices.pixel(0, y), Some(G4));
            assert_eq!(devices.pixel(119, y), Some(G4));
            assert_eq!(devices.pixel(120, y), Some(WHITE));
            assert_eq!(devices.pixel(120 + 7 * 24 - 1, y), Some(WHITE));
            assert_eq!(devices.pixel(120 + 7 * 24, y), Some(G4));
        }
        // `HOME` is white on HOME and G2 elsewhere: the brightest ink in its cells.
        let brightest = |canvas: &Canvas| {
            (0..96)
                .flat_map(|x| (16..56).map(move |y| (x, y)))
                .filter_map(|(x, y)| canvas.pixel(x, y))
                .map(|p| p[0])
                .max()
        };
        assert_eq!(brightest(&home), Some(255));
        assert_eq!(brightest(&devices), Some(G2[0]));
    }

    #[test]
    fn the_lane_draws_nothing_below_its_rails_and_the_words_stay_in_their_cells() {
        let canvas = painted(lane_of(Section::Home, &world(Still::Home)));
        assert!(
            !has_ink(&canvas, (0, 68, WIDTH, HEIGHT), GROUND),
            "nothing under the lane"
        );
        // Cell 4, the gate's place on LIVE, and cells 12 to 14 are empty on HOME with `ALL WELL`.
        assert!(!has_ink(&canvas, (96, 16, 120, 56), GROUND));
        assert!(!has_ink(
            &canvas,
            (5 * 24 + 8 * 24, 16, 15 * 24, 56),
            GROUND
        ));
        // The words and the clock are there.
        assert!(has_ink(&canvas, (120, 16, 312, 56), GROUND));
        assert!(has_ink(&canvas, (15 * 24, 16, 480, 56), GROUND));
    }

    #[test]
    fn the_count_tile_is_white_22_by_40_with_a_black_digit() {
        let canvas = painted(lane_of(Section::Devices, &world(Still::HomeAlert)));
        // The tile's corners, and just outside them.
        for (x, y) in [(313, 16), (334, 16), (313, 55), (334, 55)] {
            assert_eq!(canvas.pixel(x, y), Some(WHITE), "({x}, {y})");
        }
        for (x, y) in [(312, 30), (335, 30), (320, 15), (320, 56)] {
            assert_ne!(canvas.pixel(x, y), Some(WHITE), "({x}, {y})");
        }
        // The digit is black on it: some of the tile is dark, centred on x 324.
        let dark: Vec<usize> = (313..335)
            .flat_map(|x| (16..56).map(move |y| (x, y)))
            .filter(|&(x, y)| canvas.pixel(x, y).is_some_and(|p| p[0] < 100))
            .map(|(x, _)| x)
            .collect();
        assert!(dark.len() > 20, "a digit's ink");
        let middle = (dark.iter().min().copied().unwrap_or(0)
            + dark.iter().max().copied().unwrap_or(0)) as f32
            / 2.0;
        assert!((middle - 324.0).abs() <= 1.5, "centred at {middle}");
    }

    #[test]
    fn one_changed_word_changes_the_view_and_a_painter_gives_its_rows() {
        let mut painter = ScreenPainter::new();
        let mut w = world(Still::Home);
        assert_eq!(
            painter.paint(lane_of(Section::Home, &w), draw),
            Some(0..HEIGHT)
        );
        assert_eq!(
            painter.paint(lane_of(Section::Home, &w), draw),
            None,
            "an equal lane paints nothing"
        );
        // The gate opens: ALL WELL becomes GATE OPEN, in the rows of the lane's letters.
        w.blocking = Some(Pause {
            left_secs: 277,
            total_secs: 300,
        });
        let rows = painter
            .paint(lane_of(Section::Home, &w), draw)
            .expect("the words changed");
        assert!(rows.start >= 16 && rows.end <= 56, "{rows:?}");
        // Alerts arrive.
        w.alerts_waiting = 2;
        let rows = painter
            .paint(lane_of(Section::Home, &w), draw)
            .expect("the words changed");
        assert!(rows.start >= 16 && rows.end <= 56, "{rows:?}");
        // A minute passes: only the clock's cells change, and the write is the clock's rows.
        w.time = Some(crate::state::TimeOfDay {
            hour: 10,
            minute: 22,
        });
        let rows = painter
            .paint(lane_of(Section::Home, &w), draw)
            .expect("the clock changed");
        assert!(
            rows.start >= 16 && rows.end <= 56 && rows.len() <= 40,
            "{rows:?}"
        );
    }
}
