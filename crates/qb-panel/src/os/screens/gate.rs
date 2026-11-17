//! GATE (spec §3.5, `gateScreen`): from HOME's gate key, the choices for opening the main gate. A hollow
//! slab, `OPEN THE GATE`, `Ads pass on every device.` and `Threats stay blocked. It shuts itself.`, over
//! four keys: `KEEP SHUT`, which goes back, and `5 MIN`, `30 MIN` and `1 HOUR`, which open it for that long.
//! The main gate is not a strong action (spec §7.1).

use crate::os::nav::Section;
use crate::os::screens::lane::{self, LaneView};
use crate::os::screens::parts::{self, GATE_CHOICES, SlabLine};
use crate::os::world::World;
use crate::ui::canvas::Canvas;

/// What GATE puts on the glass: the lane. The slab and the keys never change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateView {
    /// The lane, which says `GATE`.
    pub lane: LaneView,
}

impl GateView {
    /// What GATE shows for a world.
    pub fn of(world: &World) -> Self {
        Self {
            lane: LaneView::of(Section::Gate, world),
        }
    }
}

/// How long the key at column `x` of the floor opens the gate for, in seconds; `None` for `KEEP SHUT`
/// (`floor((x - 4) / 118)`: 0 keeps, 1 to 3 open for 5, 30 and 60 minutes, the right end going on to the edge).
pub fn key_secs(x: u16) -> Option<u16> {
    match parts::four_keys_index(x) {
        0 => None,
        key => GATE_CHOICES.get(key - 1).map(|&(_, secs)| secs),
    }
}

/// Draws GATE.
pub fn draw(canvas: &mut Canvas, view: &GateView) {
    lane::draw(canvas, &view.lane);
    parts::slab(
        canvas,
        None,
        "OPEN THE GATE",
        &[
            SlabLine::bold("Ads pass on every device.", 24.0),
            SlabLine::plain("Threats stay blocked. It shuts itself.", 18.0),
        ],
    );
    parts::four_keys(canvas, "KEEP");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::ui::look::{BLUE, G4, GROUND, WHITE};

    #[test]
    fn the_four_keys_open_for_five_thirty_and_sixty_minutes() {
        let secs = |x| key_secs(x);
        assert_eq!(secs(8), None);
        assert_eq!(secs(121), None);
        assert_eq!(secs(122), Some(300));
        assert_eq!(secs(240), Some(1_800));
        assert_eq!(secs(358), Some(3_600));
        assert_eq!(secs(479), Some(3_600));
        assert_eq!(
            GATE_CHOICES.map(|(word, _)| word),
            ["5 MIN", "30 MIN", "1 HOUR"]
        );
    }

    #[test]
    fn the_view_is_the_gates_lane_and_nothing_of_the_world_changes_the_rest() {
        let view = GateView::of(&world(Still::Gate));
        assert_eq!(view.lane.words, "GATE");
        // The same lane in the world of an alert changes only the lane's tile.
        let alerts = GateView::of(&world(Still::HomeAlert));
        assert_eq!(alerts.lane.tile, Some('4'));
        assert_eq!(view.lane.tile, None);
    }

    #[test]
    fn gate_is_a_hollow_slab_over_four_keys() {
        let mut canvas = Canvas::new();
        draw(&mut canvas, &GateView::of(&world(Still::Gate)));
        // The lit rail is under the word `GATE`: x 120 for four letters.
        assert_eq!(canvas.pixel(120, 64), Some(WHITE));
        assert_eq!(canvas.pixel(215, 64), Some(WHITE));
        assert_eq!(canvas.pixel(216, 64), Some(G4));
        // The slab's frame, the keep key and the open keys.
        assert_eq!(canvas.pixel(8, 80), Some(WHITE));
        assert_eq!(canvas.pixel(300, 150), Some(GROUND));
        assert_eq!(canvas.pixel(8, 240), Some(BLUE));
        assert_eq!(canvas.pixel(126, 240), Some(WHITE));
        assert_eq!(canvas.pixel(130, 244), Some(GROUND));
    }
}
