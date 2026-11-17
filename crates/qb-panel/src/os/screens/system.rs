//! SYSTEM (spec §3.8). In phase 1 it is the ABOUT reading only, as the section's root: the lane says
//! `SYSTEM`, the screen is the `ABOUT` label, `QBERRYPI` in 64 px, two lines (`a DNS gate and watchdog for
//! the house`, `version 0.4.1 on a Pi 3B+ · panel 480×320`) and the floor's one key, `BACK`, which goes
//! HOME. The wall of six readings (VERSION, NIGHT, WI-FI, UP, LOCK, ABOUT) is P3's.

use crate::format::{self, DASH};
use crate::os::nav::Section;
use crate::os::screens::lane::{self, LaneView};
use crate::os::screens::parts;
use crate::os::world::World;
use crate::ui::canvas::Canvas;

/// The version's cells in ABOUT's second line: the line has 28 other letters, and 480 px hold 43 of 10.8 px.
const ABOUT_VERSION_CELLS: usize = 7;

/// What ABOUT puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutView {
    /// The lane, which says `SYSTEM`.
    pub lane: LaneView,
    /// The reading's second line, with the running version in it.
    pub line: String,
}

impl AboutView {
    /// What ABOUT shows for a world.
    pub fn of(world: &World) -> Self {
        let version = if world.version.trim().is_empty() {
            DASH.to_string()
        } else {
            format::fit_text(&world.version, ABOUT_VERSION_CELLS)
        };
        Self {
            lane: LaneView::of(Section::System, world),
            line: format!("version {version} on a Pi 3B+ \u{b7} panel 480\u{d7}320"),
        }
    }
}

/// Draws ABOUT.
pub fn draw(canvas: &mut Canvas, view: &AboutView) {
    lane::draw(canvas, &view.lane);
    parts::reading(
        canvas,
        "ABOUT",
        "QBERRYPI",
        &["a DNS gate and watchdog for the house", &view.line],
        Some("BACK"),
        None,
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::ui::look::{G1, WHITE};

    #[test]
    fn the_second_line_has_the_version_and_never_outgrows_the_screen() {
        let about = AboutView::of(&world(Still::About));
        assert_eq!(
            about.line,
            "version 0.4.1 on a Pi 3B+ \u{b7} panel 480\u{d7}320"
        );
        assert_eq!(about.lane.words, "SYSTEM");
        let mut long = world(Still::About);
        long.version = "0.4.1-release-candidate".to_string();
        let line = AboutView::of(&long).line;
        assert!(line.contains("0.4.1-~"), "{line}");
        // 18 px letters are 10.8 px wide, from x 7: the line ends inside the 480.
        assert!(7.0 + 10.8 * line.chars().count() as f32 <= 480.0, "{line}");
        assert_eq!(
            AboutView::of(&World::default()).line,
            format!("version {DASH} on a Pi 3B+ \u{b7} panel 480\u{d7}320")
        );
    }

    #[test]
    fn about_has_its_label_its_figure_its_two_lines_and_back() {
        let mut canvas = Canvas::new();
        draw(&mut canvas, &AboutView::of(&world(Still::About)));
        let brightest = |x0, x1, y0, y1| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter_map(|(x, y)| canvas.pixel(x, y))
                .map(|p| p[0])
                .max()
                .unwrap_or(0)
        };
        assert_eq!(brightest(0, 480, 120, 172), 255, "the figure, white");
        assert_eq!(brightest(0, 480, 190, 206), G1[0], "the first line, G1");
        assert_eq!(canvas.pixel(8, 240), Some(WHITE), "BACK's frame");
        assert_eq!(brightest(244, 472, 240, 304), 0, "no right key");
    }
}
