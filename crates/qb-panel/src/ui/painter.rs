//! The painter of every screen but LIVE: a screen is built as a `View`, a small value of exactly
//! what it draws, and painted whole into memory only when the view is not the one on the frame.
//!
//! The Drain keeps its own `Painter`, whose dirty-row logic stays. A `ScreenPainter` is for the
//! screens that change a word at a time: it does no dirty logic of its own, since a whole screen is
//! a few milliseconds in memory and the screen's write is what costs. It converts the frame to
//! RGB565 and says which rows differ from the frame before, as `rows::changed_rows` measures: the
//! span the screen's driver would send.

use std::ops::Range;

use crate::geometry::{HEIGHT, WIDTH};
use crate::rgb565::convert_into;
use crate::rows::{BAND_GAP, changed_bands, changed_rows};
use crate::ui::canvas::Canvas;
use crate::ui::look::GROUND;

/// How many bytes a row of the RGB565 frame holds.
const ROW_BYTES: usize = WIDTH * 2;

/// Paints screens, each from a view, on a frame it keeps between calls.
///
/// `V` is the type of the views: one `PartialEq` value of exactly what the screens drawn through
/// this painter put on the glass, words already fitted, so that no two views that look alike differ.
/// It holds about 1.2 MB: the RGBA frame, the RGB565 frame and the one it is compared with.
pub struct ScreenPainter<V> {
    /// The frame as RGBA, with the glyphs.
    canvas: Canvas,
    /// The frame as RGB565, little-endian, rows from the top.
    rgb565: Vec<u8>,
    /// The frame being converted, then swapped with `rgb565`.
    next: Vec<u8>,
    /// The view the frame shows; `None` before the first `paint`.
    shown: Option<V>,
    /// Whether the glass may no longer show the frame, so the next `paint` writes it all.
    stale: bool,
}

impl<V: PartialEq> Default for ScreenPainter<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: PartialEq> ScreenPainter<V> {
    /// A painter with a black frame that has painted nothing yet.
    pub fn new() -> Self {
        Self {
            canvas: Canvas::new(),
            rgb565: vec![0; WIDTH * HEIGHT * 2],
            next: vec![0; WIDTH * HEIGHT * 2],
            shown: None,
            stale: true,
        }
    }

    /// Paints the screen of `view` with `draw`, unless `view` is the one the frame shows, and says which rows of
    /// the RGB565 frame changed: from the first to one past the last that differs from the frame before.
    ///
    /// It is `None` when nothing needs writing: the view is the one shown, or it is another view that
    /// looks the same. The first call, and the first after `invalidate`, paint whatever the view is and
    /// give every row: the caller has nothing, or something else, to compare with.
    pub fn paint(&mut self, view: V, draw: impl FnOnce(&mut Canvas, &V)) -> Option<Range<usize>> {
        let stale = self.render(view, draw)?;
        let rows = if stale {
            Some(0..HEIGHT)
        } else {
            changed_rows(&self.rgb565, &self.next, ROW_BYTES)
        };
        self.commit();
        rows
    }

    /// Paints as `paint` does, and says the rows that changed as bands (`rows::changed_bands`): a write of
    /// each is as many rows as changed in it, where the one span of `paint` runs over everything between two
    /// things that changed far apart (a lamp and the clock). Empty when nothing needs writing; one band of every row
    /// for the first paint and the first after `invalidate`.
    pub fn paint_bands(
        &mut self,
        view: V,
        draw: impl FnOnce(&mut Canvas, &V),
    ) -> Vec<Range<usize>> {
        let Some(stale) = self.render(view, draw) else {
            return vec![];
        };
        let bands = if stale {
            let all = 0..HEIGHT;
            vec![all]
        } else {
            changed_bands(&self.rgb565, &self.next, ROW_BYTES, BAND_GAP)
        };
        self.commit();
        bands
    }

    /// Paints `view` into the canvas and converts it into `next`, unless it is the view that shows; says whether the
    /// glass was stale. The frame the caller compares with is still `rgb565`, until `commit`.
    fn render(&mut self, view: V, draw: impl FnOnce(&mut Canvas, &V)) -> Option<bool> {
        if !self.stale && self.shown.as_ref() == Some(&view) {
            return None;
        }
        self.canvas.clear(GROUND);
        draw(&mut self.canvas, &view);
        convert_into(self.canvas.rgba(), &mut self.next);
        self.shown = Some(view);
        Some(self.stale)
    }

    /// The bytes of memory the painter holds, as `(frames, glyphs)`: the RGBA frame and the two RGB565 frames,
    /// and what the glyphs it has rendered take (spec §6.7: about 1.2 MB of frames, and the glyphs within 2 MB
    /// together with the Drain painter's).
    pub fn bytes(&self) -> (usize, usize) {
        let (rgba, glyphs) = self.canvas.bytes();
        (rgba + self.rgb565.capacity() + self.next.capacity(), glyphs)
    }

    /// Makes `next` the frame the glass shows.
    fn commit(&mut self) {
        std::mem::swap(&mut self.rgb565, &mut self.next);
        self.stale = false;
    }

    /// Says the glass no longer shows this painter's last frame, as after another screen was written:
    /// the next `paint` paints even an equal view, and gives every row. The caller can still diff
    /// `frame` with what it knows the glass shows, for a smaller write.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// The view the frame shows, if anything was painted.
    pub fn shown(&self) -> Option<&V> {
        self.shown.as_ref()
    }

    /// The frame as RGB565, 307,200 bytes, little-endian, rows from the top: the screen takes it as it is.
    pub fn frame(&self) -> &[u8] {
        &self.rgb565
    }

    /// The frame as RGBA, 614,400 bytes, every alpha 255: the same pixels as `frame`, for tests and the preview.
    pub fn rgba(&self) -> &[u8] {
        self.canvas.rgba()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;
    use crate::ui::look::{BLUE, WHITE};

    /// A view of one word and one bar's colour.
    #[derive(PartialEq, Clone)]
    struct Words(&'static str, bool);

    /// A list of one band of rows.
    fn one(rows: Range<usize>) -> Vec<Range<usize>> {
        vec![rows]
    }

    fn draw(canvas: &mut Canvas, view: &Words) {
        use crate::ui::canvas::Face;
        canvas.text(view.0, 10.0, 100.0, Face::bold(24.0, WHITE));
        if view.1 {
            canvas.rect(Rect::new(0, 200, 480, 4), BLUE);
        }
    }

    #[test]
    fn the_first_paint_gives_every_row_and_an_equal_view_gives_none() {
        let mut painter = ScreenPainter::new();
        assert_eq!(painter.paint(Words("HOME", false), draw), Some(0..HEIGHT));
        assert_eq!(painter.paint(Words("HOME", false), draw), None);
        assert_eq!(painter.paint(Words("HOME", false), draw), None);
        assert_eq!(painter.frame().len(), WIDTH * HEIGHT * 2);
        assert_eq!(painter.rgba().len(), WIDTH * HEIGHT * 4);
    }

    #[test]
    fn a_draw_function_is_not_run_for_an_equal_view() {
        let mut painter = ScreenPainter::new();
        painter.paint(Words("HOME", false), draw);
        let mut ran = false;
        painter.paint(Words("HOME", false), |_, _| ran = true);
        assert!(!ran, "an equal view paints nothing");
        painter.paint(Words("HOMES", false), |_, _| ran = true);
        assert!(ran, "another view paints");
    }

    #[test]
    fn a_one_word_change_gives_the_rows_of_that_word() {
        let mut painter = ScreenPainter::new();
        painter.paint(Words("HOME", false), draw);
        let rows = painter.paint(Words("HELP", false), draw);
        // The word stands on y 100 and its capitals are about 17 px high: rows 82 to 100, nowhere else.
        let rows = rows.expect("the word changed");
        assert!(
            rows.start >= 80 && rows.end <= 102 && rows.len() <= 22,
            "{rows:?}"
        );
        // A bar far below changes its own rows.
        let rows = painter
            .paint(Words("HELP", true), draw)
            .expect("the bar came");
        assert_eq!(rows, 200..204);
        // Both together are the span between them.
        let rows = painter
            .paint(Words("HOME", false), draw)
            .expect("both went");
        assert!(rows.start >= 80 && rows.end == 204, "{rows:?}");
    }

    #[test]
    fn bands_are_what_paint_gives_as_one_span_when_the_changes_are_far_apart() {
        let mut a = ScreenPainter::new();
        let mut b = ScreenPainter::new();
        assert_eq!(a.paint_bands(Words("HOME", false), draw), one(0..HEIGHT));
        assert_eq!(
            a.paint_bands(Words("HOME", false), draw),
            Vec::<Range<usize>>::new()
        );
        b.paint(Words("HOME", false), draw);
        // The word and the bar far below it change together: two writes, and the span between is `paint`'s.
        let bands = a.paint_bands(Words("HELP", true), draw);
        let span = b.paint(Words("HELP", true), draw).expect("both changed");
        assert_eq!(bands.len(), 2, "{bands:?}");
        assert_eq!(bands[1], 200..204);
        assert!(bands[0].start >= 80 && bands[0].end <= 102, "{bands:?}");
        assert_eq!((bands[0].start, bands[1].end), (span.start, span.end));
        assert!(bands.iter().map(Range::len).sum::<usize>() < span.len() / 2);
        // The frames are the same whichever way they were asked for.
        assert_eq!(a.frame(), b.frame());
        // After `invalidate` the whole glass is one band again.
        a.invalidate();
        assert_eq!(a.paint_bands(Words("HELP", true), draw), one(0..HEIGHT));
    }
    #[test]
    fn a_different_view_that_looks_the_same_writes_nothing() {
        let mut painter = ScreenPainter::new();
        painter.paint(Words("HOME", false), draw);
        // Spaces draw nothing, so these two are one picture.
        painter.paint(Words("  ", false), draw);
        assert_eq!(painter.paint(Words("   ", false), draw), None);
        assert_eq!(
            painter.shown().map(|w| w.0),
            Some("   "),
            "the view is still the one shown"
        );
    }

    #[test]
    fn invalidate_repaints_an_equal_view_whole() {
        let mut painter = ScreenPainter::new();
        painter.paint(Words("HOME", false), draw);
        painter.invalidate();
        assert_eq!(painter.paint(Words("HOME", false), draw), Some(0..HEIGHT));
        assert_eq!(painter.paint(Words("HOME", false), draw), None);
    }

    #[test]
    fn the_frame_is_the_rgba_converted() {
        let mut painter = ScreenPainter::new();
        painter.paint(Words("HOME", true), draw);
        assert_eq!(
            painter.frame(),
            crate::rgba_to_rgb565(painter.rgba()).as_slice()
        );
        // The painter's frame is what a fresh painter gives for the same view, whatever came before.
        let mut fresh = ScreenPainter::new();
        painter.paint(Words("OTHER", false), draw);
        painter.paint(Words("HOME", true), draw);
        fresh.paint(Words("HOME", true), draw);
        assert_eq!(painter.frame(), fresh.frame());
        assert_eq!(painter.rgba(), fresh.rgba());
    }
}
