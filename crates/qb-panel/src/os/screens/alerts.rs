//! ALERTS (spec §3.6). In phase 1 there is no list: only ALERT, the screen of the newest waiting alert
//! (ruling R2). It is the slab (`alertScreen`), white for a high alert and blue otherwise, with the title
//! and up to three lines, over the floor's one key, `DISMISS`. The right half of the floor stays black:
//! `SEE DEVICE` is P2's. The lines are made by qberrypi; here they are only drawn.

use crate::os::nav::Section;
use crate::os::screens::lane::{self, LaneView};
use crate::os::screens::parts::{self, SlabLine};
use crate::os::world::{AlertRow, World};
use crate::ui::canvas::Canvas;
use crate::ui::look::{BLUE, WHITE};

/// What ALERT puts on the glass. `Debug` shows the alert's colour and its place in the lines, not its words.
#[derive(Clone, PartialEq, Eq)]
pub struct AlertView {
    /// The lane, which says `ALERTS` and has no count tile.
    pub lane: LaneView,
    /// Whether the slab is white (high severity) rather than blue.
    pub high: bool,
    /// The title, in capitals.
    pub title: String,
    /// The lines under it: the subject first, in 20 px bold, then the others in 18 px.
    pub lines: Vec<String>,
}

impl std::fmt::Debug for AlertView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertView")
            .field("high", &self.high)
            .field("lines", &self.lines.len())
            .finish_non_exhaustive()
    }
}

impl AlertView {
    /// What ALERT shows for the alert with this id: `None` when the world does not hold it, because it
    /// is gone (the nav pops the screen at its next refresh).
    pub fn of(world: &World, id: u64) -> Option<Self> {
        let AlertRow {
            id: top,
            high,
            title,
            lines,
        } = world.top_alert.as_ref()?;
        (*top == id).then(|| Self {
            lane: LaneView::of(Section::Alerts, world),
            high: *high,
            title: title.clone(),
            lines: lines.clone(),
        })
    }
}

/// Draws ALERT.
pub fn draw(canvas: &mut Canvas, view: &AlertView) {
    lane::draw(canvas, &view.lane);
    let lines: Vec<SlabLine<'_>> = view
        .lines
        .iter()
        .enumerate()
        .map(|(k, text)| {
            if k == 0 {
                SlabLine::bold(text, 20.0)
            } else {
                SlabLine::plain(text, 18.0)
            }
        })
        .collect();
    parts::slab(
        canvas,
        Some(if view.high { WHITE } else { BLUE }),
        &view.title,
        &lines,
    );
    parts::keys(canvas, Some("DISMISS"), None, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::ui::look::GROUND;

    #[test]
    fn the_view_is_the_alert_the_world_holds_by_its_id() {
        let w = world(Still::Alert);
        let view = AlertView::of(&w, 4).expect("the top alert");
        assert!(view.high);
        assert_eq!(view.title, "PHISHING BLOCKED");
        assert_eq!(view.lines.len(), 3);
        assert_eq!(view.lane.words, "ALERTS");
        assert_eq!(view.lane.tile, None, "no count tile under ALERTS");
        assert_eq!(
            AlertView::of(&w, 3),
            None,
            "another alert is not the one held"
        );
        assert_eq!(AlertView::of(&World::default(), 4), None);
        let said = format!("{view:?}");
        assert!(
            !said.contains("PHISHING") && !said.contains("secure-login"),
            "{said}"
        );
    }

    #[test]
    fn a_high_alert_is_a_white_slab_and_a_low_one_blue_with_black_words() {
        let mut high = Canvas::new();
        draw(
            &mut high,
            &AlertView::of(&world(Still::Alert), 4).expect("an alert"),
        );
        assert_eq!(high.pixel(8, 80), Some(WHITE));
        assert_eq!(high.pixel(471, 231), Some(WHITE));
        let mut w = world(Still::Alert);
        if let Some(top) = w.top_alert.as_mut() {
            top.high = false;
        }
        let mut low = Canvas::new();
        draw(&mut low, &AlertView::of(&w, 4).expect("an alert"));
        assert_eq!(low.pixel(8, 80), Some(BLUE));
        // Black words on it: some of the slab's top row of ink is black.
        assert!((20..300).any(|x| (90..130).any(|y| low.pixel(x, y) == Some(GROUND))));
    }

    #[test]
    fn dismiss_is_the_only_key_and_the_right_half_of_the_floor_is_black() {
        let mut canvas = Canvas::new();
        draw(
            &mut canvas,
            &AlertView::of(&world(Still::Alert), 4).expect("an alert"),
        );
        assert_eq!(canvas.pixel(8, 240), Some(WHITE));
        assert!((244..472).all(|x| (236..320).all(|y| canvas.pixel(x, y) == Some(GROUND))));
    }

    #[test]
    fn fewer_lines_leave_the_slabs_top_empty() {
        let mut w = world(Still::Alert);
        if let Some(top) = w.top_alert.as_mut() {
            top.lines.truncate(1);
        }
        let mut canvas = Canvas::new();
        draw(&mut canvas, &AlertView::of(&w, 4).expect("an alert"));
        // The one line stands on 217; the rows above it, under the title, are the slab's colour.
        assert!((140..196).all(|y| (22..300).all(|x| canvas.pixel(x, y) == Some(WHITE))));
    }
}
