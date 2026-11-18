//! The states of the whole system (spec §5.4, §12): they take the whole screen, draw their own lane (`laneMid`) and
//! offer no HOME. A touch on one does nothing but what the state itself answers.
//!
//! - **STARTING** (`ca.js` `boot`, concept A) is the checklist of five lines on tractor-feed paper, ticked from what the
//!   panel can read: the panel is on, Wi-Fi joined, the clock set, the lists loaded, DNS answering. A line that is done
//!   has a blue tick and its word at the right; the first that is not is under way (white); the rest wait (dim).
//! - **UPDATE** (`cc.js` `update`, concept C, spec §12.3 and §12.5) is the changeover: two slot boxes, the slot that
//!   served before the update on the left and the new one on the right, by their real letters, joined by a bus to DNS,
//!   which is drawn unbroken. A version is written only beside the slot that runs. Receiving fills the right box by
//!   the share received; the trial's figure is `m:ss` to its minimum time, then the word it waits for; confirmed and
//!   rolled back go HOME on a tap (the nav's) or after 30 s. DNS's dot is drawn still.
//! - **NIGHT** (`ca.js` `night`, concept A, spec §12.6) is the paper's margins in `BLUE_NIGHT`, the hour in 96 px
//!   `BLUE_DEEP` and `NIGHT WATCH`, which becomes `n ON THE LOG` in white while alerts wait. A touch wakes to HOME.
//! - **PIN** (`ca.js` `pin`, concept A, with round 8's twelve keys, `PAD` and `PADKEY`) is the authorisation slip beside
//!   the keys: what is asked (`ENTER THE PIN`, `CHOOSE A PIN`, `WRONG PIN`, `WAIT m:ss`), four boxes, what is signed
//!   `FOR`, and `THE OWNER`. The lane says what the PIN is for (`PIN TO ALLOW NAME`).
//!
//! The pad's digits never reach a view: it holds how many are typed, and nothing of which.

use std::fmt;

use crate::format::{self, fit_text, mss, wrap_name};
use crate::geometry::Rect;
use crate::os::intent::{Digit, Intent, Verdict};
use crate::os::nav::{Change, PinGoal, PinPad, Tap};
use crate::os::screens::alerts::waiting_words;
use crate::os::screens::parts::{paper, tick_box};
use crate::os::world::{UpdatePhase, VERSION_CELLS, World};
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    BLUE, BLUE_DEEP, BLUE_DK, BLUE_LT, BLUE_MD, BLUE_NIGHT, G1, G2, G3, G4, GROUND, WHITE,
};

// ---------------------------------------------------------------------------------------------
// STARTING

/// The five lines of the checklist: the word at the left. The word at the right once it is done is in `DONE_WORDS`,
/// and for the lists' line carries the figure.
const STARTING_WORDS: [&str; 5] = ["PANEL", "WI-FI", "CLOCK", "LISTS", "DNS"];

/// What a line says at the right once it is done; the lists' line, `212K NAMES`, is made from the world.
const DONE_WORDS: [&str; 5] = ["ON", "JOINED", "SET", "", "ANSWERING"];

/// Where a line of the checklist stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Done: a blue tick, and its word at the right.
    Done,
    /// The first that is not done: white.
    Now,
    /// Still to come: dim.
    Waiting,
}

/// What STARTING puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartingView {
    /// The five lines, top to bottom.
    pub steps: [Step; 5],
    /// The lists' line at the right once it is done: `212K NAMES`.
    pub names: String,
}

impl StartingView {
    /// The checklist the world's readings tick: the panel is on (it is drawing), Wi-Fi joined, the clock set, the lists
    /// are loaded (they may hold no name: none is configured, say), DNS answers. A reading that is true is done
    /// whatever the ones before it say; the first that is not is under way.
    pub fn of(world: &World) -> Self {
        let done = [
            true,
            world.wifi_joined,
            world.time.is_some(),
            world.list_names.is_some(),
            world.dns_answering,
        ];
        let now = done.iter().position(|done| !done);
        let steps = std::array::from_fn(|k| {
            if done[k] {
                Step::Done
            } else if Some(k) == now {
                Step::Now
            } else {
                Step::Waiting
            }
        });
        Self {
            steps,
            names: format!(
                "{} NAMES",
                format::short_count(world.list_names.unwrap_or(0))
            ),
        }
    }
}

/// Draws STARTING.
pub fn draw_starting(canvas: &mut Canvas, view: &StartingView) {
    canvas.lane_rails();
    canvas.lane_mid("QBERRYPI STARTING", WHITE);
    paper(canvas, 76, 320, G4);
    for (k, step) in view.steps.iter().enumerate() {
        let y = 84 + 44 * k as u16;
        let (ink, tick) = match step {
            Step::Done => (BLUE, true),
            Step::Now => (WHITE, false),
            Step::Waiting => (G4, false),
        };
        tick_box(canvas, 34, y + 8, 24, tick, ink, GROUND);
        let word = if *step == Step::Waiting { G3 } else { WHITE };
        canvas.text(
            STARTING_WORDS[k],
            70.0,
            f32::from(y) + 29.0,
            Face::bold(24.0, word),
        );
        if *step == Step::Done {
            let done = if k == 3 {
                view.names.as_str()
            } else {
                DONE_WORDS[k]
            };
            canvas.text(
                done,
                446.0,
                f32::from(y) + 27.0,
                Face::bold(16.0, G2).track(2.0).end(),
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// UPDATE

/// The four states of UPDATE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `RECEIVING UPDATE`: the old build answers, the right box fills.
    Receiving,
    /// `TRIAL CONFIRMS IN`: the new build answers on trial.
    Trial,
    /// `UPDATE CONFIRMED`: the new build stays.
    Confirmed,
    /// `UPDATE ROLLED BACK`: the old build is back.
    RolledBack,
}

impl Stage {
    /// What the lane says.
    const fn lane(self) -> &'static str {
        match self {
            Self::Receiving => "RECEIVING UPDATE",
            Self::Trial => "TRIAL CONFIRMS IN",
            Self::Confirmed => "UPDATE CONFIRMED",
            Self::RolledBack => "UPDATE ROLLED BACK",
        }
    }

    /// Whether the new build is the one that answers (`onB` in the mockup).
    const fn new_serves(self) -> bool {
        matches!(self, Self::Trial | Self::Confirmed)
    }
}

/// The rows the right box's fill rises through when the whole build is received: the box is 84 high, the fill stands
/// 2 px off its foot and 76 at most.
const FILL_ROWS: f32 = 76.0;

/// What UPDATE puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateView {
    /// Which of the four it is.
    pub stage: Stage,
    /// The letter of the slot that served before the update: the left box.
    pub before: char,
    /// The letter of the slot the new build is in: the right box.
    pub after: char,
    /// The version that runs, beside the letter of the slot that runs it; the other box has no version.
    pub version: String,
    /// The right box's figure while receiving or on trial: `62%`, `1:47`, or the word the trial waits for. Empty otherwise.
    pub figure: String,
    /// Whether the figure is 40 px (a share, a time) rather than 30 (a word).
    pub figure_big: bool,
    /// How many rows the fill has risen, 0 to 76: only while receiving.
    pub fill_rows: u16,
}

impl UpdateView {
    /// The changeover the world's update reads: an update that has just begun when the world has none yet.
    pub fn of(world: &World) -> Self {
        let update = world.update.unwrap_or_default();
        let (stage, figure, figure_big, fill_rows) = match update.phase {
            UpdatePhase::Receiving { .. } => {
                let (percent, ratio) = update.phase.received();
                (
                    Stage::Receiving,
                    format!("{percent}%"),
                    true,
                    (FILL_ROWS * ratio).round() as u16,
                )
            }
            UpdatePhase::Trial {
                left_secs,
                waiting_for,
            } => match (left_secs, waiting_for) {
                // Past the minimum time and still waiting: the word for what it waits for (spec §12.5).
                (0, Some(waiting)) => (Stage::Trial, waiting.word().to_string(), false, 0),
                _ => (Stage::Trial, mss(left_secs), true, 0),
            },
            UpdatePhase::Confirmed => (Stage::Confirmed, String::new(), false, 0),
            UpdatePhase::RolledBack => (Stage::RolledBack, String::new(), false, 0),
        };
        Self {
            stage,
            before: update.before.letter(),
            after: update.after.letter(),
            version: fit_text(&world.version, VERSION_CELLS),
            figure,
            figure_big,
            fill_rows,
        }
    }
}

/// A slot's label: `SLOT A`, and ` · 0.4.1` after it when it is the build that runs.
fn slot_label(letter: char, version: Option<&str>) -> String {
    match version {
        Some(version) if !version.is_empty() => format!("SLOT {letter} \u{b7} {version}"),
        _ => format!("SLOT {letter}"),
    }
}

/// Draws UPDATE (`cc.js` `update`).
pub fn draw_update(canvas: &mut Canvas, view: &UpdateView) {
    let new_serves = view.stage.new_serves();
    canvas.lane_rails();
    canvas.lane_mid(view.stage.lane(), WHITE);
    let left = Rect::new(24, 82, 180, 84);
    let right = Rect::new(276, 82, 180, 84);
    // The slot that served before: solid while it serves, an outline after.
    if new_serves {
        let ink = if view.stage == Stage::Trial {
            BLUE_MD
        } else {
            G3
        };
        canvas.hollow(left, ink, 2, GROUND);
    } else {
        canvas.rect(left, BLUE);
    }
    canvas.text(
        &slot_label(view.before, (!new_serves).then_some(view.version.as_str())),
        34.0,
        105.0,
        Face::bold(16.0, if new_serves { G2 } else { BLUE_NIGHT }),
    );
    let (word, ink) = match (new_serves, view.stage) {
        (false, _) => ("RUNNING", WHITE),
        (true, Stage::Trial) => ("STANDS BY", BLUE_LT),
        (true, _) => ("SPARE", G2),
    };
    canvas.text(word, 33.0, 148.0, Face::bold(30.0, ink));
    // The new slot.
    match view.stage {
        Stage::Confirmed => canvas.rect(right, BLUE),
        Stage::RolledBack => canvas.hollow(right, G3, 2, GROUND),
        Stage::Trial => canvas.hollow(right, WHITE, 4, GROUND),
        Stage::Receiving => canvas.hollow(right, WHITE, 2, GROUND),
    }
    if view.stage == Stage::Receiving && view.fill_rows > 0 {
        canvas.rect(
            Rect::new(280, 162 - view.fill_rows, 172, view.fill_rows),
            BLUE_DK,
        );
    }
    let label_ink = match view.stage {
        Stage::Confirmed => BLUE_NIGHT,
        Stage::RolledBack => G3,
        Stage::Receiving | Stage::Trial => G1,
    };
    canvas.text(
        &slot_label(view.after, new_serves.then_some(view.version.as_str())),
        286.0,
        105.0,
        Face::bold(16.0, label_ink),
    );
    match view.stage {
        Stage::Receiving | Stage::Trial => {
            let px = if view.figure_big { 40.0 } else { 30.0 };
            let baseline = if view.figure_big { 152.0 } else { 148.0 };
            canvas.text(&view.figure, 285.0, baseline, Face::bold(px, WHITE));
        }
        Stage::Confirmed => canvas.text("RUNNING", 285.0, 148.0, Face::bold(30.0, WHITE)),
        Stage::RolledBack => canvas.text("REJECTED", 285.0, 148.0, Face::bold(30.0, G2)),
    }
    // The drops and the bus: the one that serves is solid white, the other dashed.
    for (x, live) in [(114_u16, !new_serves), (366, new_serves)] {
        if live {
            canvas.rect(Rect::new(x - 3, 166, 6, 50), WHITE);
        } else {
            canvas.rect(Rect::new(x - 2, 166, 4, 16), G4);
            canvas.rect(Rect::new(x - 2, 200, 4, 16), G4);
        }
    }
    canvas.rect(Rect::new(111, 212, 258, 6), G4);
    canvas.rect(
        Rect::new(if new_serves { 237 } else { 111 }, 212, 132, 6),
        WHITE,
    );
    canvas.rect(Rect::new(237, 212, 6, 30), WHITE);
    // DNS, unbroken. Its dot is drawn still (spec §12.4): no beat.
    canvas.rect(Rect::new(100, 242, 280, 60), BLUE);
    canvas.dot(124.0, 272.0, 8.0, BLUE_NIGHT);
    canvas.text("DNS ANSWERING", 144.0, 281.0, Face::bold(24.0, WHITE));
    let notes = match view.stage {
        Stage::Receiving => [
            format!("{} SERVES", view.before),
            format!("{} FILLS", view.after),
        ],
        Stage::Trial => [
            format!("{} SERVES", view.after),
            format!("{} WAITS", view.before),
        ],
        Stage::Confirmed | Stage::RolledBack => ["TAP FOR".to_string(), "HOME".to_string()],
    };
    canvas.text(&notes[0], 392.0, 268.0, Face::bold(16.0, G2));
    canvas.text(&notes[1], 392.0, 290.0, Face::bold(16.0, G2));
    if view.stage == Stage::RolledBack {
        canvas.text("NOTHING", 8.0, 268.0, Face::bold(16.0, G2));
        canvas.text("CHANGED", 8.0, 290.0, Face::bold(16.0, G2));
    }
}

// ---------------------------------------------------------------------------------------------
// NIGHT

/// What NIGHT puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchView {
    /// The hour: `02:14`, or `--:--` while the clock is not set.
    pub clock: String,
    /// `4 ON THE LOG`, in white, while alerts wait; `None` says `NIGHT WATCH` in the dark blue.
    pub log: Option<String>,
}

impl WatchView {
    /// What the dark clock says for a world.
    pub fn of(world: &World) -> Self {
        Self {
            clock: format::clock(world.time),
            log: (world.alerts_waiting > 0).then(|| waiting_words(world.alerts_waiting)),
        }
    }
}

/// The centre of the line under the hour: `NIGHT WATCH` in 16 px with 2 px tracking is 125.6 wide, starting at x 174.
const WATCH_WORDS_X: f32 = 236.8;

/// Draws NIGHT (`ca.js` `night`).
pub fn draw_watch(canvas: &mut Canvas, view: &WatchView) {
    paper(canvas, 4, 320, BLUE_NIGHT);
    canvas.text(&view.clock, 96.0, 204.0, Face::bold(96.0, BLUE_DEEP));
    let (words, ink) = match &view.log {
        Some(log) => (log.as_str(), WHITE),
        None => ("NIGHT WATCH", BLUE_DEEP),
    };
    canvas.text(
        words,
        WATCH_WORDS_X,
        244.0,
        Face::bold(16.0, ink).track(2.0).mid(),
    );
}

// ---------------------------------------------------------------------------------------------
// PIN

/// The pad's twelve keys in the order of round 8's `PAD`: three across, four down.
const PAD: [&str; 12] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "CANCEL", "0", "<",
];

/// One of the pad's keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadKey {
    /// A digit, 0 to 9.
    Digit(Digit),
    /// `CANCEL`: back to where the pad came from, the action not done.
    Cancel,
    /// `<`: takes the last digit back.
    Erase,
}

/// Where the pad's key `i` stands (`PADKEY`): 91 by 50, three across from x 184 and four down from y 80, 58 apart.
pub const fn pad_rect(i: usize) -> Rect {
    let x = [184, 282, 381][i % 3];
    Rect::new(x, 80 + (i / 3) as u16 * 58, 91, 50)
}

/// The key a press at `(x, y)` is on: each key's target is its rectangle and 4 px round it (`inR(.., x - 4, y - 4, 99,
/// 58)`); the first key whose target holds the press is the one (two targets share two columns).
pub fn pad_key_at(x: u16, y: u16) -> Option<PadKey> {
    (0..PAD.len()).find_map(|i| {
        let key = pad_rect(i);
        let target = Rect::new(key.x - 4, key.y - 4, key.w + 8, key.h + 8);
        target.contains(x, y).then(|| match i {
            9 => PadKey::Cancel,
            11 => PadKey::Erase,
            10 => PadKey::Digit(Digit::new(0)),
            digit => PadKey::Digit(Digit::new(digit as u8 + 1)),
        })
    })
}

/// What the pad's question says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// `ENTER THE PIN`.
    Enter,
    /// `CHOOSE A PIN`.
    Choose,
    /// `WRONG PIN`, until the next digit.
    Wrong,
    /// `WAIT m:ss`: the lock's backoff, which takes no digits.
    Wait(u32),
}

impl Ask {
    /// The words, as the slip prints them.
    fn text(&self) -> String {
        match self {
            Self::Enter => "ENTER THE PIN".to_string(),
            Self::Choose => "CHOOSE A PIN".to_string(),
            Self::Wrong => "WRONG PIN".to_string(),
            Self::Wait(secs) => format!("WAIT {}", mss(u16::try_from(*secs).unwrap_or(u16::MAX))),
        }
    }
}

/// What the PIN pad puts on the glass. `Debug` keeps what the slip is signed for out of it.
#[derive(Clone, PartialEq, Eq)]
pub struct PinView {
    /// The lane's words: `PIN TO ALLOW NAME`.
    pub lane: &'static str,
    /// What the pad asks.
    pub ask: Ask,
    /// How many digits are typed, 0 to 4: boxes filled. Which digits they are is nowhere here.
    pub filled: usize,
    /// What is signed for, under `FOR`: one to three lines of 16 letters.
    pub sub: Vec<String>,
}

impl fmt::Debug for PinView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinView")
            .field("lane", &self.lane)
            .field("ask", &self.ask)
            .field("filled", &self.filled)
            .finish_non_exhaustive()
    }
}

/// How many letters a line of what is signed for holds: the slip's column is 162 px of 9.6.
const FOR_CELLS: usize = 16;

/// A line of words on at most two lines of `FOR_CELLS`, broken at a space (`wrapWords`): a line that does not fit has
/// its start kept and its end cut with a `~`.
fn wrap_words(words: &str) -> Vec<String> {
    let words = fit_text(words, 2 * FOR_CELLS + 1);
    if words.chars().count() <= FOR_CELLS {
        return vec![words];
    }
    let letters: Vec<char> = words.chars().collect();
    let cut = (1..=FOR_CELLS).rev().find(|&at| letters[at] == ' ');
    match cut {
        Some(at) => vec![
            letters[..at].iter().collect(),
            fit_text(&letters[at + 1..].iter().collect::<String>(), FOR_CELLS),
        ],
        None => vec![fit_text(&words, FOR_CELLS)],
    }
}

/// The lane's words, and what is signed for, of the action the pad holds.
fn held_words(intent: &Intent, world: &World) -> (&'static str, Vec<String>) {
    let name_lines = |name: &str| {
        let [first, second] = wrap_name(name, FOR_CELLS);
        let mut lines = vec![first];
        if !second.is_empty() {
            lines.push(second);
        }
        lines
    };
    match intent {
        Intent::DeviceAds { mac, .. } => {
            let device = world
                .device(mac)
                .map_or_else(|| "a device".to_string(), |row| row.name.clone());
            ("PIN TO ALLOW ADS", wrap_words(&format!("on {device}")))
        }
        Intent::Name {
            name,
            verdict: Verdict::ClearRefuse,
        } => ("PIN TO ANSWER AGAIN", name_lines(name)),
        Intent::Name { name, .. } => ("PIN TO ALLOW NAME", name_lines(name)),
        Intent::Restart => ("PIN TO RESTART", vec!["restart the board".to_string()]),
        Intent::ClearPin => ("PIN TO UNLOCK", vec!["turn the lock off".to_string()]),
        Intent::SetPin(_) => ("PIN TO CHANGE PIN", vec!["choose a new PIN".to_string()]),
        _ => ("PIN TO CONFIRM", vec!["this change".to_string()]),
    }
}

impl PinView {
    /// The pad as the nav holds it, read with the world (the device it is signed for, the lock's wait). A pad the nav
    /// does not hold is the one that chooses a PIN, with nothing typed.
    pub fn of(pad: Option<&PinPad>, world: &World) -> Self {
        let Some(pad) = pad else {
            return Self {
                lane: "SET A NEW PIN",
                ask: Ask::Choose,
                filled: 0,
                sub: vec!["four digits".to_string()],
            };
        };
        let (lane, sub) = match pad.goal() {
            PinGoal::Do { intent, .. } => held_words(intent, world),
            PinGoal::Change => ("PIN TO CHANGE PIN", vec!["choose a new PIN".to_string()]),
            PinGoal::Choose => ("SET A NEW PIN", vec!["four digits".to_string()]),
        };
        let wait = pad.wait_secs(world);
        let ask = if wait > 0 {
            Ask::Wait(wait)
        } else if pad.wrong() {
            Ask::Wrong
        } else if matches!(pad.goal(), PinGoal::Choose) {
            Ask::Choose
        } else {
            Ask::Enter
        };
        Self {
            lane,
            ask,
            filled: pad.len(),
            sub,
        }
    }
}

/// The pad's taps: a digit is typed (the fourth asks to verify, or sets the PIN when choosing one), `<` takes the last
/// back, `CANCEL` goes back to where the pad came from. While the lock makes the owner wait, only `CANCEL` is a key.
pub fn pin_tap(pad: Option<&PinPad>, world: &World, x: u16, y: u16) -> Tap {
    let Some(key) = pad_key_at(x, y) else {
        return Tap::None;
    };
    let Some(pad) = pad else {
        return if key == PadKey::Cancel {
            Tap::Nav(Change::Back)
        } else {
            Tap::None
        };
    };
    let waiting = pad.wait_secs(world) > 0;
    match key {
        PadKey::Cancel => Tap::Nav(Change::Back),
        PadKey::Erase if waiting || pad.is_empty() => Tap::None,
        PadKey::Erase => Tap::Nav(Change::PinErase),
        PadKey::Digit(_) if waiting || pad.len() >= 4 => Tap::None,
        PadKey::Digit(digit) if pad.len() < 3 => Tap::Nav(Change::PinDigit(digit)),
        PadKey::Digit(digit) => {
            let digits = pad.digits_with(digit);
            match pad.goal() {
                // The fourth digit of a new PIN sets it, and the pad is done.
                PinGoal::Choose => Tap::Act(Intent::SetPin(digits), Some(Change::Back)),
                // The fourth digit of the PIN asked for goes to be checked; the answer comes back as `nav::verified`.
                PinGoal::Do { .. } | PinGoal::Change => {
                    Tap::Act(Intent::Verify(digits), Some(Change::PinDigit(digit)))
                }
            }
        }
    }
}

/// Draws the PIN pad (`ca.js` `pin`).
pub fn draw_pin(canvas: &mut Canvas, view: &PinView) {
    canvas.lane_rails();
    canvas.lane_mid(view.lane, WHITE);
    for y in (80..308).step_by(12) {
        canvas.rect(Rect::new(174, y, 2, 6), G4);
    }
    let ask_ink = if matches!(view.ask, Ask::Wrong | Ask::Wait(_)) {
        WHITE
    } else {
        G2
    };
    canvas.text(
        &view.ask.text(),
        6.4,
        106.0,
        Face::bold(16.0, ask_ink).track(1.5),
    );
    for i in 0..4_u16 {
        let typed = usize::from(i) < view.filled;
        canvas.hollow(
            Rect::new(8 + i * 40, 122, 32, 44),
            if typed { WHITE } else { G3 },
            2,
            GROUND,
        );
        if typed {
            canvas.rect(Rect::new(18 + i * 40, 138, 12, 12), WHITE);
        }
    }
    canvas.text("FOR", 8.0, 198.0, Face::regular(16.0, G2));
    for (k, line) in view.sub.iter().enumerate() {
        canvas.text(line, 8.0, 220.0 + 22.0 * k as f32, Face::bold(16.0, G1));
    }
    canvas.rect(Rect::new(8, 272, 150, 2), G3);
    canvas.text("THE OWNER", 8.0, 294.0, Face::regular(16.0, G3));
    for (i, glyph) in PAD.iter().enumerate() {
        let key = pad_rect(i);
        let (cx, top) = (f32::from(key.x) + f32::from(key.w) / 2.0, f32::from(key.y));
        if *glyph == "CANCEL" {
            canvas.hollow(key, G2, 4, GROUND);
            canvas.text(
                glyph,
                cx,
                top + 31.0,
                Face::bold(16.0, WHITE).track(1.0).mid(),
            );
        } else {
            // The keys look the same while the lock makes the owner wait (they take nothing then): a wait that dimmed
            // them would be a write of the whole pad, and the question says `WAIT m:ss`.
            canvas.hollow(key, G3, 2, GROUND);
            canvas.text(glyph, cx, top + 36.0, Face::bold(30.0, WHITE).mid());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rgb;
    use crate::os::fixture::{Still, mac_of, world};
    use crate::os::nav::Nav;
    use crate::os::world::{Slot, UpdateRead, Waiting};

    fn is(canvas: &Canvas, x: usize, y: usize, color: Rgb) -> bool {
        canvas.pixel(x, y) == Some(color)
    }

    fn drawn<V>(view: &V, draw: fn(&mut Canvas, &V)) -> Canvas {
        let mut canvas = Canvas::new();
        draw(&mut canvas, view);
        canvas
    }

    /// The ink of a box: how many pixels in it are not `ground`.
    fn ink(canvas: &Canvas, (x0, y0, x1, y1): (usize, usize, usize, usize), ground: Rgb) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.pixel(x, y) != Some(ground))
            .count()
    }

    // ---- STARTING ----

    #[test]
    fn the_checklist_ticks_what_the_readings_say_and_the_first_that_is_not_is_under_way() {
        let mut w = World::default();
        let steps = |w: &World| StartingView::of(w).steps;
        // The panel is on from its first frame; Wi-Fi is the one under way.
        assert_eq!(
            steps(&w),
            [
                Step::Done,
                Step::Now,
                Step::Waiting,
                Step::Waiting,
                Step::Waiting
            ]
        );
        w.wifi_joined = true;
        w.time = Some(crate::state::TimeOfDay {
            hour: 10,
            minute: 21,
        });
        assert_eq!(
            steps(&w),
            [Step::Done, Step::Done, Step::Done, Step::Now, Step::Waiting]
        );
        w.list_names = Some(212_000);
        w.dns_answering = true;
        assert_eq!(steps(&w), [Step::Done; 5]);
        assert_eq!(StartingView::of(&w).names, "212K NAMES");
        // A reading that is true is done whatever comes before it: DNS up while Wi-Fi is not yet.
        let early = World {
            dns_answering: true,
            ..World::default()
        };
        assert_eq!(
            steps(&early),
            [
                Step::Done,
                Step::Now,
                Step::Waiting,
                Step::Waiting,
                Step::Done
            ]
        );
        // Lists that hold no names are loaded all the same (none may be configured), and say so; lists that are
        // not known yet are not.
        let empty = World {
            list_names: Some(0),
            ..World::default()
        };
        assert_eq!(steps(&empty)[3], Step::Done);
        assert_eq!(StartingView::of(&empty).names, "0 NAMES");
        assert_eq!(steps(&World::default())[3], Step::Waiting);
    }

    #[test]
    fn the_checklist_is_drawn_as_the_paper_with_a_tick_a_line() {
        let w = world(Still::Boot);
        let canvas = drawn(&StartingView::of(&w), draw_starting);
        // The first line is done: a blue tick box at (34, 92) with its filled square, and `ON` at the right in G2.
        assert!(is(&canvas, 34, 92, BLUE) && is(&canvas, 46, 104, BLUE));
        assert!(ink(&canvas, (410, 100, 448, 114), GROUND) > 20, "ON");
        // The second is under way: a white frame, black inside. The third to fifth wait: a dim frame.
        assert!(is(&canvas, 34, 136, WHITE) && is(&canvas, 46, 148, GROUND));
        for y in [180, 224, 268] {
            assert!(is(&canvas, 34, y, G4), "{y}");
        }
        // Sprocket holes: the paper is there under the lane.
        assert!(is(&canvas, 11, 86, G4) && is(&canvas, 469, 86, G4));
        // The lane says it in the middle: 17 letters from cell 1.5.
        assert!(ink(&canvas, (36, 16, 444, 56), GROUND) > 600);
    }

    // ---- UPDATE ----

    fn update_world(phase: UpdatePhase, before: Slot, after: Slot, version: &str) -> World {
        World {
            update: Some(UpdateRead {
                phase,
                before,
                after,
            }),
            version: version.to_string(),
            ..World::default()
        }
    }

    #[test]
    fn the_changeover_names_the_slots_by_their_letters_and_the_version_of_the_one_that_runs() {
        let receiving = UpdateView::of(&update_world(
            UpdatePhase::Receiving {
                received: 3_100_000,
                length: 5_000_000,
            },
            Slot::B,
            Slot::A,
            "0.4.1",
        ));
        assert_eq!((receiving.before, receiving.after), ('B', 'A'));
        assert_eq!(receiving.stage, Stage::Receiving);
        assert_eq!(receiving.figure, "62%");
        assert_eq!(receiving.fill_rows, 47, "round(76 x 0.62)");
        assert_eq!(slot_label('B', Some("0.4.1")), "SLOT B \u{b7} 0.4.1");
        assert_eq!(slot_label('A', None), "SLOT A");
        assert_eq!(slot_label('A', Some("")), "SLOT A");
        // On trial the new slot is the one that runs, and the figure is the time to the minimum.
        let trial = UpdateView::of(&update_world(
            UpdatePhase::Trial {
                left_secs: 107,
                waiting_for: None,
            },
            Slot::A,
            Slot::B,
            "0.4.2",
        ));
        assert_eq!((trial.figure.as_str(), trial.figure_big), ("1:47", true));
        // Past the minimum, still waiting: the word it waits for, smaller. Not waiting, the time is 0:00.
        for (waiting, word) in [
            (Waiting::Wifi, "WI-FI"),
            (Waiting::Clock, "CLOCK"),
            (Waiting::Upstream, "UPSTREAM"),
        ] {
            let held = UpdateView::of(&update_world(
                UpdatePhase::Trial {
                    left_secs: 0,
                    waiting_for: Some(waiting),
                },
                Slot::A,
                Slot::B,
                "0.4.2",
            ));
            assert_eq!((held.figure.as_str(), held.figure_big), (word, false));
        }
        let up = UpdateView::of(&update_world(
            UpdatePhase::Trial {
                left_secs: 0,
                waiting_for: None,
            },
            Slot::A,
            Slot::B,
            "0.4.2",
        ));
        assert_eq!(up.figure, "0:00");
        // A minimum time still running is shown as the time, though the trial waits for something too.
        let both = UpdateView::of(&update_world(
            UpdatePhase::Trial {
                left_secs: 30,
                waiting_for: Some(Waiting::Clock),
            },
            Slot::A,
            Slot::B,
            "0.4.2",
        ));
        assert_eq!(both.figure, "0:30");
        // The world that has no update yet is an update that has begun.
        let none = UpdateView::of(&World::default());
        assert_eq!(
            (none.stage, none.figure.as_str(), none.fill_rows),
            (Stage::Receiving, "0%", 0)
        );
    }

    #[test]
    fn the_receiving_share_is_cut_and_the_fill_never_passes_the_box() {
        let share = |received, length| {
            let phase = UpdatePhase::Receiving { received, length };
            (
                phase.received().0,
                UpdateView::of(&update_world(phase, Slot::A, Slot::B, "")).fill_rows,
            )
        };
        assert_eq!(share(0, 0), (0, 0), "no length yet");
        assert_eq!(share(0, 1_000), (0, 0));
        assert_eq!(
            share(999, 1_000),
            (99, 76),
            "99.9 % reads 99, and fills 76 rows"
        );
        assert_eq!(share(1_000, 1_000), (100, 76));
        assert_eq!(
            share(5_000, 1_000),
            (100, 76),
            "more than the whole is the whole"
        );
        assert_eq!(share(1, 1_000), (0, 0));
        assert_eq!(share(u64::MAX, u64::MAX), (100, 76));
        assert!(
            !UpdatePhase::Trial {
                left_secs: 1,
                waiting_for: None
            }
            .is_done()
        );
        assert!(UpdatePhase::Confirmed.is_done() && UpdatePhase::RolledBack.is_done());
    }

    #[test]
    fn the_changeover_is_drawn_as_the_still_has_it_with_the_slot_letters_that_are_real() {
        // Receiving: A runs and is solid blue, B fills; the bus is white on A's side.
        let receiving = drawn(&UpdateView::of(&world(Still::UpdateReceiving)), draw_update);
        assert!(is(&receiving, 100, 100, BLUE), "the running slot is solid");
        assert!(is(&receiving, 366, 82, WHITE), "the new slot is an outline");
        assert!(
            is(&receiving, 440, 150, BLUE_DK),
            "the fill, from the foot up"
        );
        assert!(
            is(&receiving, 440, 100, GROUND),
            "above the fill the box is black"
        );
        assert!(is(&receiving, 114, 190, WHITE) && !is(&receiving, 366, 190, WHITE));
        // The confirmed update: B is solid and A an outline; the white drop is B's.
        let confirmed = drawn(&UpdateView::of(&world(Still::UpdateConfirmed)), draw_update);
        assert!(is(&confirmed, 366, 100, BLUE) && is(&confirmed, 24, 82, G3));
        assert!(is(&confirmed, 366, 190, WHITE) && !is(&confirmed, 114, 190, WHITE));
        // Rolled back: A runs, B is a dim outline, and `NOTHING CHANGED` stands at the left of DNS.
        let back = drawn(
            &UpdateView::of(&world(Still::UpdateRolledBack)),
            draw_update,
        );
        assert!(is(&back, 100, 100, BLUE) && is(&back, 276, 82, G3));
        assert!(
            ink(&back, (8, 255, 100, 296), GROUND) > 80,
            "NOTHING CHANGED"
        );
        assert_eq!(ink(&receiving, (8, 255, 100, 296), GROUND), 0);
        // DNS stands unbroken in every state, its dot dark and still.
        for canvas in [&receiving, &confirmed, &back] {
            assert!(is(canvas, 100, 242, BLUE) && is(canvas, 379, 301, BLUE));
            assert!(is(canvas, 124, 272, BLUE_NIGHT));
        }
        // A slot named B that serves before the update, and A that takes it: the letters follow.
        let swapped = UpdateView::of(&update_world(
            UpdatePhase::Trial {
                left_secs: 60,
                waiting_for: None,
            },
            Slot::B,
            Slot::A,
            "0.4.2",
        ));
        assert_eq!((swapped.before, swapped.after), ('B', 'A'));
    }

    #[test]
    fn the_trial_figure_and_the_receiving_fill_change_a_few_rows_a_write() {
        // One second of the trial: the figure's digits; one step of the fill: its edge and the share's digits.
        let trial = |left| {
            UpdateView::of(&update_world(
                UpdatePhase::Trial {
                    left_secs: left,
                    waiting_for: None,
                },
                Slot::A,
                Slot::B,
                "0.4.2",
            ))
        };
        let (a, b) = (
            drawn(&trial(107), draw_update),
            drawn(&trial(106), draw_update),
        );
        let rows: Vec<usize> = (0..320)
            .filter(|&y| (0..480).any(|x| a.pixel(x, y) != b.pixel(x, y)))
            .collect();
        let span = rows.last().unwrap() - rows.first().unwrap() + 1;
        assert!(span <= 40, "{span} rows");
    }

    // ---- NIGHT ----

    #[test]
    fn night_says_the_hour_and_the_watch_or_how_many_are_on_the_log() {
        let quiet = world(Still::Night);
        let view = WatchView::of(&quiet);
        assert_eq!((view.clock.as_str(), view.log.as_deref()), ("02:14", None));
        let mut waiting = quiet.clone();
        waiting.alerts_waiting = 4;
        assert_eq!(WatchView::of(&waiting).log.as_deref(), Some("4 ON THE LOG"));
        waiting.alerts_waiting = 1200;
        assert_eq!(
            WatchView::of(&waiting).log.as_deref(),
            Some("999+ ON THE LOG")
        );
        let unset = World::default();
        assert_eq!(WatchView::of(&unset).clock, "--:--");
    }

    #[test]
    fn night_is_two_blues_and_white_only_for_the_log_that_waits() {
        let quiet = drawn(&WatchView::of(&world(Still::Night)), draw_watch);
        let mut count = std::collections::HashMap::new();
        for y in 0..320 {
            for x in 0..480 {
                *count.entry(quiet.pixel(x, y).unwrap()).or_insert(0_u32) += 1;
            }
        }
        // Black, the margins' blue, the hour's blue, and the anti-aliased edges between them; never white or grey.
        assert!(
            count
                .keys()
                .all(|c| c[0] <= 0x12 && c[1] <= 0x1b && c[2] <= 0x44),
            "{count:?}"
        );
        assert!(is(&quiet, 11, 86, BLUE_NIGHT) && is(&quiet, 22, 4, BLUE_NIGHT));
        assert!(ink(&quiet, (96, 130, 384, 206), GROUND) > 3000, "the hour");
        let mut waiting = world(Still::Night);
        waiting.alerts_waiting = 4;
        let alert = drawn(&WatchView::of(&waiting), draw_watch);
        let white = (170..310)
            .flat_map(|x| (232..250).map(move |y| (x, y)))
            .filter(|&(x, y)| is(&alert, x, y, WHITE))
            .count();
        assert!(white > 40, "{white}");
        let none = (170..310)
            .flat_map(|x| (232..250).map(move |y| (x, y)))
            .filter(|&(x, y)| is(&quiet, x, y, WHITE))
            .count();
        assert_eq!(none, 0);
    }

    // ---- PIN ----

    #[test]
    fn the_pads_keys_are_round_8s_and_a_press_takes_the_first_key_whose_target_holds_it() {
        // Three across from x 184, 282, 381; four down from y 80, 58 apart; 91 by 50.
        for (i, (x, y)) in [
            (184, 80),
            (282, 80),
            (381, 80),
            (184, 138),
            (282, 138),
            (381, 138),
            (184, 196),
            (282, 196),
            (381, 196),
            (184, 254),
            (282, 254),
            (381, 254),
        ]
        .into_iter()
        .enumerate()
        {
            let key = pad_rect(i);
            assert_eq!((key.x, key.y, key.w, key.h), (x, y, 91, 50), "{i}");
        }
        let digits: Vec<Option<PadKey>> = (0..12)
            .map(|i| {
                let key = pad_rect(i);
                pad_key_at(key.x + 45, key.y + 25)
            })
            .collect();
        let want = [
            PadKey::Digit(Digit::new(1)),
            PadKey::Digit(Digit::new(2)),
            PadKey::Digit(Digit::new(3)),
            PadKey::Digit(Digit::new(4)),
            PadKey::Digit(Digit::new(5)),
            PadKey::Digit(Digit::new(6)),
            PadKey::Digit(Digit::new(7)),
            PadKey::Digit(Digit::new(8)),
            PadKey::Digit(Digit::new(9)),
            PadKey::Cancel,
            PadKey::Digit(Digit::new(0)),
            PadKey::Erase,
        ];
        assert_eq!(digits, want.map(Some));
        // The targets reach 4 px beyond a key; between the columns, the first column's holds the two shared pixels.
        assert_eq!(pad_key_at(180, 76), Some(PadKey::Digit(Digit::new(1))));
        assert_eq!(
            pad_key_at(278, 100),
            Some(PadKey::Digit(Digit::new(1))),
            "the one shared column"
        );
        assert_eq!(pad_key_at(279, 100), Some(PadKey::Digit(Digit::new(2))));
        assert_eq!(pad_key_at(179, 100), None);
        assert_eq!(pad_key_at(100, 100), None, "the slip is no key");
        assert_eq!(pad_key_at(200, 309), None);
    }

    #[test]
    fn the_lane_and_the_for_lines_say_what_is_signed_for() {
        let w = world(Still::Names);
        let allow = Intent::Name {
            name: "ads.example.net".into(),
            verdict: Verdict::Allow,
        };
        assert_eq!(
            held_words(&allow, &w),
            ("PIN TO ALLOW NAME", vec!["ads.example.net".to_string()])
        );
        let again = Intent::Name {
            name: "forum.example.com".into(),
            verdict: Verdict::ClearRefuse,
        };
        // (17 letters: one more than a line holds, so it goes on two, broken after a dot.)
        assert_eq!(
            held_words(&again, &w),
            (
                "PIN TO ANSWER AGAIN",
                vec!["forum.example.".to_string(), "com".to_string()]
            )
        );
        // A name past 16 letters goes on two lines, broken after a dot.
        let long = Intent::Name {
            name: "metrics.analytics.example.org".into(),
            verdict: Verdict::Allow,
        };
        let (_, lines) = held_words(&long, &w);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.chars().count() <= FOR_CELLS));
        // The device, by the word the wall uses for it.
        let sam = Intent::DeviceAds {
            mac: mac_of(23),
            pass: true,
        };
        let devices = world(Still::Devices);
        assert_eq!(
            held_words(&sam, &devices),
            ("PIN TO ALLOW ADS", vec!["on Sam's phone".to_string()])
        );
        let (_, gone) = held_words(
            &Intent::DeviceAds {
                mac: mac_of(99),
                pass: true,
            },
            &devices,
        );
        assert_eq!(gone, ["on a device"]);
        assert_eq!(
            held_words(&Intent::Restart, &w),
            ("PIN TO RESTART", vec!["restart the board".to_string()])
        );
        assert_eq!(
            held_words(&Intent::ClearPin, &w),
            ("PIN TO UNLOCK", vec!["turn the lock off".to_string()])
        );
        assert_eq!(
            held_words(
                &Intent::SetPin(crate::os::intent::PinDigits::new([1, 2, 3, 4])),
                &w
            )
            .0,
            "PIN TO CHANGE PIN"
        );
        // The lane's longest word fits the 20 cells.
        assert!("PIN TO ANSWER AGAIN".len() <= 20);
    }

    #[test]
    fn a_label_on_two_words_breaks_at_a_space_and_one_without_is_cut() {
        assert_eq!(wrap_words("on Desk PC"), ["on Desk PC"]);
        assert_eq!(wrap_words("on Living room TV"), ["on Living room", "TV"]);
        let one_word = wrap_words(&format!("on {}", "W".repeat(24)));
        assert_eq!(one_word.len(), 2, "{one_word:?}");
        assert!(
            one_word
                .iter()
                .all(|line| line.chars().count() <= FOR_CELLS)
        );
        let word = wrap_words(&"W".repeat(40));
        assert_eq!(word.len(), 1);
        assert_eq!(word[0].chars().count(), FOR_CELLS);
        assert!(word[0].ends_with('~'));
    }

    /// A pad over NAME, as the still has it: the lock on, ALLOW NAME held, two digits typed.
    fn pad_still() -> (Nav, World) {
        let w = world(Still::Pin);
        (Still::Pin.nav(&w), w)
    }

    #[test]
    fn the_pad_view_is_the_ask_the_boxes_and_what_is_signed_for_and_never_a_digit() {
        let (nav, w) = pad_still();
        let view = PinView::of(nav.pin(), &w);
        assert_eq!(view.lane, "PIN TO ALLOW NAME");
        assert_eq!(view.ask, Ask::Enter);
        assert_eq!(view.filled, 2);
        assert_eq!(view.sub, ["ads.example.net"]);
        let said = format!("{view:?}");
        assert!(!said.contains("example"), "{said}");
        assert!(
            said.contains("filled: 2"),
            "a count of the boxes, no digit: {said}"
        );
        // The words of each ask.
        assert_eq!(Ask::Enter.text(), "ENTER THE PIN");
        assert_eq!(Ask::Choose.text(), "CHOOSE A PIN");
        assert_eq!(Ask::Wrong.text(), "WRONG PIN");
        assert_eq!(Ask::Wait(30).text(), "WAIT 0:30");
        assert_eq!(Ask::Wait(900).text(), "WAIT 15:00");
        assert_eq!(
            Ask::Wait(u32::MAX).text(),
            "WAIT 99:59",
            "held to what the figure fits"
        );
        // No pad held: the one that chooses.
        let bare = PinView::of(None, &w);
        assert_eq!((bare.lane, bare.ask), ("SET A NEW PIN", Ask::Choose));
    }

    #[test]
    fn the_pad_is_drawn_as_the_slip_beside_the_keys() {
        let (nav, w) = pad_still();
        let canvas = drawn(&PinView::of(nav.pin(), &w), draw_pin);
        // The dashed divider at x 174, the four boxes (two typed: white with a square), the signature line.
        assert!(
            is(&canvas, 174, 80, G4) && is(&canvas, 175, 85, G4) && is(&canvas, 174, 86, GROUND)
        );
        assert!(is(&canvas, 8, 122, WHITE) && is(&canvas, 48, 122, WHITE));
        assert!(is(&canvas, 18, 138, WHITE) && is(&canvas, 58, 149, WHITE));
        assert!(is(&canvas, 88, 122, G3) && is(&canvas, 128, 122, G3));
        assert!(is(&canvas, 98, 138, GROUND), "no square in an empty box");
        assert!(is(&canvas, 8, 272, G3) && is(&canvas, 157, 273, G3));
        // The keys: a 2 px G3 frame, and CANCEL's 4 px G2 one.
        assert!(
            is(&canvas, 184, 80, G3) && is(&canvas, 184, 81, G3) && is(&canvas, 186, 82, GROUND)
        );
        assert!(
            is(&canvas, 184, 254, G2) && is(&canvas, 187, 257, G2) && is(&canvas, 188, 258, GROUND)
        );
        // The ask is G2 while it asks, white once it says WRONG.
        let ask_white = |canvas: &Canvas| {
            (6..170)
                .flat_map(|x| (92..108).map(move |y| (x, y)))
                .filter(|&(x, y)| is(canvas, x, y, WHITE))
                .count()
        };
        assert_eq!(ask_white(&canvas), 0);
        let mut wrong = PinView::of(nav.pin(), &w);
        wrong.ask = Ask::Wrong;
        assert!(ask_white(&drawn(&wrong, draw_pin)) > 30);
    }

    #[test]
    fn while_the_lock_makes_the_owner_wait_the_question_says_so_and_the_digits_take_nothing() {
        let (nav, mut w) = pad_still();
        w.pin_wait_secs = 30;
        let view = PinView::of(nav.pin(), &w);
        assert_eq!(view.ask, Ask::Wait(30));
        let canvas = drawn(&view, draw_pin);
        let at_rest = drawn(&PinView::of(nav.pin(), &world(Still::Pin)), draw_pin);
        // Only the question differs from the pad at rest: the keys are drawn as they were.
        let keys = |canvas: &Canvas| ink(canvas, (180, 76, 476, 308), GROUND);
        assert_eq!(keys(&canvas), keys(&at_rest));
        assert!(is(&canvas, 184, 254, G2), "CANCEL is as it was");
        for (x, y) in [(229, 105), (326, 279), (426, 279)] {
            assert_eq!(pin_tap(nav.pin(), &w, x, y), Tap::None, "({x}, {y})");
        }
        assert_eq!(
            pin_tap(nav.pin(), &w, 229, 279),
            Tap::Nav(Change::Back),
            "CANCEL still leaves"
        );
    }
}
