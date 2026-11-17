//! The parts the screens below the lane are made of (`screens.js`, `parts.js`): the slab that says what a
//! thing is, the floor of keys under it, the four keys of the gate's choices, a reading with its
//! figure, and the padlock a strong key wears while the lock is on.
//!
//! Every function draws on a `Canvas`; the numbers are the mockup's, and the README's "Numbers for
//! building the wall".

use crate::geometry::{Rect, Rgb};
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    BLUE, BLUE_NIGHT, FLOOR_SPLIT_X, FLOOR_Y, FOUR_KEYS_PITCH, FOUR_KEYS_X0, G1, G2, GROUND, KEY_H,
    KEY_W2, KEY_W4, KEY_X2, KEY_Y, SLAB_H, SLAB_W, SLAB_X, SLAB_Y, WHITE,
};

/// The gate's three open keys, as `GATE_OPTS` has them: the word and the seconds it opens for.
pub const GATE_CHOICES: [(&str, u16); 3] = [("5 MIN", 300), ("30 MIN", 1_800), ("1 HOUR", 3_600)];

/// A title longer than this many letters is set smaller: 30 px instead of 40.
const TITLE_BIG_LETTERS: usize = 18;

/// One line of a slab's words: the text, its size, and whether it is bold.
#[derive(Debug, Clone, Copy)]
pub struct SlabLine<'a> {
    /// The words.
    pub text: &'a str,
    /// The size: 24, 20 or 18.
    pub px: f32,
    /// Bold, as the first line is.
    pub bold: bool,
}

impl<'a> SlabLine<'a> {
    /// A line of bold words, as the first line of a slab is.
    pub const fn bold(text: &'a str, px: f32) -> Self {
        Self {
            text,
            px,
            bold: true,
        }
    }

    /// A line of regular words.
    pub const fn plain(text: &'a str, px: f32) -> Self {
        Self {
            text,
            px,
            bold: false,
        }
    }
}

/// The slab: 464 by 152 at (8, 80). `ground` is its colour, solid, or `None` for the hollow slab, a
/// white frame round black. The title is 40 px bold (30 px past 18 letters); the lines stand on the
/// slab's foot, the last at y 217, 24 px apart upward.
pub fn slab(canvas: &mut Canvas, ground: Option<Rgb>, title: &str, lines: &[SlabLine<'_>]) {
    let area = Rect::new(SLAB_X, SLAB_Y, SLAB_W, SLAB_H);
    match ground {
        Some(color) => canvas.rect(area, color),
        None => canvas.hollow(area, WHITE, 4, GROUND),
    }
    let big = title.chars().count() <= TITLE_BIG_LETTERS;
    let (px, baseline) = if big { (40.0, 46.0) } else { (30.0, 42.0) };
    let title_ink = ground.map_or(WHITE, |_| GROUND);
    canvas.text(
        title,
        22.0 - px * 0.05,
        f32::from(SLAB_Y) + baseline,
        Face::bold(px, title_ink),
    );
    let mut baseline = f32::from(SLAB_Y + SLAB_H) - 15.0;
    for line in lines.iter().rev() {
        let ink = match (ground, line.bold) {
            (Some(_), _) => GROUND,
            (None, true) => WHITE,
            (None, false) => G1,
        };
        let face = if line.bold {
            Face::bold(line.px, ink)
        } else {
            Face::regular(line.px, ink)
        };
        canvas.text(line.text, 21.0, baseline, face);
        baseline -= 24.0;
    }
}

/// The floor's keys, two of them (`keys`): `left` is the key that keeps things as they are, a white frame
/// round black; `right` is the key that changes them, solid white. A strong key wears the padlock while
/// the lock is on, which `locked` says. A missing key leaves its half of the floor black.
pub fn keys(canvas: &mut Canvas, left: Option<&str>, right: Option<&str>, locked: bool) {
    if let Some(word) = left {
        canvas.hollow(Rect::new(8, KEY_Y, KEY_W2, KEY_H), WHITE, 4, GROUND);
        canvas.text(word, 122.0, 281.0, Face::bold(24.0, WHITE).mid());
    }
    if let Some(word) = right {
        canvas.rect(Rect::new(KEY_X2, KEY_Y, KEY_W2, KEY_H), WHITE);
        let shift = if locked { 12.0 } else { 0.0 };
        canvas.text(word, 358.0 + shift, 281.0, Face::bold(24.0, GROUND).mid());
        if locked {
            let width = Face::bold(24.0, GROUND).width(word.chars().count());
            // The mockup's x is fractional; a rectangle here is whole pixels.
            let x = (358.0 - width / 2.0 - 14.0).round().max(0.0) as u16;
            padlock(canvas, x, 262, GROUND, WHITE);
        }
    }
}

/// A padlock from two rectangles (`lockMark`): the shackle, a hollow 10 by 10, over the body, 16 by 11.
/// A strong key wears it while the lock is on. `ground` is what the shackle's hole shows.
pub fn padlock(canvas: &mut Canvas, x: u16, y: u16, color: Rgb, ground: Rgb) {
    canvas.hollow(Rect::new(x + 3, y, 10, 10), color, 2, ground);
    canvas.rect(Rect::new(x, y + 7, 16, 11), color);
}

/// The gate's four keys on the floor (`fourKeys`): the shut key that stays, solid blue with `keep` over
/// `SHUT`, and the three open keys.
pub fn four_keys(canvas: &mut Canvas, keep: &str) {
    shut_key(canvas, key_rect(0), keep);
    for (i, (value, _)) in GATE_CHOICES.iter().enumerate() {
        open_key(canvas, key_rect(i + 1), value);
    }
}

/// The place of the floor's `i`th key of four.
pub fn key_rect(i: usize) -> Rect {
    let step = u16::try_from(i).unwrap_or(0) * FOUR_KEYS_PITCH;
    Rect::new(FOUR_KEYS_X0 + step, KEY_Y, KEY_W4, KEY_H)
}

/// Which of four keys a press at column `x` on the floor is on: 0 to 3, by `floor((x - 4) / 118)`, the
/// ends going on to the screen's edges.
pub fn four_keys_index(x: u16) -> usize {
    usize::from(x.saturating_sub(4) / FOUR_KEYS_PITCH).min(3)
}

/// Whether a press at row `y` is on the floor.
pub const fn on_floor(y: u16) -> bool {
    y >= FLOOR_Y
}

/// Whether a press at column `x` is on the floor's left key (of two).
pub const fn on_left(x: u16) -> bool {
    x < FLOOR_SPLIT_X
}

/// A shut key (`shutKey`): solid blue, `label` small over `SHUT`.
pub fn shut_key(canvas: &mut Canvas, r: Rect, label: &str) {
    canvas.rect(r, BLUE);
    let (cx, cy) = centre(r);
    canvas.text(
        label,
        cx,
        cy - 10.0,
        Face::bold(16.0, BLUE_NIGHT).track(2.0).mid(),
    );
    canvas.text("SHUT", cx, cy + 22.0, Face::bold(24.0, WHITE).mid());
}

/// An open key (`openKey`): a white frame round black, `OPEN` small over `value`.
pub fn open_key(canvas: &mut Canvas, r: Rect, value: &str) {
    canvas.hollow(r, WHITE, 4, GROUND);
    let (cx, cy) = centre(r);
    canvas.text("OPEN", cx, cy - 10.0, Face::bold(16.0, G1).track(2.0).mid());
    canvas.text(value, cx, cy + 22.0, Face::bold(24.0, WHITE).mid());
}

/// The middle of a rectangle.
fn centre(r: Rect) -> (f32, f32) {
    (
        f32::from(r.x) + f32::from(r.w) / 2.0,
        f32::from(r.y) + f32::from(r.h) / 2.0,
    )
}

/// A reading with a screen of its own (`reading`): a small label, one large figure, one or two lines, and
/// the floor. The label is 16 px at baseline 104, the figure 64 px at 170, the lines 18 px at 202 and 225.
pub fn reading(
    canvas: &mut Canvas,
    label: &str,
    figure: &str,
    lines: &[&str],
    left: Option<&str>,
    right: Option<&str>,
    locked: bool,
) {
    canvas.text(label, 6.4, 104.0, Face::bold(16.0, G2).track(2.5));
    canvas.text(figure, 3.0, 170.0, Face::bold(64.0, WHITE));
    for (k, line) in lines.iter().enumerate() {
        canvas.text(line, 7.0, 202.0 + 23.0 * k as f32, Face::regular(18.0, G1));
    }
    keys(canvas, left, right, locked);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(canvas: &Canvas, x: usize, y: usize, color: Rgb) -> bool {
        canvas.pixel(x, y) == Some(color)
    }

    #[test]
    fn a_solid_slab_is_464_by_152_at_8_80_and_a_hollow_one_is_a_white_frame() {
        let mut solid = Canvas::new();
        slab(&mut solid, Some(BLUE), "NEW DEVICE", &[]);
        for (x, y) in [(8, 80), (471, 80), (8, 231), (471, 231), (300, 200)] {
            assert!(is(&solid, x, y, BLUE), "({x}, {y})");
        }
        for (x, y) in [(7, 80), (472, 80), (8, 79), (8, 232)] {
            assert!(is(&solid, x, y, GROUND), "({x}, {y})");
        }
        let mut hollow = Canvas::new();
        slab(&mut hollow, None, "OPEN THE GATE", &[]);
        for (x, y) in [(8, 80), (11, 231), (471, 150), (300, 83)] {
            assert!(is(&hollow, x, y, WHITE), "frame ({x}, {y})");
        }
        // Inside the frame, away from the title, it is black.
        assert!(is(&hollow, 300, 150, GROUND));
        assert!(is(&hollow, 12, 200, GROUND));
    }

    #[test]
    fn a_title_of_eighteen_letters_is_big_and_of_nineteen_is_smaller() {
        // Ink rows: 40 px letters are taller than 30 px letters.
        let height = |title: &str| {
            let mut canvas = Canvas::new();
            slab(&mut canvas, Some(WHITE), title, &[]);
            let ink: Vec<usize> = (82..140)
                .filter(|&y| (10..470).any(|x| canvas.pixel(x, y) != Some(WHITE)))
                .collect();
            ink.last().copied().unwrap_or(0) - ink.first().copied().unwrap_or(0)
        };
        let big = height(&"H".repeat(18));
        let small = height(&"H".repeat(19));
        assert!(big > small + 5, "{big} against {small}");
    }

    #[test]
    fn the_lines_stand_on_the_foot_and_a_missing_line_leaves_the_top_ones_empty() {
        let ink_in = |canvas: &Canvas, rows: std::ops::Range<usize>| {
            rows.clone()
                .any(|y| (14..466).any(|x| canvas.pixel(x, y) != Some(GROUND)))
        };
        let mut canvas = Canvas::new();
        slab(
            &mut canvas,
            None,
            "T",
            &[
                SlabLine::bold("Hxxxxx", 24.0),
                SlabLine::plain("Hxxxxx", 18.0),
            ],
        );
        // The last line's baseline is y 217; the first's 193 (a capital is about 0.73 em tall).
        assert!(ink_in(&canvas, 200..218), "the last line");
        assert!(ink_in(&canvas, 174..194), "the first line");
        assert!(
            !ink_in(&canvas, 135..170),
            "nothing between the title and the lines"
        );
    }

    #[test]
    fn the_floor_has_two_keys_a_hollow_one_on_the_left_and_a_solid_one_on_the_right() {
        let mut canvas = Canvas::new();
        keys(&mut canvas, Some("BACK"), Some("RESTART"), false);
        // Left: a white frame 228 by 64 at (8, 240), black inside. Right: solid white at (244, 240).
        for (x, y) in [(8, 240), (235, 303), (8, 303), (235, 240)] {
            assert!(is(&canvas, x, y, WHITE), "left frame ({x}, {y})");
        }
        assert!(is(&canvas, 20, 250, GROUND));
        for (x, y) in [(244, 240), (471, 303), (250, 245)] {
            assert!(is(&canvas, x, y, WHITE), "right ({x}, {y})");
        }
        assert!(is(&canvas, 7, 270, GROUND) && is(&canvas, 240, 270, GROUND));
        // The right key's letters are black on it.
        assert!((300..420).any(|x| (262..285).any(|y| is(&canvas, x, y, GROUND))));
        // One key leaves the other half black.
        let mut only_left = Canvas::new();
        keys(&mut only_left, Some("DISMISS"), None, false);
        assert!(is(&only_left, 8, 240, WHITE));
        assert!((244..472).all(|x| (240..304).all(|y| is(&only_left, x, y, GROUND))));
    }

    #[test]
    fn a_locked_strong_key_wears_a_padlock_left_of_its_word() {
        let mut plain = Canvas::new();
        keys(&mut plain, Some("BACK"), Some("RESTART"), false);
        let mut locked = Canvas::new();
        keys(&mut locked, Some("BACK"), Some("RESTART"), true);
        // `RESTART`, 7 letters of 14.4 px, centred at 358 + 12: its left edge is at 358 - 50.4 + 12 = 319.6; the
        // lock stands at x 358 - 50.4 - 14 = 293.6, 16 wide, from y 262 to 280.
        let lock_ink = |canvas: &Canvas| {
            (294..307)
                .flat_map(|x| (262..281).map(move |y| (x, y)))
                .filter(|&(x, y)| is(canvas, x, y, GROUND))
                .count()
        };
        assert_eq!(lock_ink(&plain), 0, "no padlock, no ink there");
        let ink = lock_ink(&locked);
        // The body is 16 by 11 and the shackle a 10 by 10 frame of 2; 13 of the 16 columns are in this box.
        assert!(ink > 120, "{ink}");
        // The left key never wears one, whatever `locked` says.
        let mut left_only = Canvas::new();
        keys(&mut left_only, Some("BACK"), None, true);
        let mut left_plain = Canvas::new();
        keys(&mut left_plain, Some("BACK"), None, false);
        assert_eq!(left_only.rgba(), left_plain.rgba());
    }

    #[test]
    fn the_padlock_is_a_hollow_shackle_over_a_body() {
        let mut canvas = Canvas::new();
        canvas.clear(WHITE);
        padlock(&mut canvas, 100, 50, GROUND, WHITE);
        // Body: 16 by 11 at (100, 57).
        for (x, y) in [(100, 57), (115, 57), (100, 67), (115, 67)] {
            assert!(is(&canvas, x, y, GROUND), "body ({x}, {y})");
        }
        // Shackle: a 10 by 10 frame of 2 at (103, 50); the hole is the ground.
        for (x, y) in [(103, 50), (112, 50), (103, 56)] {
            assert!(is(&canvas, x, y, GROUND), "shackle ({x}, {y})");
        }
        for (x, y) in [(106, 53), (109, 55)] {
            assert!(is(&canvas, x, y, WHITE), "hole ({x}, {y})");
        }
    }

    #[test]
    fn the_four_keys_are_a_shut_one_and_three_open_ones_118_apart() {
        let mut canvas = Canvas::new();
        four_keys(&mut canvas, "KEEP");
        // The shut key: solid blue at (8, 240), 110 by 64.
        for (x, y) in [(8, 240), (117, 303), (20, 250)] {
            assert!(is(&canvas, x, y, BLUE), "({x}, {y})");
        }
        // The open keys: white frames at 126, 244 and 362.
        for left in [126, 244, 362] {
            for (dx, dy) in [(0, 0), (109, 0), (0, 63), (109, 63)] {
                assert!(is(&canvas, left + dx, 240 + dy, WHITE), "{left}");
            }
            assert!(is(&canvas, left + 55, 245, GROUND));
        }
        for i in 0..4 {
            assert_eq!(usize::from(key_rect(i).x), 8 + 118 * i);
        }
    }

    #[test]
    fn the_keys_taps_are_where_the_floor_says() {
        assert!(!on_floor(235) && on_floor(236));
        assert!(on_left(239) && !on_left(240));
        let index = |x| four_keys_index(x);
        assert_eq!(
            [0, 3, 4, 121, 122, 239, 240, 357, 358, 479].map(index),
            [0, 0, 0, 0, 1, 1, 2, 2, 3, 3]
        );
    }

    #[test]
    fn a_reading_is_a_label_a_figure_two_lines_and_the_floor() {
        let mut canvas = Canvas::new();
        reading(
            &mut canvas,
            "ABOUT",
            "QBERRYPI",
            &["one", "two"],
            Some("BACK"),
            None,
            false,
        );
        let ink = |rows: std::ops::Range<usize>| {
            rows.clone()
                .any(|y| (0..480).any(|x| canvas.pixel(x, y) != Some(GROUND)))
        };
        assert!(ink(94..106), "the label stands on 104");
        assert!(ink(125..172), "the figure stands on 170");
        assert!(ink(190..204) && ink(213..227), "the two lines");
        assert!(ink(240..304), "the floor");
        assert!(!ink(72..92), "nothing above the label");
    }
}
