//! The canvas the OS screens are drawn on: a whole 480 x 320 RGBA frame, and the mockup's drawing
//! words on it: `rect`, `hollow`, `dot`, `tri` and text set glyph by glyph (`tx`, `txr`, `laneSay`
//! of `parts.js` and `d6.js`).
//!
//! A rectangle is whole pixels, as a browser fills one. A dot and a triangle have analytic
//! coverage: each pixel is sampled 4 x 4 times, so a shape's edge is the same wherever it stands and
//! a dot on a pixel corner is symmetric, and no table of masks is needed (the Drain keeps its
//! stamps, which are the browser's own for round 6). Text comes from the shared glyph cache.
//!
//! Nothing here panics, whatever the position, the size or the text: what falls outside the frame is
//! left out.

use crate::geometry::{Anchor, HEIGHT, Rect, Rgb, TextSlot, WIDTH, Weight};
use crate::text::{GlyphCache, mix_channel};
use crate::ui::look::{
    ADVANCE_EM, G4, GROUND, LANE_BASELINE, LANE_CELL, LANE_PX, RAIL_BOTTOM_Y, RAIL_H, RAIL_TOP_Y,
};

/// How many bytes a row of the RGBA frame holds.
const ROW: usize = WIDTH * 4;

/// How many samples a pixel's coverage takes in each direction.
const SAMPLES: u32 = 4;

/// How a run of text is set: its size, weight, colour, the tracking between letters, and where the
/// `x` it is given sits in the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    /// The font size in pixels.
    pub px: f32,
    /// Whether the weight is bold.
    pub bold: bool,
    /// The colour of the letters.
    pub color: Rgb,
    /// The extra space after each letter, in pixels.
    pub tracking: f32,
    /// Where `x` sits in the run.
    pub anchor: Anchor,
}

impl Face {
    /// Regular weight, no tracking, `x` at the run's left edge.
    pub const fn regular(px: f32, color: Rgb) -> Self {
        Self {
            px,
            bold: false,
            color,
            tracking: 0.0,
            anchor: Anchor::Start,
        }
    }

    /// Bold weight, no tracking, `x` at the run's left edge.
    pub const fn bold(px: f32, color: Rgb) -> Self {
        Self {
            bold: true,
            ..Self::regular(px, color)
        }
    }

    /// The same with `tracking` pixels after each letter.
    #[must_use]
    pub const fn track(self, tracking: f32) -> Self {
        Self { tracking, ..self }
    }

    /// The same with `x` at the run's centre.
    #[must_use]
    pub const fn mid(self) -> Self {
        Self {
            anchor: Anchor::Middle,
            ..self
        }
    }

    /// The same with `x` at the run's right edge.
    #[must_use]
    pub const fn end(self) -> Self {
        Self {
            anchor: Anchor::End,
            ..self
        }
    }

    /// How wide a run of `letters` letters is: each takes the font's advance plus the tracking, and
    /// the last one's tracking is not part of it (the mockup's `tw`).
    pub fn width(&self, letters: usize) -> f32 {
        if letters == 0 {
            return 0.0;
        }
        letters as f32 * (self.px * ADVANCE_EM + self.tracking) - self.tracking
    }
}

/// A 480 x 320 RGBA frame, rows from the top, every alpha 255, with the glyphs it draws from.
pub struct Canvas {
    /// The frame.
    rgba: Vec<u8>,
    /// The rendered letters.
    glyphs: GlyphCache,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    /// A black frame.
    pub fn new() -> Self {
        let mut canvas = Self {
            rgba: vec![0; WIDTH * HEIGHT * 4],
            glyphs: GlyphCache::new(),
        };
        canvas.clear(GROUND);
        canvas
    }

    /// The bytes of the frame, and of the glyphs the canvas has rendered (see `GlyphCache::bytes`).
    pub fn bytes(&self) -> (usize, usize) {
        (self.rgba.capacity(), self.glyphs.bytes())
    }

    /// The frame: 614,400 bytes of RGBA, rows from the top.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The colour of a pixel, or `None` outside the frame.
    pub fn pixel(&self, x: usize, y: usize) -> Option<Rgb> {
        if x >= WIDTH || y >= HEIGHT {
            return None;
        }
        let at = y * ROW + x * 4;
        let pixel = self.rgba.get(at..at + 3)?;
        Some([pixel[0], pixel[1], pixel[2]])
    }

    /// Fills the whole frame with one colour.
    pub fn clear(&mut self, color: Rgb) {
        for pixel in self.rgba.as_chunks_mut::<4>().0 {
            *pixel = [color[0], color[1], color[2], u8::MAX];
        }
    }

    /// Fills the rectangle `x0..x1` by `y0..y1` of the frame, clipped to it.
    fn fill(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), color: Rgb) {
        let clip = |v: i32, max: usize| usize::try_from(v.max(0)).map_or(0, |v| v.min(max));
        let (x0, x1) = (clip(x0, WIDTH), clip(x1, WIDTH));
        let (y0, y1) = (clip(y0, HEIGHT), clip(y1, HEIGHT));
        if x0 >= x1 {
            return;
        }
        for y in y0..y1 {
            let Some(span) = self.rgba.get_mut(y * ROW + x0 * 4..y * ROW + x1 * 4) else {
                continue;
            };
            for pixel in span.as_chunks_mut::<4>().0 {
                *pixel = [color[0], color[1], color[2], u8::MAX];
            }
        }
    }

    /// A rectangle of whole pixels, in one colour.
    pub fn rect(&mut self, r: Rect, color: Rgb) {
        let (x, y) = (i32::from(r.x), i32::from(r.y));
        self.fill((x, y, x + i32::from(r.w), y + i32::from(r.h)), color);
    }

    /// A hollow box as the mockup draws it: a block of `color`, then the ground `inside` it, `t` pixels in.
    pub fn hollow(&mut self, r: Rect, color: Rgb, t: u16, inside: Rgb) {
        self.rect(r, color);
        let t = i32::from(t);
        let (x, y) = (i32::from(r.x), i32::from(r.y));
        self.fill(
            (x + t, y + t, x + i32::from(r.w) - t, y + i32::from(r.h) - t),
            inside,
        );
    }

    /// Blends `color` into a pixel, `coverage` out of 255 of the way; the alpha is left as it is.
    fn blend(&mut self, x: i32, y: i32, color: Rgb, coverage: u8) {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if x >= WIDTH || y >= HEIGHT || coverage == 0 {
            return;
        }
        let at = y * ROW + x * 4;
        if let Some(pixel) = self.rgba.get_mut(at..at + 3) {
            for (under, channel) in pixel.iter_mut().zip(color) {
                *under = mix_channel(*under, channel, coverage);
            }
        }
    }

    /// Shades the pixels of the box `x0..x1` by `y0..y1` by how much of each `inside` holds: 4 x 4
    /// samples at the centres of a pixel's sixteenths, each counted if `inside` says it is in.
    fn shade(
        &mut self,
        (x0, y0, x1, y1): (i32, i32, i32, i32),
        color: Rgb,
        inside: impl Fn(f32, f32) -> bool,
    ) {
        let step = 1.0 / SAMPLES as f32;
        for y in y0.max(0)..y1.min(HEIGHT as i32) {
            for x in x0.max(0)..x1.min(WIDTH as i32) {
                let mut hit = 0_u32;
                for j in 0..SAMPLES {
                    for i in 0..SAMPLES {
                        let sx = x as f32 + (i as f32 + 0.5) * step;
                        let sy = y as f32 + (j as f32 + 0.5) * step;
                        hit += u32::from(inside(sx, sy));
                    }
                }
                let total = SAMPLES * SAMPLES;
                // The nearest of 255, so that all sixteen are 255.
                let coverage = (hit * 255 + total / 2) / total;
                self.blend(x, y, color, u8::try_from(coverage).unwrap_or(u8::MAX));
            }
        }
    }

    /// A disc of radius `r` centred on `(cx, cy)`, with analytic coverage. A centre on a whole
    /// number is on a pixel corner, as the mockup's circles are.
    pub fn dot(&mut self, cx: f32, cy: f32, r: f32, color: Rgb) {
        if !(r > 0.0 && r.is_finite() && cx.is_finite() && cy.is_finite()) {
            return;
        }
        // Saturating casts: a disc far outside the frame draws nothing.
        let bounds = (
            (cx - r).floor() as i32,
            (cy - r).floor() as i32,
            (cx + r).ceil() as i32,
            (cy + r).ceil() as i32,
        );
        let r2 = r * r;
        self.shade(bounds, color, |x, y| {
            let (dx, dy) = (x - cx, y - cy);
            dx * dx + dy * dy <= r2
        });
    }

    /// A triangle with analytic coverage.
    pub fn tri(&mut self, a: (f32, f32), b: (f32, f32), c: (f32, f32), color: Rgb) {
        let all = [a.0, a.1, b.0, b.1, c.0, c.1];
        if !all.iter().all(|v| v.is_finite()) {
            return;
        }
        let low = |f: fn(f32, f32) -> f32, i: usize| f(f(all[i], all[i + 2]), all[i + 4]);
        let bounds = (
            low(f32::min, 0).floor() as i32,
            low(f32::min, 1).floor() as i32,
            low(f32::max, 0).ceil() as i32,
            low(f32::max, 1).ceil() as i32,
        );
        // Which side of the edge from `p` to `q` a point is on.
        let side = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| {
            (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0)
        };
        self.shade(bounds, color, |x, y| {
            let (s1, s2, s3) = (side(a, b, x, y), side(b, c, x, y), side(c, a, x, y));
            (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0)
        });
    }

    /// Sets one line of text glyph by glyph, as the mockup's `tx` does: each glyph is centred in a
    /// slot of the font's advance plus the tracking, and `x` is the left edge of the first slot, the
    /// centre of the run (less the last letter's tracking) or its right edge, by `face.anchor`.
    /// `baseline` is the y the letters stand on.
    pub fn text(&mut self, s: &str, x: f32, baseline: f32, face: Face) {
        if s.is_empty() {
            return;
        }
        let advance = face.px * ADVANCE_EM + face.tracking;
        let width = face.width(s.chars().count());
        let left = match face.anchor {
            Anchor::Start => x,
            Anchor::Middle => x - width / 2.0,
            Anchor::End => x - width,
        };
        let weight = if face.bold {
            Weight::Bold
        } else {
            Weight::Regular
        };
        let mut utf8 = [0; 4];
        for (i, c) in s.chars().enumerate() {
            let slot = TextSlot {
                x: left + i as f32 * advance + face.px * ADVANCE_EM / 2.0,
                baseline,
                anchor: Anchor::Middle,
                size: face.px,
                weight,
                color: face.color,
            };
            self.glyphs
                .draw(&mut self.rgba, 0, &slot, c.encode_utf8(&mut utf8));
        }
    }

    /// The lane's rails, above and below its letters.
    pub fn rails(&mut self, color: Rgb) {
        let width = u16::try_from(WIDTH).unwrap_or(u16::MAX);
        self.rect(Rect::new(0, RAIL_TOP_Y, width, RAIL_H), color);
        self.rect(Rect::new(0, RAIL_BOTTOM_Y, width, RAIL_H), color);
    }

    /// The lane's rails in their own colour, `G4`.
    pub fn lane_rails(&mut self) {
        self.rails(G4);
    }

    /// The lane speaking in bare capitals, one letter a cell from `cell`, as `laneSay` does: 24 px bold
    /// centred in the cell, a space leaving its cell empty. `cell` is fractional for `lane_mid`.
    pub fn lane_say(&mut self, s: &str, cell: f32, color: Rgb) {
        let face = Face::bold(LANE_PX, color).mid();
        for (i, c) in s.chars().enumerate() {
            if c == ' ' {
                continue;
            }
            let x = (cell + i as f32) * LANE_CELL + LANE_CELL / 2.0;
            let mut utf8 = [0; 4];
            self.text(c.encode_utf8(&mut utf8), x, LANE_BASELINE, face);
        }
    }

    /// A lane word in the middle of the lane's 20 cells (the mockup's `laneMid`), for the states of the whole system.
    pub fn lane_mid(&mut self, s: &str, color: Rgb) {
        let cells = WIDTH as f32 / LANE_CELL;
        let letters = s.chars().count() as f32;
        self.lane_say(s, (cells - letters) / 2.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::look::{BLUE, BLUE_DEEP, WHITE};

    /// Every pixel of the frame that is not `ground`, as its coordinates.
    fn inked(canvas: &Canvas, ground: Rgb) -> Vec<(usize, usize)> {
        let mut out = vec![];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if canvas.pixel(x, y) != Some(ground) {
                    out.push((x, y));
                }
            }
        }
        out
    }

    /// The ink's box as `(min_x, min_y, max_x, max_y)`, ends included.
    fn bounds(canvas: &Canvas) -> (usize, usize, usize, usize) {
        let ink = inked(canvas, GROUND);
        assert!(!ink.is_empty(), "something was drawn");
        let xs = ink.iter().map(|p| p.0);
        let ys = ink.iter().map(|p| p.1);
        (
            xs.clone().min().unwrap_or(0),
            ys.clone().min().unwrap_or(0),
            xs.max().unwrap_or(0),
            ys.max().unwrap_or(0),
        )
    }

    /// The coverage of a white pixel on the ground, 0 to 255.
    fn white_at(canvas: &Canvas, x: usize, y: usize) -> u32 {
        u32::from(canvas.pixel(x, y).map_or(0, |p| p[0]))
    }

    #[test]
    fn a_new_canvas_is_black_and_opaque() {
        let canvas = Canvas::new();
        assert_eq!(canvas.rgba().len(), WIDTH * HEIGHT * 4);
        assert!(
            canvas
                .rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 0, 255])
        );
        assert_eq!(canvas.pixel(WIDTH, 0), None);
        assert_eq!(canvas.pixel(0, HEIGHT), None);
    }

    #[test]
    fn a_rect_is_whole_pixels_and_is_clipped_to_the_frame() {
        let mut canvas = Canvas::new();
        canvas.rect(Rect::new(10, 20, 3, 2), BLUE);
        let ink = inked(&canvas, GROUND);
        assert_eq!(ink.len(), 6);
        assert_eq!(canvas.pixel(10, 20), Some(BLUE));
        assert_eq!(canvas.pixel(12, 21), Some(BLUE));
        assert_eq!(
            canvas.pixel(13, 21),
            Some(GROUND),
            "the right end is excluded"
        );
        assert_eq!(
            canvas.pixel(12, 22),
            Some(GROUND),
            "the bottom end is excluded"
        );
        // Past the frame, and a rectangle with no size.
        canvas.rect(Rect::new(470, 310, 50, 50), WHITE);
        assert_eq!(canvas.pixel(479, 319), Some(WHITE));
        canvas.rect(Rect::new(0, 0, 0, 40), WHITE);
        canvas.rect(Rect::new(u16::MAX, u16::MAX, u16::MAX, u16::MAX), WHITE);
        assert_eq!(canvas.pixel(0, 0), Some(GROUND));
    }

    #[test]
    fn a_hollow_is_a_border_of_t_pixels_round_the_ground_given() {
        let mut canvas = Canvas::new();
        canvas.rect(Rect::new(0, 0, 100, 100), BLUE_DEEP);
        canvas.hollow(Rect::new(10, 10, 20, 12), WHITE, 3, GROUND);
        // The border.
        for (x, y) in [
            (10, 10),
            (29, 10),
            (10, 21),
            (29, 21),
            (12, 15),
            (27, 15),
            (20, 12),
            (20, 19),
        ] {
            assert_eq!(canvas.pixel(x, y), Some(WHITE), "({x}, {y})");
        }
        // The inside is the colour asked for, the whole of it.
        for (x, y) in [(13, 13), (26, 13), (13, 18), (26, 18), (20, 15)] {
            assert_eq!(canvas.pixel(x, y), Some(GROUND), "({x}, {y})");
        }
        let inside = (0..100)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.pixel(x, y) == Some(GROUND))
            .count();
        assert_eq!(inside, (20 - 6) * (12 - 6));
        // Outside the box the other colour is untouched.
        assert_eq!(canvas.pixel(9, 15), Some(BLUE_DEEP));
        assert_eq!(canvas.pixel(30, 15), Some(BLUE_DEEP));
        // The inside can be another colour than the ground, as on a mini wall tile.
        canvas.hollow(Rect::new(50, 50, 10, 10), WHITE, 2, BLUE_DEEP);
        assert_eq!(canvas.pixel(55, 55), Some(BLUE_DEEP));
        assert_eq!(canvas.pixel(50, 50), Some(WHITE));
        // A border wider than half the box leaves it all border: nothing inside to fill.
        canvas.hollow(Rect::new(70, 70, 4, 4), WHITE, 3, GROUND);
        assert_eq!(canvas.pixel(72, 72), Some(WHITE));
    }

    #[test]
    fn a_dot_on_a_pixel_corner_is_symmetric_and_has_its_area() {
        let mut canvas = Canvas::new();
        canvas.dot(100.0, 100.0, 5.0, WHITE);
        // Mirrors about the centre: pixel x <-> 199 - x, pixel y <-> 199 - y, and the diagonal.
        let mut area = 0.0;
        for y in 90..110 {
            for x in 90..110 {
                let c = white_at(&canvas, x, y);
                area += f64::from(c as u16) / 255.0;
                assert_eq!(c, white_at(&canvas, 199 - x, y), "left-right at ({x}, {y})");
                assert_eq!(c, white_at(&canvas, x, 199 - y), "up-down at ({x}, {y})");
                assert_eq!(c, white_at(&canvas, y, x), "diagonal at ({x}, {y})");
            }
        }
        let exact = std::f64::consts::PI * 25.0;
        assert!((area - exact).abs() < 0.8, "area {area}, a disc of {exact}");
        // The middle is solid and the corner of the box is bare.
        assert_eq!(canvas.pixel(100, 100), Some(WHITE));
        assert_eq!(canvas.pixel(95, 95), Some(GROUND));
    }

    #[test]
    fn a_dot_stays_within_its_radius_plus_one() {
        for (cx, cy, r) in [
            (60.0_f32, 60.0, 5.0),
            (60.5, 60.5, 2.5),
            (60.3, 60.7, 11.0),
            (60.0, 60.0, 0.7),
        ] {
            let mut canvas = Canvas::new();
            canvas.dot(cx, cy, r, WHITE);
            let ink = inked(&canvas, GROUND);
            assert!(!ink.is_empty(), "({cx}, {cy}) r {r} drew nothing");
            for (x, y) in ink {
                // The pixel's nearest point to the centre is within the radius: `+ 1` covers the pixel's size.
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                assert!(
                    (dx * dx + dy * dy).sqrt() <= r + 1.0,
                    "({x}, {y}) is far from ({cx}, {cy}) r {r}"
                );
            }
        }
    }

    #[test]
    fn a_dot_blends_over_what_is_under_it_and_survives_the_frame_edge() {
        let mut canvas = Canvas::new();
        canvas.rect(Rect::new(0, 0, 40, 40), BLUE);
        canvas.dot(20.0, 20.0, 6.0, WHITE);
        let edge = canvas.pixel(24, 23).unwrap_or_default();
        assert!(
            edge[0] > BLUE[0] && edge[0] < 255,
            "a blended edge, {edge:?}"
        );
        assert_eq!(canvas.pixel(20, 20), Some(WHITE));
        assert_eq!(
            canvas.pixel(20, 13),
            Some(BLUE),
            "outside the disc, the ground under it"
        );
        // Off the frame, not a number, or no size: nothing, and no panic.
        canvas.dot(-50.0, 10.0, 5.0, WHITE);
        canvas.dot(f32::NAN, 10.0, 5.0, WHITE);
        canvas.dot(10.0, 10.0, f32::INFINITY, WHITE);
        canvas.dot(10.0, 10.0, 0.0, WHITE);
        canvas.dot(10.0, 10.0, -3.0, WHITE);
        canvas.dot(0.0, 0.0, 6.0, WHITE);
        canvas.dot(480.0, 320.0, 6.0, WHITE);
        assert_eq!(canvas.pixel(479, 319), Some(WHITE));
    }

    #[test]
    fn a_triangle_is_filled_whichever_way_it_winds() {
        let mut a = Canvas::new();
        let mut b = Canvas::new();
        a.tri((10.0, 10.0), (10.0, 26.0), (30.0, 18.0), WHITE);
        b.tri((30.0, 18.0), (10.0, 26.0), (10.0, 10.0), WHITE);
        assert_eq!(a.rgba(), b.rgba());
        let area: f64 = (0..40)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(white_at(&a, x, y) as u16) / 255.0)
            .sum();
        assert!((area - 160.0).abs() < 1.5, "area {area}, a triangle of 160");
        assert_eq!(a.pixel(14, 18), Some(WHITE));
        assert_eq!(a.pixel(29, 10), Some(GROUND));
        // Not a number: nothing.
        a.tri((f32::NAN, 0.0), (1.0, 1.0), (2.0, 0.0), BLUE);
    }

    #[test]
    fn a_text_is_anchored_where_it_says() {
        let word = "HOME";
        let face = Face::bold(24.0, WHITE);
        let run = face.width(4);
        assert!((run - 57.6).abs() < 0.001, "four 14.4 px slots");
        let (mut start, mut mid, mut end) = (Canvas::new(), Canvas::new(), Canvas::new());
        start.text(word, 200.0, 100.0, face);
        mid.text(word, 200.0, 100.0, face.mid());
        end.text(word, 200.0, 100.0, face.end());
        let (s, m, e) = (bounds(&start), bounds(&mid), bounds(&end));
        // The ink is inside its slots, which are 3 px or so in from the ends for H and E at 24 px.
        assert!(
            s.0 >= 200 && s.0 <= 205 && (s.2 as f32) < 200.0 + run,
            "start {s:?}"
        );
        assert!(e.2 as f32 <= 200.0 && e.2 as f32 > 200.0 - 6.0, "end {e:?}");
        let centre = (m.0 + m.2) as f32 / 2.0;
        assert!((centre - 200.0).abs() <= 2.5, "middle {m:?}");
        // The three are the same run, moved.
        assert_eq!(m.2 - m.0, s.2 - s.0);
        assert_eq!(e.2 - e.0, s.2 - s.0);
        // The letters stand on the baseline: the ink ends at it (HOME has no descender).
        assert!(
            (99..=100).contains(&s.3),
            "the ink ends on the baseline, {s:?}"
        );
        assert!(
            s.1 >= 100 - 24 && s.1 <= 100 - 15,
            "capitals about 17 px high, {s:?}"
        );
    }

    #[test]
    fn tracking_opens_the_run_and_its_last_letter_adds_none() {
        let plain = Face::bold(16.0, WHITE);
        let spaced = plain.track(2.0);
        assert!((spaced.width(7) - (7.0 * 11.6 - 2.0)).abs() < 0.001);
        assert_eq!(spaced.width(0), 0.0);
        let (mut a, mut b) = (Canvas::new(), Canvas::new());
        a.text("REFUSED", 50.0, 80.0, plain);
        b.text("REFUSED", 50.0, 80.0, spaced);
        let (a, b) = (bounds(&a), bounds(&b));
        // Six gaps of 2 px open the run by 12 px (to within a pixel of ink).
        let opened = (b.2 - a.2) as i32;
        assert!((11..=13).contains(&opened), "opened by {opened}");
        // Centred, the run's middle does not move with the tracking.
        let (mut c, mut d) = (Canvas::new(), Canvas::new());
        c.text("REFUSED", 240.0, 80.0, plain.mid());
        d.text("REFUSED", 240.0, 80.0, spaced.mid());
        let (c, d) = (bounds(&c), bounds(&d));
        assert!(((c.0 + c.2) as i32 - (d.0 + d.2) as i32).abs() <= 2);
    }

    #[test]
    fn text_with_nothing_to_draw_or_nowhere_to_go_is_left_out() {
        let mut canvas = Canvas::new();
        canvas.text("", 10.0, 50.0, Face::bold(24.0, WHITE));
        canvas.text("   ", 10.0, 50.0, Face::bold(24.0, WHITE));
        canvas.text("A", f32::NAN, 50.0, Face::bold(24.0, WHITE));
        canvas.text("A", 10.0, f32::INFINITY, Face::bold(24.0, WHITE));
        canvas.text("A", 10.0, 50.0, Face::bold(0.0, WHITE));
        canvas.text("A", 10.0, 50.0, Face::bold(-8.0, WHITE));
        canvas.text("A", -400.0, 50.0, Face::bold(24.0, WHITE));
        canvas.text("A", 900.0, 50.0, Face::bold(24.0, WHITE));
        assert!(inked(&canvas, GROUND).is_empty());
    }

    #[test]
    fn the_lane_says_one_letter_a_cell_and_leaves_spaces_empty() {
        let mut canvas = Canvas::new();
        canvas.lane_say("A B", 0.0, WHITE);
        let ink = inked(&canvas, GROUND);
        let in_cell = |c: usize| ink.iter().filter(|p| p.0 / 24 == c).count();
        assert!(in_cell(0) > 20, "the A is in cell 0");
        assert_eq!(in_cell(1), 0, "the space leaves its cell empty");
        assert!(in_cell(2) > 20, "the B is in cell 2");
        // A letter is centred in its cell: its ink's middle is the cell's.
        let first: Vec<usize> = ink.iter().filter(|p| p.0 / 24 == 0).map(|p| p.0).collect();
        let middle = (first.iter().min().copied().unwrap_or(0)
            + first.iter().max().copied().unwrap_or(0)) as f32
            / 2.0;
        assert!((middle - 12.0).abs() <= 1.0, "centred at {middle}");
        // Letters stand on 45.
        assert!(matches!(ink.iter().map(|p| p.1).max(), Some(44 | 45)));
    }

    #[test]
    fn lane_mid_centres_a_word_on_the_lane() {
        let mut canvas = Canvas::new();
        canvas.lane_mid("QBERRYPI STARTING", WHITE);
        let (min_x, _, max_x, _) = bounds(&canvas);
        let centre = (min_x + max_x) as f32 / 2.0;
        assert!((centre - 240.0).abs() <= 6.0, "centred at {centre}");
        // Twenty letters fill the lane from edge to edge.
        let mut full = Canvas::new();
        full.lane_mid("ABCDEFGHIJKLMNOPQRST", WHITE);
        let (first, _, last, _) = bounds(&full);
        assert!(first < 8 && last > 470, "{first}..{last}");
    }

    #[test]
    fn the_rails_are_the_lane_rails() {
        let mut canvas = Canvas::new();
        canvas.lane_rails();
        for y in [4, 7, 64, 67] {
            assert_eq!(canvas.pixel(0, y), Some(G4), "row {y}");
            assert_eq!(canvas.pixel(479, y), Some(G4), "row {y}");
        }
        for y in [3, 8, 63, 68] {
            assert_eq!(canvas.pixel(100, y), Some(GROUND), "row {y}");
        }
    }
}
