//! NAMES (spec §3.7, drawn as round 9's concept C, spec §12): a manifold. A pipe a name, as thick as it was
//! asked, ending at its valve: a solid blue valve is shut (the name is refused), a white outline is open, a
//! white plate across the pipe is sealed for good (a threat).
//!
//! - **NAMES** is `REFUSED MOST TODAY` with the door `YOUR OWN n ▸` and the day's three most refused names, the
//!   thickest pipe first; a row is a door to its NAME.
//! - **YOUR LIST** is `VALVES YOU SET`, the door `REFUSED MOST ▸` and the newest three entries of the owner's
//!   own list: a white pipe and an open valve for a name they allowed, a blue one and a shut valve for a name
//!   they refused, and on the right the day they set it on the panel (`TODAY`, `MON`; nothing for an entry
//!   pushed from the desktop, spec §12.8). Nothing set yet: `no valves set by you yet`.
//! - **NAME** is one name as a mimic: the pipe from `THE HOUSE` to the valve and the answer beyond it
//!   (`ANSWERED`, `NO ANSWER`, `SEALED`), what became of it today and which list holds it, and the floor in C's
//!   thin dress. No device stands on the pipe (privacy, spec §12.1): the Pi keeps no record of who asked what,
//!   so the left node is the house, inert.
//!
//! The keys: `KEEP REFUSED` + `ALLOW NAME` (strong) on a name the lists refuse, `KEEP ANSWERED` + `REFUSE NAME`
//! on one they answer, and for the owner's own entries `KEEP ALLOWED` + `REFUSE AGAIN` and `KEEP REFUSED` +
//! `ANSWER AGAIN` (strong, since it loosens), which take the entry back (spec §12.7). A threat has `BACK` and
//! `NEVER ALLOWED`. The padlock sits on the strong key, and on the valve's handle, while the lock is on.

use crate::format;
use crate::geometry::Rect;
use crate::os::intent::{Intent, Verdict};
use crate::os::nav::{Change, Screen, Section, Tap};
use crate::os::screens::lane::{self, LaneView};
use crate::os::screens::parts::{self, Dress, LIST_DOOR, Right, node};
use crate::os::world::{MAX_OWNER_ROWS, MAX_TOP_NAMES, NameClass, NameFacts, OwnerVerdict, World};
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    BLUE, BLUE_DK, BLUE_MD, FLOOR_SPLIT_X, FLOOR_Y, G1, G2, G3, G4, GROUND, WHITE,
};

/// A pipe's row: its top for the first, and the distance between two.
const PIPE_Y0: u16 = 136;
/// See `PIPE_Y0`.
const PIPE_PITCH: u16 = 56;

/// The first row a press on a pipe's door counts from: its top less the 2 px margin of `door(0, y - 2, 480, 56)`.
const PIPE_HIT_Y0: u16 = PIPE_Y0 - 2;

/// How many cells a pipe's name has: 24 px from x 7, clear of the count at its right.
const PIPE_NAME_CELLS: usize = 21;

/// How many cells the name on NAME has: 30 px from x 7.
const TITLE_CELLS: usize = 25;

/// How many cells the line under NAME's mimic has: 16 px from x 8 to the screen's edge.
const LINE_CELLS: usize = 49;

/// The pipe's thinnest, for a name that is open or has no count.
const PIPE_THIN: f32 = 6.0;

/// The pipe's share of the thickness: the thickest is `PIPE_THIN` plus this, for the most refused name.
const PIPE_SPAN: f32 = 10.0;

/// The three pipes' colours, by row: a shut pipe is blue, mid blue, dark blue.
const PIPE_BLUES: [crate::geometry::Rgb; 3] = [BLUE, BLUE_MD, BLUE_DK];

/// What state a valve is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Valve {
    /// Solid blue: the name is refused.
    Shut,
    /// A white outline: the name is answered.
    Open,
    /// A white plate across the pipe: a threat, which can never be let through.
    Sealed,
}

/// One pipe of NAMES or YOUR LIST, as it is drawn.
#[derive(Clone, PartialEq, Eq)]
pub struct PipeView {
    /// The name, fitted to `PIPE_NAME_CELLS` as a site.
    pub name: String,
    /// The word at the right of the name: the count (`1,204`), or the day (`MON`); empty for an entry with no day.
    pub right: String,
    /// Whether the valve is shut (and the pipe blue) rather than open (white).
    pub shut: bool,
    /// How thick a shut pipe is, in pixels, from 6 to 16: by how many times the name was refused.
    pub thickness: u8,
}

impl std::fmt::Debug for PipeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipeView")
            .field("shut", &self.shut)
            .field("thickness", &self.thickness)
            .finish_non_exhaustive()
    }
}

/// What NAMES puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamesView {
    /// The lane, which says `NAMES`.
    pub lane: LaneView,
    /// How many names the owner's list holds: the door's figure.
    pub owner_total: u32,
    /// The day's most refused names, most refused first.
    pub pipes: Vec<PipeView>,
}

/// What YOUR LIST puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YoursView {
    /// The lane, which says `NAMES`.
    pub lane: LaneView,
    /// The newest entries of the owner's list.
    pub pipes: Vec<PipeView>,
}

/// The thickness of a shut pipe for a name refused `count` times when the most refused was refused `max`:
/// `round(6 + 10 · count / max)`.
fn thickness(count: u32, max: u32) -> u8 {
    let share = f64::from(count) / f64::from(max.max(1));
    // From 6 to 16, so a byte holds it.
    (f64::from(PIPE_THIN) + f64::from(PIPE_SPAN) * share.min(1.0)).round() as u8
}

impl NamesView {
    /// What NAMES shows for a world.
    pub fn of(world: &World) -> Self {
        let max = world
            .top_names
            .iter()
            .map(|row| row.count)
            .max()
            .unwrap_or(1);
        Self {
            lane: LaneView::of(Section::Names, world),
            owner_total: world.owner_total,
            pipes: world
                .top_names
                .iter()
                .take(MAX_TOP_NAMES)
                .map(|row| PipeView {
                    name: format::fit_site(&row.name, PIPE_NAME_CELLS),
                    right: format::count(row.count),
                    shut: !row.open,
                    thickness: thickness(row.count, max),
                })
                .collect(),
        }
    }
}

impl YoursView {
    /// What YOUR LIST shows for a world: the newest entries, each a valve the owner set.
    pub fn of(world: &World) -> Self {
        Self {
            lane: LaneView::of(Section::Names, world),
            pipes: world
                .owner_newest
                .iter()
                .take(MAX_OWNER_ROWS)
                .map(|row| PipeView {
                    name: format::fit_site(&row.name, PIPE_NAME_CELLS),
                    right: row
                        .day
                        .map_or_else(String::new, |day| day.caps().to_string()),
                    shut: row.verdict == OwnerVerdict::Refuse,
                    thickness: PIPE_THIN as u8,
                })
                .collect(),
        }
    }
}

/// A valve (`valve`): two triangles tip to tip, 2·`hw` wide and 2·`hh` tall at `(cx, cy)`. Shut is solid blue, open a
/// white outline (a white pair with a black pair 2 px in), sealed a white plate across the pipe with two
/// rivets.
fn valve(canvas: &mut Canvas, cx: f32, cy: f32, hw: f32, hh: f32, state: Valve) {
    let pair = |canvas: &mut Canvas, color, k: f32| {
        canvas.tri(
            (cx - hw + k * 1.6, cy - hh + k * 2.6),
            (cx - hw + k * 1.6, cy + hh - k * 2.6),
            (cx - k * 1.4, cy),
            color,
        );
        canvas.tri(
            (cx + hw - k * 1.6, cy - hh + k * 2.6),
            (cx + hw - k * 1.6, cy + hh - k * 2.6),
            (cx + k * 1.4, cy),
            color,
        );
    };
    match state {
        Valve::Sealed => {
            canvas.rect_f(cx - 7.0, cy - hh - 2.0, 14.0, hh * 2.0 + 4.0, WHITE);
            canvas.dot(cx, cy - hh + 6.0, 2.5, GROUND);
            canvas.dot(cx, cy + hh - 6.0, 2.5, GROUND);
        }
        Valve::Shut => pair(canvas, BLUE, 0.0),
        Valve::Open => {
            pair(canvas, WHITE, 0.0);
            pair(canvas, GROUND, 2.0);
        }
    }
}

/// One row of pipe (`pipeRow`), the `k`th: the name, the figure at its right, the pipe, the valve and the
/// stub of pipe beyond it.
fn draw_pipe(canvas: &mut Canvas, k: usize, pipe: &PipeView) {
    let y = f32::from(PIPE_Y0) + f32::from(PIPE_PITCH) * k as f32;
    canvas.text(&pipe.name, 7.0, y + 23.0, Face::bold(24.0, WHITE));
    canvas.text(&pipe.right, 384.0, y + 22.0, Face::bold(20.0, G1).end());
    let th = if pipe.shut {
        f32::from(pipe.thickness)
    } else {
        PIPE_THIN
    };
    let color = if pipe.shut {
        PIPE_BLUES[k.min(PIPE_BLUES.len() - 1)]
    } else {
        WHITE
    };
    canvas.rect_f(8.0, y + 38.0 - th / 2.0, 384.0, th, color);
    valve(
        canvas,
        416.0,
        y + 38.0,
        24.0,
        16.0,
        if pipe.shut { Valve::Shut } else { Valve::Open },
    );
    canvas.rect_f(
        440.0,
        y + 36.0,
        32.0,
        4.0,
        if pipe.shut { G4 } else { WHITE },
    );
}

/// Draws NAMES.
pub fn draw_names(canvas: &mut Canvas, view: &NamesView) {
    lane::draw(canvas, &view.lane);
    canvas.text(
        "REFUSED MOST TODAY",
        8.0,
        100.0,
        Face::bold(16.0, G1).track(2.0),
    );
    canvas.text(
        "shut valve \u{b7} none passes",
        8.0,
        122.0,
        Face::regular(16.0, G2),
    );
    parts::list_door(canvas, "YOUR OWN", Some(view.owner_total));
    for (k, pipe) in view.pipes.iter().enumerate() {
        draw_pipe(canvas, k, pipe);
    }
    if view.pipes.is_empty() {
        canvas.text(
            "nothing refused yet today",
            8.0,
            170.0,
            Face::regular(18.0, G3),
        );
    }
}

/// Draws YOUR LIST.
pub fn draw_yours(canvas: &mut Canvas, view: &YoursView) {
    lane::draw(canvas, &view.lane);
    canvas.text(
        "VALVES YOU SET",
        8.0,
        100.0,
        Face::bold(16.0, G1).track(2.0),
    );
    canvas.text(
        "these overrule the lists",
        8.0,
        122.0,
        Face::regular(16.0, G2),
    );
    parts::list_door(canvas, "REFUSED MOST", None);
    for (k, pipe) in view.pipes.iter().enumerate() {
        draw_pipe(canvas, k, pipe);
    }
    if view.pipes.is_empty() {
        canvas.text(
            "no valves set by you yet",
            8.0,
            170.0,
            Face::regular(18.0, G3),
        );
    }
}

/// The pipe a press at row `y` is on: the rows from 134, 56 apart; `None` above, below the three, or beyond `held`.
fn pipe_hit(held: usize, y: u16) -> Option<usize> {
    let k = usize::from(y.checked_sub(PIPE_HIT_Y0)? / PIPE_PITCH);
    (k < 3 && k < held).then_some(k)
}

/// NAMES' taps: the door `YOUR OWN` opens YOUR LIST; a pipe opens its name.
pub fn names_tap(world: &World, x: u16, y: u16) -> Tap {
    if LIST_DOOR.contains(x, y) {
        return Tap::Nav(Change::Open(Screen::Yours));
    }
    pipe_hit(world.top_names.len(), y).map_or(Tap::None, |k| {
        Tap::Nav(Change::Open(Screen::Name(world.top_names[k].name.clone())))
    })
}

/// YOUR LIST's taps: the door `REFUSED MOST` goes back to NAMES; a pipe opens its name.
pub fn yours_tap(world: &World, x: u16, y: u16) -> Tap {
    if LIST_DOOR.contains(x, y) {
        return Tap::Nav(Change::Back);
    }
    pipe_hit(world.owner_newest.len(), y).map_or(Tap::None, |k| {
        Tap::Nav(Change::Open(Screen::Name(
            world.owner_newest[k].name.clone(),
        )))
    })
}

// ---------------------------------------------------------------------------------------------
// NAME.

/// The facts of the name a NAME screen shows, if the loop has read them: `None` for a name the world holds none
/// for, which is the moment between the screen opening and the read, and the screen draws the bare lane.
fn facts_of<'a>(world: &'a World, name: &str) -> Option<&'a NameFacts> {
    world.name_facts.as_ref().filter(|facts| facts.name == name)
}

/// What the floor's right key of NAME does, for a name: `None` for a threat, which is never allowed.
///
/// A name the lists refuse can be allowed (strong); one they answer can be refused; and an entry of the
/// owner's own can be taken back: `REFUSE AGAIN` for an allow (stricter: never asks) and `ANSWER AGAIN` for
/// a refusal (it loosens: strong).
pub fn right_verdict(facts: &NameFacts) -> Option<Verdict> {
    if facts.class == NameClass::Threat {
        return None;
    }
    Some(match facts.owner.map(|entry| entry.verdict) {
        Some(OwnerVerdict::Allow) => Verdict::ClearAllow,
        Some(OwnerVerdict::Refuse) => Verdict::ClearRefuse,
        None if facts.shut() => Verdict::Allow,
        None => Verdict::Refuse,
    })
}

/// The right key's word for a verdict.
const fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Allow => "ALLOW NAME",
        Verdict::Refuse => "REFUSE NAME",
        Verdict::ClearAllow => "REFUSE AGAIN",
        Verdict::ClearRefuse => "ANSWER AGAIN",
    }
}

/// The left key's word: what staying as things are says.
fn left_word(facts: &NameFacts) -> &'static str {
    if facts.class == NameClass::Threat {
        "BACK"
    } else if facts.shut() {
        "KEEP REFUSED"
    } else if facts.owner.is_some() {
        "KEEP ALLOWED"
    } else {
        "KEEP ANSWERED"
    }
}

/// Which list holds a name, as the line says it.
const fn list_word(class: NameClass) -> &'static str {
    match class {
        NameClass::Threat => "on a threat list",
        NameClass::Ads => "on the ads list",
        NameClass::Deny => "on the deny list",
        NameClass::None => "on no list",
    }
}

/// The line under the mimic (`nameFacts`): what became of the name and which list holds it, as one line. A
/// threat says it is never allowed instead.
fn line_of(facts: &NameFacts) -> String {
    if facts.class == NameClass::Threat {
        return "On a threat list. Threats are never allowed.".to_string();
    }
    let shut = facts.shut();
    let did = if shut { "refused" } else { "answered" };
    let what = match (facts.owner, facts.count) {
        // The owner's entry: since when, as the panel set it (nothing for one pushed from the desktop).
        (Some(entry), _) => format!(
            "{} by you{}",
            if shut { "refused" } else { "allowed" },
            entry.day.map_or_else(String::new, |day| day.since())
        ),
        (None, Some(count)) => format!("{did} {} times today", format::count(count)),
        (None, None) => format!("{did} today"),
    };
    let mut chars = what.chars();
    let sentence = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect::<String>())
        .unwrap_or_default();
    format::fit_text(
        &format!("{sentence} \u{b7} {}.", list_word(facts.class)),
        LINE_CELLS,
    )
}

/// What NAME puts on the glass.
#[derive(Clone, PartialEq, Eq)]
pub struct NameView {
    /// The lane: the section the panel came by (`NAMES`, or `ALERTS` for a threat opened from its slip).
    pub lane: LaneView,
    /// The name, fitted to `TITLE_CELLS` as a site.
    pub title: String,
    /// The valve's state: sealed for a threat, shut while the name is refused, else open.
    pub valve: Valve,
    /// The line under the mimic.
    pub line: String,
    /// The left key's word.
    pub left: &'static str,
    /// The right key: a verdict and its word, or `None` for a threat.
    pub right: Option<Verdict>,
    /// Whether the lock is on: the strong key and a shut valve's handle wear the padlock.
    pub lock_on: bool,
}

impl std::fmt::Debug for NameView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameView")
            .field("valve", &self.valve)
            .field("left", &self.left)
            .field("right", &self.right)
            .field("lock_on", &self.lock_on)
            .finish_non_exhaustive()
    }
}

impl NameView {
    /// What NAME shows for the name with this text under `root`: `None` while the world holds no facts for it.
    pub fn of(world: &World, root: Section, name: &str) -> Option<Self> {
        let facts = facts_of(world, name)?;
        Some(Self {
            lane: LaneView::of(root, world),
            title: format::fit_site(&facts.name, TITLE_CELLS),
            valve: if facts.class == NameClass::Threat {
                Valve::Sealed
            } else if facts.shut() {
                Valve::Shut
            } else {
                Valve::Open
            },
            line: line_of(facts),
            left: left_word(facts),
            right: right_verdict(facts),
            lock_on: world.lock_on,
        })
    }

    /// Whether the right key is strong and the lock is on: it wears the padlock.
    fn right_locked(&self) -> bool {
        self.lock_on
            && self.right.is_some_and(|verdict| {
                Intent::Name {
                    name: String::new(),
                    verdict,
                }
                .is_strong()
            })
    }
}

/// NAME's taps: the floor's left key keeps things as they are and goes back; the right key carries out its
/// verdict and goes back. The strong ones are marked by `Intent::is_strong`, and `nav::tap` holds them in the PIN pad while the lock is on.
pub fn name_tap(world: &World, name: &str, x: u16, y: u16) -> Tap {
    let Some(facts) = facts_of(world, name) else {
        return Tap::None;
    };
    if y < FLOOR_Y {
        return Tap::None;
    }
    if x < FLOOR_SPLIT_X {
        return Tap::Nav(Change::Back);
    }
    right_verdict(facts).map_or(Tap::None, |verdict| {
        Tap::Act(
            Intent::Name {
                name: facts.name.clone(),
                verdict,
            },
            Some(Change::Back),
        )
    })
}

/// Draws NAME.
pub fn draw_name(canvas: &mut Canvas, view: &NameView) {
    lane::draw(canvas, &view.lane);
    canvas.text(&view.title, 7.0, 108.0, Face::bold(30.0, WHITE));
    let shut = view.valve != Valve::Open;
    let pipe = if shut { BLUE } else { WHITE };
    // The house, where every name comes from: inert (spec §12.1).
    node(
        canvas,
        Rect::new(8, 146, 150, 52),
        "THE HOUSE",
        WHITE,
        WHITE,
    );
    canvas.rect(Rect::new(158, 169, 96, 6), pipe);
    if view.valve == Valve::Sealed {
        valve(canvas, 288.0, 172.0, 34.0, 24.0, Valve::Sealed);
    } else {
        // The stem and its handle.
        canvas.rect(Rect::new(286, 134, 4, 38), pipe);
        canvas.rect(Rect::new(268, 130, 40, 6), pipe);
        valve(canvas, 288.0, 172.0, 34.0, 24.0, view.valve);
        if shut && view.lock_on {
            parts::padlock(canvas, 316, 124, WHITE, GROUND);
        }
    }
    if shut {
        // Nothing comes through.
        for x in (326..350).step_by(10) {
            canvas.rect(Rect::new(x, 170, 5, 4), G4);
        }
    } else {
        canvas.rect(Rect::new(322, 169, 28, 6), WHITE);
    }
    if view.valve == Valve::Sealed {
        canvas.rect(Rect::new(254, 169, 27, 6), BLUE);
    }
    let (word, color, ink) = match view.valve {
        Valve::Open => ("ANSWERED", WHITE, WHITE),
        Valve::Sealed => ("SEALED", G4, G3),
        Valve::Shut => ("NO ANSWER", G4, G3),
    };
    node(canvas, Rect::new(350, 146, 122, 52), word, color, ink);
    canvas.text(&view.line, 8.0, 226.0, Face::regular(16.0, G1));
    let right = match view.right {
        Some(verdict) => Right::Key(verdict_word(verdict)),
        None => Right::Inert("NEVER ALLOWED"),
    };
    parts::keys9(
        canvas,
        Dress::C,
        Some(view.left),
        right,
        view.right_locked(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::os::world::{DayWord, NameRow, OwnerEntry, OwnerRow, Weekday};

    fn is(canvas: &Canvas, x: usize, y: usize, color: crate::geometry::Rgb) -> bool {
        canvas.pixel(x, y) == Some(color)
    }

    fn facts(class: NameClass, owner: Option<(OwnerVerdict, Option<DayWord>)>) -> NameFacts {
        NameFacts {
            name: "ads.example.net".into(),
            class,
            owner: owner.map(|(verdict, day)| OwnerEntry { verdict, day }),
            count: Some(1204),
        }
    }

    #[test]
    fn the_pipes_are_as_thick_as_the_name_was_refused_and_blue_by_row() {
        let view = NamesView::of(&world(Still::Names));
        assert_eq!(view.lane.words, "NAMES");
        assert_eq!(view.owner_total, 2);
        let names: Vec<&str> = view.pipes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ads.example.net",
                "tracker.example.com",
                "metrics.example.org"
            ]
        );
        let rights: Vec<&str> = view.pipes.iter().map(|p| p.right.as_str()).collect();
        assert_eq!(rights, ["1,204", "861", "540"]);
        // round(6 + 10 n / max): 16, 13 (6 + 7.15), 11 (6 + 4.49).
        let thick: Vec<u8> = view.pipes.iter().map(|p| p.thickness).collect();
        assert_eq!(thick, [16, 13, 10]);
        assert!(view.pipes.iter().all(|p| p.shut));
        assert_eq!(
            thickness(0, 0),
            6,
            "nothing refused: the thinnest, and no division by zero"
        );
        assert_eq!(thickness(5, 3), 16, "more than the max is held to it");
    }

    #[test]
    fn the_manifold_draws_a_pipe_a_valve_and_a_stub_for_each_name() {
        let mut canvas = Canvas::new();
        draw_names(&mut canvas, &NamesView::of(&world(Still::Names)));
        // Row 0 (y 136): the blue pipe 16 thick centred on y 174, the shut valve at x 416, the stub in G4.
        assert!(
            is(&canvas, 100, 174, BLUE)
                && is(&canvas, 100, 166, BLUE)
                && is(&canvas, 100, 181, BLUE)
        );
        assert!(is(&canvas, 100, 164, GROUND) && is(&canvas, 100, 183, GROUND));
        assert!(
            is(&canvas, 405, 174, BLUE) && is(&canvas, 427, 174, BLUE),
            "the valve"
        );
        assert!(is(&canvas, 450, 174, G4) && is(&canvas, 450, 173, G4));
        // Row 1 and row 2 are the mid and the dark blue.
        assert!(is(&canvas, 100, 230, BLUE_MD));
        assert!(is(&canvas, 100, 286, BLUE_DK));
        // The header door and the two header lines.
        assert!(is(&canvas, 272, 80, G2));
        assert!((8..260).any(|x| (88..102).any(|y| canvas.pixel(x, y) == Some(G1))));
    }

    #[test]
    fn your_list_has_a_valve_for_each_entry_white_open_or_blue_shut_and_the_day_it_was_set() {
        let w = world(Still::Yours);
        let view = YoursView::of(&w);
        let rights: Vec<&str> = view.pipes.iter().map(|p| p.right.as_str()).collect();
        assert_eq!(rights, ["MON", "SUN"]);
        assert_eq!(
            view.pipes.iter().map(|p| p.shut).collect::<Vec<_>>(),
            [false, true],
            "shop-ads was allowed (open), forum refused (shut)"
        );
        let mut canvas = Canvas::new();
        draw_yours(&mut canvas, &view);
        // The allowed one is a thin white pipe with a white stub; the refused one the mid blue (row 1's) with a dark stub.
        assert!(
            is(&canvas, 100, 174, WHITE)
                && is(&canvas, 100, 170, GROUND)
                && is(&canvas, 100, 177, GROUND)
                && is(&canvas, 450, 174, WHITE)
        );
        assert!(is(&canvas, 100, 230, BLUE_MD) && is(&canvas, 450, 230, G4));
        // The entry pushed from the desktop has no day; the entry set today says `TODAY`.
        let mut changed = w.clone();
        changed.owner_newest[0].day = None;
        changed.owner_newest[1].day = Some(DayWord::Today);
        let pipes = YoursView::of(&changed).pipes;
        assert_eq!(
            (pipes[0].right.as_str(), pipes[1].right.as_str()),
            ("", "TODAY")
        );
    }

    #[test]
    fn an_empty_list_says_so_in_the_mockups_words() {
        let mut w = world(Still::Yours);
        w.owner_newest.clear();
        let mut canvas = Canvas::new();
        draw_yours(&mut canvas, &YoursView::of(&w));
        assert!((8..250).any(|x| (156..172).any(|y| canvas.pixel(x, y) == Some(G3))));
        let mut none = world(Still::Names);
        none.top_names.clear();
        let mut canvas = Canvas::new();
        draw_names(&mut canvas, &NamesView::of(&none));
        assert!((8..250).any(|x| (156..172).any(|y| canvas.pixel(x, y) == Some(G3))));
    }

    #[test]
    fn a_pipe_is_a_door_to_its_name_and_the_header_door_is_the_other_list() {
        let w = world(Still::Names);
        let open = |name: &str| Tap::Nav(Change::Open(Screen::Name(name.into())));
        assert_eq!(names_tap(&w, 100, 150), open("ads.example.net"));
        assert_eq!(names_tap(&w, 100, 200), open("tracker.example.com"));
        assert_eq!(names_tap(&w, 479, 301), open("metrics.example.org"));
        assert_eq!(names_tap(&w, 0, 134), open("ads.example.net"));
        for (x, y) in [(100, 133), (100, 302), (100, 110), (200, 90)] {
            assert_eq!(names_tap(&w, x, y), Tap::None, "({x}, {y})");
        }
        // The door: 264 to 480 by 72 to 132.
        for (x, y) in [(264, 72), (479, 131), (300, 100)] {
            assert_eq!(
                names_tap(&w, x, y),
                Tap::Nav(Change::Open(Screen::Yours)),
                "({x}, {y})"
            );
        }
        assert_eq!(names_tap(&w, 263, 100), Tap::None);
        // YOUR LIST: its door goes back; its two pipes open names; the third place is nothing.
        let y = world(Still::Yours);
        assert_eq!(yours_tap(&y, 300, 100), Tap::Nav(Change::Back));
        assert_eq!(yours_tap(&y, 100, 150), open("shop-ads.example.net"));
        assert_eq!(yours_tap(&y, 100, 200), open("forum.example.com"));
        assert_eq!(yours_tap(&y, 100, 260), Tap::None);
        assert_eq!(names_tap(&World::default(), 100, 150), Tap::None);
    }

    #[test]
    fn name_is_a_mimic_from_the_house_through_a_valve_to_the_answer() {
        let w = world(Still::Name);
        let view = NameView::of(&w, Section::Names, "ads.example.net").expect("facts");
        assert_eq!(view.valve, Valve::Shut);
        assert_eq!(
            view.line,
            "Refused 1,204 times today \u{b7} on the ads list."
        );
        assert_eq!(
            (view.left, view.right),
            ("KEEP REFUSED", Some(Verdict::Allow))
        );
        let mut canvas = Canvas::new();
        draw_name(&mut canvas, &view);
        // THE HOUSE is white and no device stands there; the pipe is blue to the shut valve at x 288.
        assert!(is(&canvas, 8, 146, WHITE) && is(&canvas, 157, 197, WHITE));
        assert!(is(&canvas, 200, 172, BLUE) && is(&canvas, 275, 172, BLUE));
        assert!(is(&canvas, 288, 134, BLUE), "the stem");
        assert!(is(&canvas, 270, 132, BLUE), "the handle");
        // Nothing comes out: dashes in G4, and the answer node says NO ANSWER in G3.
        assert!(is(&canvas, 327, 171, G4) && is(&canvas, 337, 171, G4));
        assert!(is(&canvas, 350, 146, G4));
        assert!((360..470).any(|x| (160..190).any(|y| canvas.pixel(x, y) == Some(G3))));
        // No lock on: no padlock on the handle (white at (316..332, 124..142)).
        assert!(
            !(316..332)
                .flat_map(|x| (124..142).map(move |y| (x, y)))
                .any(|(x, y)| is(&canvas, x, y, WHITE))
        );
    }

    #[test]
    fn name_has_no_device_on_its_pipe() {
        // Privacy (spec §12.1): the left node is the house for every name, whatever the world knows.
        for still in [Still::Name, Still::NameThreat] {
            let w = world(still);
            let name = w.name_facts.as_ref().expect("facts").name.clone();
            let view = NameView::of(&w, Section::Names, &name).expect("facts");
            let mut canvas = Canvas::new();
            draw_name(&mut canvas, &view);
            // The node's label is 9 letters of 9.6 px centred on 83: it stands 40 to 126, and is white.
            assert!((40..126).any(|x| (156..190).any(|y| is(&canvas, x, y, WHITE))));
            let said = format!("{view:?}");
            assert!(!said.contains("example") && !said.contains("Sam"), "{said}");
        }
    }

    #[test]
    fn the_lock_puts_a_padlock_on_the_strong_key_and_on_the_handle_of_a_shut_valve() {
        let mut locked = world(Still::NameLocked);
        assert!(locked.lock_on);
        let view = NameView::of(&locked, Section::Names, "ads.example.net").expect("facts");
        let mut canvas = Canvas::new();
        draw_name(&mut canvas, &view);
        // The handle's padlock: white at (316.., 124..) with a hole; and a black one on the white key.
        assert!(
            is(&canvas, 320, 135, WHITE),
            "the body of the handle's padlock"
        );
        assert!(
            (272..284).any(|x| (262..281).any(|y| is(&canvas, x, y, GROUND))),
            "the key's padlock, left of its word"
        );
        // An open valve has no handle padlock, and `REFUSE NAME` is not strong, so no padlock on the key either.
        locked.name_facts = Some(facts(NameClass::None, None));
        let open = NameView::of(&locked, Section::Names, "ads.example.net").expect("facts");
        assert_eq!(open.valve, Valve::Open);
        let mut plain = Canvas::new();
        draw_name(&mut plain, &open);
        assert!(!is(&plain, 320, 135, WHITE));
        assert!((250..274).all(|x| (262..281).all(|y| is(&plain, x, y, WHITE))));
    }

    #[test]
    fn the_keys_for_every_state_of_a_name() {
        use NameClass::{Ads, Deny, None as NoList, Threat};
        use OwnerVerdict::{Allow, Refuse};
        let day = Some(DayWord::On(Weekday::Mon));
        // (the name's facts, the left word, the right verdict and word, the valve)
        let cases = [
            (
                facts(Ads, None),
                "KEEP REFUSED",
                Some(Verdict::Allow),
                Valve::Shut,
            ),
            (
                facts(Deny, None),
                "KEEP REFUSED",
                Some(Verdict::Allow),
                Valve::Shut,
            ),
            (
                facts(NoList, None),
                "KEEP ANSWERED",
                Some(Verdict::Refuse),
                Valve::Open,
            ),
            (
                facts(Ads, Some((Allow, day))),
                "KEEP ALLOWED",
                Some(Verdict::ClearAllow),
                Valve::Open,
            ),
            (
                facts(NoList, Some((Refuse, day))),
                "KEEP REFUSED",
                Some(Verdict::ClearRefuse),
                Valve::Shut,
            ),
            (facts(Threat, None), "BACK", None, Valve::Sealed),
            (
                facts(Threat, Some((Allow, None))),
                "BACK",
                None,
                Valve::Sealed,
            ),
        ];
        for (facts, left, right, valve) in cases {
            let w = World {
                name_facts: Some(facts.clone()),
                ..World::default()
            };
            let view = NameView::of(&w, Section::Names, "ads.example.net").expect("facts");
            assert_eq!(
                (view.left, view.right, view.valve),
                (left, right, valve),
                "{facts:?}"
            );
        }
        assert_eq!(
            [
                Verdict::Allow,
                Verdict::Refuse,
                Verdict::ClearAllow,
                Verdict::ClearRefuse
            ]
            .map(verdict_word),
            ["ALLOW NAME", "REFUSE NAME", "REFUSE AGAIN", "ANSWER AGAIN"]
        );
    }

    #[test]
    fn a_threat_has_back_and_never_allowed_and_a_sealed_plate_and_no_key() {
        let w = world(Still::NameThreat);
        let name = "secure-login.example.org";
        let view = NameView::of(&w, Section::Alerts, name).expect("facts");
        assert_eq!(
            view.lane.words, "ALERTS",
            "opened from the slip, the lane says where it came from"
        );
        assert_eq!(view.line, "On a threat list. Threats are never allowed.");
        assert_eq!(view.valve, Valve::Sealed);
        let mut canvas = Canvas::new();
        draw_name(&mut canvas, &view);
        // The plate: white 14 wide across the pipe at x 281 to 295; two black rivets; a short blue piece before it.
        assert!(
            is(&canvas, 282, 150, WHITE)
                && is(&canvas, 288, 154, GROUND)
                && is(&canvas, 288, 190, GROUND)
        );
        assert!(is(&canvas, 260, 171, BLUE));
        // No stem or handle for a sealed pipe.
        assert!(is(&canvas, 288, 132, GROUND) && is(&canvas, 270, 132, GROUND));
        // The right key is a dim outline: no white in it.
        assert!(
            !(250..470)
                .flat_map(|x| (242..302).map(move |y| (x, y)))
                .any(|(x, y)| is(&canvas, x, y, WHITE))
        );
        // Taps: BACK goes back; the right half does nothing.
        assert_eq!(name_tap(&w, name, 100, 280), Tap::Nav(Change::Back));
        assert_eq!(name_tap(&w, name, 400, 280), Tap::None);
    }

    #[test]
    fn the_right_key_carries_the_verdict_and_goes_back_and_only_the_strong_ones_are_marked() {
        let name = "ads.example.net";
        let mut w = world(Still::Name);
        let right = |w: &World| name_tap(w, name, 400, 280);
        let act = |verdict| {
            Tap::Act(
                Intent::Name {
                    name: name.into(),
                    verdict,
                },
                Some(Change::Back),
            )
        };
        assert_eq!(right(&w), act(Verdict::Allow));
        assert_eq!(name_tap(&w, name, 100, 280), Tap::Nav(Change::Back), "KEEP");
        assert_eq!(name_tap(&w, name, 239, 236), Tap::Nav(Change::Back));
        assert_eq!(name_tap(&w, name, 240, 236), act(Verdict::Allow));
        // Above the floor is nothing: the mimic has no door, and its left node least of all (spec §12.1).
        for (x, y) in [(80, 170), (200, 172), (400, 170), (100, 235), (20, 100)] {
            assert_eq!(name_tap(&w, name, x, y), Tap::None, "({x}, {y})");
        }
        // The strong ones: ALLOW NAME and ANSWER AGAIN; never REFUSE NAME or REFUSE AGAIN.
        for (class, owner, verdict, strong) in [
            (NameClass::Ads, None, Verdict::Allow, true),
            (NameClass::None, None, Verdict::Refuse, false),
            (
                NameClass::Ads,
                Some((OwnerVerdict::Allow, None)),
                Verdict::ClearAllow,
                false,
            ),
            (
                NameClass::None,
                Some((OwnerVerdict::Refuse, None)),
                Verdict::ClearRefuse,
                true,
            ),
        ] {
            w.name_facts = Some(facts(class, owner));
            let Tap::Act(intent, Some(Change::Back)) = right(&w) else {
                panic!("an act");
            };
            assert_eq!(
                intent,
                Intent::Name {
                    name: name.into(),
                    verdict
                }
            );
            assert_eq!(intent.is_strong(), strong, "{verdict:?}");
        }
        // A screen about another name, or about none, has no keys.
        assert_eq!(name_tap(&w, "other.example.org", 400, 280), Tap::None);
        assert_eq!(name_tap(&World::default(), name, 400, 280), Tap::None);
    }

    #[test]
    fn the_lines_say_what_became_of_the_name_and_which_list_holds_it() {
        use NameClass::{Ads, Deny, None as NoList};
        use OwnerVerdict::{Allow, Refuse};
        let today = Some(DayWord::Today);
        let mon = Some(DayWord::On(Weekday::Mon));
        let cases = [
            (
                facts(Ads, None),
                "Refused 1,204 times today \u{b7} on the ads list.",
            ),
            (
                facts(Deny, None),
                "Refused 1,204 times today \u{b7} on the deny list.",
            ),
            (
                NameFacts {
                    count: None,
                    ..facts(Ads, None)
                },
                "Refused today \u{b7} on the ads list.",
            ),
            (
                NameFacts {
                    count: Some(1),
                    ..facts(NoList, None)
                },
                "Answered 1 times today \u{b7} on no list.",
            ),
            (
                facts(Ads, Some((Allow, mon))),
                "Allowed by you since Mon \u{b7} on the ads list.",
            ),
            (
                facts(NoList, Some((Refuse, today))),
                "Refused by you today \u{b7} on no list.",
            ),
            (
                facts(Ads, Some((Allow, None))),
                "Allowed by you \u{b7} on the ads list.",
            ),
        ];
        for (facts, line) in cases {
            assert_eq!(line_of(&facts), line);
        }
        assert!(line_of(&facts(Ads, None)).chars().count() <= LINE_CELLS);
        // A name the Pi does not hold facts for yet is the bare lane, not a half-drawn mimic.
        assert_eq!(NameView::of(&World::default(), Section::Names, "x"), None);
    }

    #[test]
    fn a_long_name_is_fitted_to_the_pipe_and_to_the_title_but_the_intent_keeps_it_whole() {
        let long = "a.very.long.host.name.of.a.tracker.example.org";
        let w = World {
            top_names: vec![NameRow {
                name: long.into(),
                count: 5,
                open: false,
            }],
            owner_newest: vec![OwnerRow {
                name: long.into(),
                verdict: OwnerVerdict::Allow,
                day: None,
            }],
            name_facts: Some(NameFacts {
                name: long.into(),
                ..facts(NameClass::Ads, None)
            }),
            ..World::default()
        };
        assert_eq!(NamesView::of(&w).pipes[0].name, "a.tracker.example.org");
        assert_eq!(
            NameView::of(&w, Section::Names, long)
                .map(|v| v.title)
                .as_deref(),
            Some("of.a.tracker.example.org"),
            "host labels go first, whole, until it fits 25 cells"
        );
        assert_eq!(
            names_tap(&w, 100, 150),
            Tap::Nav(Change::Open(Screen::Name(long.into()))),
            "a screen is about a name by its text, not by what is shown of it"
        );
    }
}
