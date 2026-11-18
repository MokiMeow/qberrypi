//! ALERTS (spec §3.6, drawn as round 9's concept A, spec §12): the watch log on tractor-feed paper, and an
//! alert as its slip torn off, stamped with what the Pi did.
//!
//! - **The log** (`alerts`) prints the newest three waiting alerts with the newest last, at the bottom, over
//!   the line the machine is still typing (`10:21  WATCH CONTINUES`, a cursor and `n ON THE LOG`). An entry is
//!   two lines on blue-bar paper: `hh:mm  URGENT` (or `ROUTINE`) and the device, then the title; a white flag
//!   in the margin for an urgent one, a blue one for the rest. Each is a door to its slip.
//! - **Nothing to report** is an open watch: when the last entry was filed and when this watch opened (each
//!   left out when the Pi does not know it, spec §12.3), `NOTHING TO REPORT`, `hh:mm  ALL QUIET` and the cursor.
//! - **ALERT** (`alert`) is the slip: `URGENT · hh:mm`, the stamp (`REFUSED`, `ADMITTED` or `OBSERVED`, spec
//!   §12.2), the title and up to three lines, over the floor in A's dress: `DISMISS` (acks, back to the log) and
//!   `SEE DEVICE` (a dim outline and no door when the device is not on the wall). For an alert about a name
//!   (`threat`, `rebind`) the slip's name line is a door to NAME.
//!
//! The cursor is the one thing that moves at rest (spec §12.4): 10 by 18 px, on for 4 frames and off for 4 at 8 a
//! second, so its write is the 18 rows it stands in. It is part of the log's view as `World::cursor_on`, which the
//! caller flips on its frame count, so that nothing else is painted twice (the painter writes the rows that
//! changed, and only those).

use crate::format;
use crate::geometry::Rect;
use crate::os::intent::Intent;
use crate::os::nav::{Change, Screen, Section, Tap};
use crate::os::screens::lane::{self, LaneView};
use crate::os::screens::parts::{self, Dress, Right};
use crate::os::world::{AlertRow, MAX_ALERT_ROWS, World};
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    BLUE, BLUE_LT, BLUE_NIGHT, FLOOR_SPLIT_X, FLOOR_Y, G1, G2, G3, G4, GROUND, WHITE,
};

/// The top of the log's first entry: entries stand 60 px apart, the newest at the bottom.
const LOG_Y0: u16 = 92;

/// The distance between two entries of the log.
const LOG_PITCH: u16 = 60;

/// The slip's top, and its height; its width is 428 from x 26.
const SLIP_Y: u16 = 82;

/// The log's door for an entry: the entry's rows, and 24 to 456 across.
const ENTRY_DOOR_X: u16 = 24;

/// The width of an entry's door.
const ENTRY_DOOR_W: u16 = 432;

/// The slip's name line is a door from this row down to the slip's foot (`door(24, 156, 432, 76)`).
const NAME_DOOR: Rect = Rect::new(24, 156, 432, 76);

/// How many alerts the log counts in words before it says `999+`: `999+ ON THE LOG` stands clear of the typed line.
const LOG_COUNT_MAX: u32 = 999;

/// One entry of the log as it is printed.
#[derive(Clone, PartialEq, Eq)]
pub struct EntryView {
    /// Whether it is urgent: a white flag and a white title.
    pub high: bool,
    /// The first line's left: `10:19  URGENT`, with the time, or `MON` for an older one, before it.
    pub head: String,
    /// The device it is about, in capitals; empty when none is known.
    pub who: String,
    /// The title.
    pub title: String,
}

impl std::fmt::Debug for EntryView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryView")
            .field("high", &self.high)
            .finish_non_exhaustive()
    }
}

/// What the log prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogBody {
    /// Entries: the newest three, newest first (the screen prints them upside down: the newest at the
    /// bottom), and how many wait in words.
    Entries {
        /// The entries, newest first.
        rows: Vec<EntryView>,
        /// `4 ON THE LOG`.
        waiting: String,
    },
    /// Nothing waits: the open watch.
    Quiet {
        /// `MON 21:40`: when the last entry was filed; `None` when there never was one.
        last_entry: Option<String>,
        /// `TUE 00:00`: when this watch opened; `None` while the clock is not set.
        watch_opened: Option<String>,
    },
}

/// What ALERTS puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogView {
    /// The lane, which says `ALERTS` and has no count tile.
    pub lane: LaneView,
    /// The entries, or the open watch.
    pub body: LogBody,
    /// The time the typed line starts with: `10:21`, or `--:--` while the clock is not set.
    pub clock: String,
    /// The cursor, on for its 4 frames: the one motion at rest.
    pub cursor: bool,
}

/// `4 ON THE LOG`, with the count held to `999+`.
pub(crate) fn waiting_words(n: u32) -> String {
    if n > LOG_COUNT_MAX {
        format!("{LOG_COUNT_MAX}+ ON THE LOG")
    } else {
        format!("{n} ON THE LOG")
    }
}

impl LogView {
    /// What the log shows for a world: the newest three alerts, or the open watch when none waits.
    pub fn of(world: &World) -> Self {
        let body = if world.alerts.is_empty() {
            LogBody::Quiet {
                last_entry: world.last_entry.map(|moment| moment.word()),
                watch_opened: world.watch_opened.map(|moment| moment.word()),
            }
        } else {
            let rows: Vec<EntryView> = world
                .alerts
                .iter()
                .take(MAX_ALERT_ROWS)
                .map(|row| EntryView {
                    high: row.high,
                    head: match row.at {
                        Some(at) => format!("{}  {}", at.word(), row.precedence()),
                        None => row.precedence().to_string(),
                    },
                    who: row.who.to_uppercase(),
                    title: row.title.clone(),
                })
                .collect();
            let held = u32::try_from(rows.len()).unwrap_or(u32::MAX);
            LogBody::Entries {
                waiting: waiting_words(world.alerts_waiting.max(held)),
                rows,
            }
        };
        Self {
            lane: LaneView::of(Section::Alerts, world),
            body,
            clock: format::clock(world.time),
            cursor: world.cursor_on,
        }
    }
}

/// The top of the log's `i`th place from the top, 0 to 2: where an entry stands.
const fn entry_y(i: u16) -> u16 {
    LOG_Y0 + i * LOG_PITCH
}

/// The place from the top of the entry `k`, counted from the newest: the newest is the last, at the bottom.
const fn place_of(k: usize) -> u16 {
    (MAX_ALERT_ROWS - 1 - k) as u16
}

/// Draws the log, or the open watch.
pub fn draw_log(canvas: &mut Canvas, view: &LogView) {
    lane::draw(canvas, &view.lane);
    parts::paper(canvas, 76, 320, G4);
    parts::tear(canvas, 26, 454, 76, G4, false);
    match &view.body {
        LogBody::Entries { rows, waiting } => {
            for (k, entry) in rows.iter().enumerate() {
                let i = place_of(k);
                let y = entry_y(i);
                let fy = f32::from(y);
                if i.is_multiple_of(2) {
                    canvas.rect(Rect::new(26, y, 428, 58), BLUE_NIGHT);
                }
                canvas.rect(
                    Rect::new(30, y + 6, 6, 46),
                    if entry.high { WHITE } else { BLUE },
                );
                canvas.text(
                    &entry.head,
                    46.0,
                    fy + 20.0,
                    Face::bold(16.0, if entry.high { WHITE } else { G2 }).track(1.0),
                );
                canvas.text(&entry.who, 446.0, fy + 20.0, Face::bold(16.0, G2).end());
                canvas.text(
                    &entry.title,
                    45.0,
                    fy + 46.0,
                    Face::bold(24.0, if entry.high { WHITE } else { BLUE_LT }),
                );
            }
            let typed = format!("{}  WATCH CONTINUES", view.clock);
            let face = Face::regular(18.0, G2);
            canvas.text(&typed, 46.0, 302.0, face);
            if view.cursor {
                let x = 46.0 + face.width(typed.chars().count() + 1);
                canvas.rect_f(x, 287.0, 10.0, 18.0, G1);
            }
            canvas.text(waiting, 446.0, 301.0, Face::bold(16.0, G3).end());
        }
        LogBody::Quiet {
            last_entry,
            watch_opened,
        } => {
            if let Some(moment) = last_entry {
                canvas.text(
                    &format!("{moment}  LAST ENTRY FILED"),
                    44.0,
                    124.0,
                    Face::regular(16.0, G3),
                );
            }
            if let Some(moment) = watch_opened {
                canvas.text(
                    &format!("{moment}  WATCH OPENED"),
                    44.0,
                    150.0,
                    Face::regular(16.0, G3),
                );
            }
            canvas.text("NOTHING TO REPORT", 43.0, 206.0, Face::bold(24.0, G1));
            let typed = format!("{}  ALL QUIET", view.clock);
            let face = Face::regular(18.0, G2);
            canvas.text(&typed, 44.0, 240.0, face);
            if view.cursor {
                let x = 44.0 + face.width(typed.chars().count() + 1);
                canvas.rect_f(x, 225.0, 10.0, 18.0, G1);
            }
        }
    }
}

/// The log's taps: an entry opens its slip. Nothing else on the paper is a door.
pub fn log_tap(world: &World, x: u16, y: u16) -> Tap {
    if !(ENTRY_DOOR_X..ENTRY_DOOR_X + ENTRY_DOOR_W).contains(&x) {
        return Tap::None;
    }
    world
        .alerts
        .iter()
        .take(MAX_ALERT_ROWS)
        .enumerate()
        .find(|&(k, _)| {
            let top = entry_y(place_of(k));
            (top..top + LOG_PITCH).contains(&y)
        })
        .map_or(Tap::None, |(_, row)| {
            Tap::Nav(Change::Open(Screen::Alert(row.id)))
        })
}

/// What ALERT puts on the glass: the slip.
#[derive(Clone, PartialEq, Eq)]
pub struct AlertView {
    /// The lane, which says `ALERTS` and has no count tile.
    pub lane: LaneView,
    /// Whether the slip is white (urgent) rather than blue.
    pub high: bool,
    /// The line at the top left: `URGENT · 10:19`, or `ROUTINE` alone while the time is not known.
    pub head: String,
    /// The stamp's word: `REFUSED`, `ADMITTED` or `OBSERVED`.
    pub stamp: &'static str,
    /// The title, in capitals.
    pub title: String,
    /// The lines under it: the subject first, 20 px bold, then the others at 16 px.
    pub lines: Vec<String>,
    /// Whether `SEE DEVICE` is a key: the alert names a device that is on the wall.
    pub see_device: bool,
}

impl std::fmt::Debug for AlertView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertView")
            .field("high", &self.high)
            .field("stamp", &self.stamp)
            .field("lines", &self.lines.len())
            .field("see_device", &self.see_device)
            .finish_non_exhaustive()
    }
}

/// Whether `SEE DEVICE` is a key for this alert: the device it names is on the wall.
fn sees_device(world: &World, alert: &AlertRow) -> Option<crate::os::world::Mac> {
    alert.device.filter(|mac| world.device(mac).is_some())
}

impl AlertView {
    /// What ALERT shows for the alert with this id: `None` when the world does not hold it, because it
    /// is gone (the nav pops the screen at its next refresh).
    pub fn of(world: &World, id: u64) -> Option<Self> {
        let alert = world.alert(id)?;
        Some(Self {
            lane: LaneView::of(Section::Alerts, world),
            high: alert.high,
            head: match alert.at {
                Some(at) => format!("{} \u{b7} {}", alert.precedence(), at.word()),
                None => alert.precedence().to_string(),
            },
            stamp: alert.stamp.word(),
            title: alert.title.clone(),
            lines: alert.lines.clone(),
            see_device: sees_device(world, alert).is_some(),
        })
    }
}

/// ALERT's taps: the floor's left key dismisses the alert (acks it and goes back to the log); the right opens
/// the device it names, when it is on the wall; for an alert about a name, the name line is a door to NAME.
pub fn alert_tap(world: &World, id: u64, x: u16, y: u16) -> Tap {
    let Some(alert) = world.alert(id) else {
        return Tap::None;
    };
    if y >= FLOOR_Y {
        return if x < FLOOR_SPLIT_X {
            Tap::Act(Intent::Ack(id), Some(Change::Back))
        } else {
            sees_device(world, alert)
                .map_or(Tap::None, |mac| Tap::Nav(Change::Open(Screen::Device(mac))))
        };
    }
    match &alert.name {
        Some(name) if NAME_DOOR.contains(x, y) => {
            Tap::Nav(Change::Open(Screen::Name(name.clone())))
        }
        _ => Tap::None,
    }
}

/// Draws ALERT: the slip.
pub fn draw_alert(canvas: &mut Canvas, view: &AlertView) {
    lane::draw(canvas, &view.lane);
    parts::paper(canvas, 76, 320, G4);
    let ground = if view.high { WHITE } else { BLUE };
    canvas.rect(Rect::new(26, SLIP_Y, 428, 150), ground);
    parts::tear(canvas, 26, 454, SLIP_Y, GROUND, false);
    parts::tear(canvas, 26, 454, SLIP_Y + 150, GROUND, true);
    canvas.text(&view.head, 40.0, 112.0, Face::bold(16.0, GROUND).track(1.0));
    parts::ink_stamp(canvas, Rect::new(340, 92, 104, 28), view.stamp, ground);
    let (px, baseline) = if view.title.chars().count() > 16 {
        (30.0, 152.0)
    } else {
        (40.0, 156.0)
    };
    canvas.text(
        &view.title,
        40.0 - px * 0.05,
        baseline,
        Face::bold(px, GROUND),
    );
    for (k, line) in view.lines.iter().enumerate() {
        let (px, bold) = if k == 0 { (20.0, true) } else { (16.0, false) };
        let face = if bold {
            Face::bold(px, GROUND)
        } else {
            Face::regular(px, GROUND)
        };
        let y = 180.0 + 20.0 * k as f32 - if k == 2 { 1.0 } else { 0.0 };
        canvas.text(line, 40.0, y, face);
    }
    parts::keys9(
        canvas,
        Dress::A,
        Some("DISMISS"),
        if view.see_device {
            Right::Key("SEE DEVICE")
        } else {
            Right::Inert("SEE DEVICE")
        },
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, mac_of, world};
    use crate::os::world::{AlertStamp, Moment, Weekday, When};

    fn is(canvas: &Canvas, x: usize, y: usize, color: crate::geometry::Rgb) -> bool {
        canvas.pixel(x, y) == Some(color)
    }

    #[test]
    fn the_log_prints_the_newest_three_with_the_newest_last_and_says_how_many_wait() {
        let w = world(Still::Alerts);
        let view = LogView::of(&w);
        assert_eq!(view.lane.words, "ALERTS");
        assert_eq!(view.lane.tile, None, "no count tile under ALERTS");
        assert_eq!(view.clock, "10:21");
        assert!(view.cursor, "the stills are taken with the cursor on");
        let LogBody::Entries { rows, waiting } = &view.body else {
            panic!("entries");
        };
        assert_eq!(waiting, "4 ON THE LOG", "four wait; three are printed");
        let heads: Vec<&str> = rows.iter().map(|row| row.head.as_str()).collect();
        assert_eq!(heads, ["10:19  URGENT", "09:52  ROUTINE", "02:14  URGENT"]);
        let who: Vec<&str> = rows.iter().map(|row| row.who.as_str()).collect();
        assert_eq!(who, ["SAM'S PHONE", "\u{b7}216", "\u{b7}143"]);
        let said = format!("{view:?}");
        assert!(
            !said.contains("PHISHING") && !said.contains("SAM"),
            "no name in a Debug: {said}"
        );
    }

    #[test]
    fn with_none_waiting_the_log_is_the_open_watch_with_what_the_pi_knows() {
        let w = world(Still::AlertsNone);
        let LogBody::Quiet {
            last_entry,
            watch_opened,
        } = LogView::of(&w).body
        else {
            panic!("quiet");
        };
        assert_eq!(last_entry.as_deref(), Some("MON 21:40"));
        assert_eq!(watch_opened.as_deref(), Some("TUE 00:00"));
        // The store never had an alert, and the clock is not set: both are left out, not zeros.
        let mut bare = w.clone();
        bare.last_entry = None;
        bare.watch_opened = None;
        bare.time = None;
        let view = LogView::of(&bare);
        assert_eq!(
            view.body,
            LogBody::Quiet {
                last_entry: None,
                watch_opened: None
            }
        );
        assert_eq!(view.clock, "--:--");
        let mut canvas = Canvas::new();
        draw_log(&mut canvas, &view);
        // The two lines of G3 are not there; `NOTHING TO REPORT` is.
        let ink = |rows: std::ops::Range<usize>, color| {
            rows.flat_map(|y| (30..450).map(move |x| (x, y)))
                .any(|(x, y)| is(&canvas, x, y, color))
        };
        assert!(!ink(112..128, G3) && !ink(138..154, G3));
        assert!(ink(188..210, G1));
    }

    #[test]
    fn an_entry_is_blue_bar_paper_with_a_margin_flag_and_the_newest_stands_at_the_bottom() {
        let mut canvas = Canvas::new();
        draw_log(&mut canvas, &LogView::of(&world(Still::Alerts)));
        // Three places 60 apart from y 92; the first and the third (the oldest and the newest) have the blue bar.
        assert!(is(&canvas, 100, 93, BLUE_NIGHT) && is(&canvas, 100, 213, BLUE_NIGHT));
        assert!(
            is(&canvas, 400, 160, GROUND),
            "the middle entry is on plain paper"
        );
        // The margin flags: white for urgent (the oldest, ARP SPOOF, and the newest), blue for the routine one.
        assert!(is(&canvas, 32, 100, WHITE) && is(&canvas, 32, 220, WHITE));
        assert!(is(&canvas, 32, 160, BLUE));
        // The typed line and the sprocket holes.
        assert!(
            (46..300).any(|x| (290..304).any(|y| canvas.pixel(x, y).is_some_and(|p| p[0] > 100)))
        );
        assert!(is(&canvas, 11, 86, G4) && is(&canvas, 469, 302, G4));
    }

    #[test]
    fn the_cursor_is_10_by_18_after_the_typed_line_and_toggles_nothing_else() {
        let mut on = world(Still::Alerts);
        on.cursor_on = true;
        let mut off = on.clone();
        off.cursor_on = false;
        let (mut a, mut b) = (Canvas::new(), Canvas::new());
        draw_log(&mut a, &LogView::of(&on));
        draw_log(&mut b, &LogView::of(&off));
        // Only rows 287 to 304 differ, in columns 294 to 304 (x 294.4 to 304.4).
        let differing: Vec<(usize, usize)> = (0..320)
            .flat_map(|y| (0..480).map(move |x| (x, y)))
            .filter(|&(x, y)| a.pixel(x, y) != b.pixel(x, y))
            .collect();
        let (xs, ys): (Vec<usize>, Vec<usize>) = differing.iter().copied().unzip();
        assert_eq!(
            (xs.iter().min(), xs.iter().max()),
            (Some(&294), Some(&304)),
            "10 px wide from x 294.4"
        );
        assert_eq!((ys.iter().min(), ys.iter().max()), (Some(&287), Some(&304)));
        // The quiet log's cursor stands after `10:21  ALL QUIET `, on 225 to 242.
        let (mut c, mut d) = (Canvas::new(), Canvas::new());
        let mut quiet_on = world(Still::AlertsNone);
        quiet_on.cursor_on = true;
        let mut quiet_off = quiet_on.clone();
        quiet_off.cursor_on = false;
        draw_log(&mut c, &LogView::of(&quiet_on));
        draw_log(&mut d, &LogView::of(&quiet_off));
        let rows: Vec<usize> = (0..320)
            .filter(|&y| (0..480).any(|x| c.pixel(x, y) != d.pixel(x, y)))
            .collect();
        assert_eq!((rows.first(), rows.last()), (Some(&225), Some(&242)));
    }

    #[test]
    fn a_press_on_an_entry_opens_its_slip_and_nothing_else_on_the_paper_does() {
        let w = world(Still::Alerts);
        let open = |id: u64| Tap::Nav(Change::Open(Screen::Alert(id)));
        // Newest at the bottom: id 4 at y 212, 3 in the middle, 2 at the top (the log holds three of the four).
        assert_eq!(log_tap(&w, 200, 240), open(4));
        assert_eq!(log_tap(&w, 200, 180), open(3));
        assert_eq!(log_tap(&w, 200, 110), open(2));
        for (x, y) in [(23, 240), (456, 240), (200, 91), (200, 272), (200, 300)] {
            assert_eq!(log_tap(&w, x, y), Tap::None, "({x}, {y})");
        }
        assert_eq!(log_tap(&w, 24, 212), open(4));
        assert_eq!(log_tap(&w, 455, 271), open(4));
        // With one alert only the bottom place is held.
        let mut one = w.clone();
        one.alerts.truncate(1);
        assert_eq!(log_tap(&one, 200, 240), open(4));
        assert_eq!(log_tap(&one, 200, 110), Tap::None);
        assert_eq!(log_tap(&World::default(), 200, 240), Tap::None);
    }

    #[test]
    fn the_slip_has_the_stamp_the_title_and_lines_and_the_floor_in_a_dress() {
        let w = world(Still::Alert);
        let view = AlertView::of(&w, 4).expect("the newest alert");
        assert!(view.high && view.see_device);
        assert_eq!(view.head, "URGENT \u{b7} 10:19");
        assert_eq!(view.stamp, "REFUSED");
        assert_eq!(view.title, "PHISHING BLOCKED");
        assert_eq!(view.lines.len(), 3);
        assert_eq!(AlertView::of(&w, 3).map(|v| v.stamp), Some("ADMITTED"));
        assert_eq!(AlertView::of(&w, 2).map(|v| v.stamp), Some("OBSERVED"));
        assert_eq!(AlertView::of(&w, 1), None, "the fourth is not in the log");
        assert_eq!(AlertView::of(&World::default(), 4), None);
        let said = format!("{view:?}");
        assert!(
            !said.contains("PHISHING") && !said.contains("secure-login"),
            "{said}"
        );
        let mut canvas = Canvas::new();
        draw_alert(&mut canvas, &view);
        // White slip 428 by 150 at (26, 82), torn at the top and the foot; the stamp's black frame.
        assert!(
            is(&canvas, 100, 85, WHITE)
                && is(&canvas, 30, 200, WHITE)
                && is(&canvas, 450, 200, WHITE)
        );
        assert!(
            is(&canvas, 32, 86, GROUND),
            "the first tooth cut into the slip"
        );
        assert!(is(&canvas, 340, 92, GROUND) && is(&canvas, 443, 119, GROUND));
        // The keys: DISMISS on the left, SEE DEVICE solid on the right, the perforation between slip and keys.
        assert!(is(&canvas, 20, 240, WHITE) && is(&canvas, 300, 250, WHITE));
        assert!(is(&canvas, 8, 235, G3));
    }

    #[test]
    fn the_slip_is_blue_for_a_routine_alert_with_a_time_when_it_has_one() {
        let mut w = world(Still::Alert);
        w.alerts[0].high = false;
        w.alerts[0].at = None;
        let view = AlertView::of(&w, 4).expect("alert");
        assert_eq!(view.head, "ROUTINE", "no time known: no time said");
        let mut canvas = Canvas::new();
        draw_alert(&mut canvas, &view);
        assert!(is(&canvas, 100, 85, BLUE));
        w.alerts[0].at = Some(When::Day(Weekday::Mon));
        assert_eq!(
            AlertView::of(&w, 4).map(|v| v.head).as_deref(),
            Some("ROUTINE \u{b7} MON")
        );
        // A long title is set at 30 px.
        w.alerts[0].title = "SUSPICIOUS LOOKUPS".into();
        let mut small = Canvas::new();
        draw_alert(&mut small, &AlertView::of(&w, 4).expect("alert"));
        let ink_rows = |canvas: &Canvas| {
            (122..160)
                .filter(|&y| (40..300).any(|x| canvas.pixel(x, y) == Some(GROUND)))
                .count()
        };
        w.alerts[0].title = "ARP SPOOF".into();
        let mut big = Canvas::new();
        draw_alert(&mut big, &AlertView::of(&w, 4).expect("alert"));
        assert!(ink_rows(&big) > ink_rows(&small), "40 px is taller than 30");
    }

    #[test]
    fn see_device_is_a_key_only_when_the_device_is_on_the_wall() {
        let mut w = world(Still::Alert);
        let key = |w: &World| alert_tap(w, 4, 400, 280);
        assert_eq!(
            key(&w),
            Tap::Nav(Change::Open(Screen::Device(mac_of(23)))),
            "Sam's phone is on the wall"
        );
        w.alerts[0].device = Some([2, 0, 0, 0, 0, 99]);
        assert_eq!(key(&w), Tap::None, "a device that is not on the wall");
        let view = AlertView::of(&w, 4).expect("alert");
        assert!(!view.see_device);
        let mut canvas = Canvas::new();
        draw_alert(&mut canvas, &view);
        assert!(is(&canvas, 250, 245, GROUND), "no solid key");
        assert!(
            is(&canvas, 300, 240, G4),
            "a dim outline where it would stand"
        );
        w.alerts[0].device = None;
        assert_eq!(key(&w), Tap::None, "an alert that names no device");
    }

    #[test]
    fn the_slips_keys_dismiss_and_see_and_the_name_line_of_a_threat_is_a_door() {
        let w = world(Still::Alert);
        assert_eq!(
            alert_tap(&w, 4, 100, 280),
            Tap::Act(Intent::Ack(4), Some(Change::Back)),
            "DISMISS acks and goes back to the log"
        );
        assert_eq!(alert_tap(&w, 4, 239, 236), alert_tap(&w, 4, 0, 319));
        assert_ne!(alert_tap(&w, 4, 240, 236), alert_tap(&w, 4, 239, 236));
        // The name line: the slip's foot, from y 156 to 231 and x 24 to 455.
        let name = Tap::Nav(Change::Open(Screen::Name(
            "secure-login.example.org".into(),
        )));
        for (x, y) in [(100, 180), (24, 156), (455, 231), (300, 200)] {
            assert_eq!(alert_tap(&w, 4, x, y), name, "({x}, {y})");
        }
        for (x, y) in [
            (100, 120),
            (23, 180),
            (456, 180),
            (100, 155),
            (100, 232),
            (100, 235),
        ] {
            assert_eq!(alert_tap(&w, 4, x, y), Tap::None, "({x}, {y})");
        }
        // A new device's slip names no name: no door.
        assert_eq!(alert_tap(&w, 3, 100, 180), Tap::None);
        // An alert that is not in the log has no keys.
        assert_eq!(alert_tap(&w, 7, 100, 280), Tap::None);
    }

    #[test]
    fn the_stamp_and_the_precedence_are_what_the_row_says() {
        let mut w = world(Still::Alert);
        w.alerts[0].stamp = AlertStamp::Observed;
        assert_eq!(AlertView::of(&w, 4).map(|v| v.stamp), Some("OBSERVED"));
        w.last_entry = Some(Moment {
            day: Weekday::Sat,
            time: crate::TimeOfDay { hour: 7, minute: 5 },
        });
        assert_eq!(w.last_entry.map(Moment::word).as_deref(), Some("SAT 07:05"));
    }

    #[test]
    fn a_count_past_999_is_held_to_999_plus() {
        assert_eq!(waiting_words(4), "4 ON THE LOG");
        assert_eq!(waiting_words(999), "999 ON THE LOG");
        assert_eq!(waiting_words(1000), "999+ ON THE LOG");
        assert_eq!(waiting_words(u32::MAX), "999+ ON THE LOG");
        // The count never says fewer than the entries printed.
        let mut w = world(Still::Alerts);
        w.alerts_waiting = 1;
        let LogBody::Entries { waiting, .. } = LogView::of(&w).body else {
            panic!("entries");
        };
        assert_eq!(waiting, "3 ON THE LOG");
    }
}
