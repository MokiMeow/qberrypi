//! The screens, one file for each family. Each is a drawing of a view built from the `World`.
//!
//! `View` is the one value every screen drawn through the `ScreenPainter` is a variant of: `view_of(nav,
//! world)` makes the view of what the panel shows, and `draw` paints it. LIVE is the Drain's own and
//! has no view here; `live` holds the OS's glue to it (its taps).

pub mod alerts;
pub mod devices;
pub mod gate;
pub mod home;
pub mod lane;
pub mod live;
pub mod parts;
pub mod system;

use crate::os::nav::{Nav, Screen, Section};
use crate::os::world::World;
use crate::ui::canvas::Canvas;

/// What a screen puts on the glass, whichever screen it is.
///
/// `Debug` shows no name: the views that hold words keep them out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// A section with no screen drawn yet: the lane alone, which names it.
    Bare(lane::LaneView),
    /// HOME.
    Home(Box<home::HomeView>),
    /// GATE.
    Gate(gate::GateView),
    /// ABOUT, SYSTEM's root in P1.
    About(system::AboutView),
    /// ALERT.
    Alert(alerts::AlertView),
    /// DEVICES, a page of the wall.
    Wall(Box<devices::WallView>),
    /// DEVICE.
    Device(Box<devices::DeviceView>),
    /// CONFIRM, over a device.
    Confirm(Box<devices::ConfirmView>),
    /// ITS GATE, over a device.
    ItsGate(Box<devices::ItsGateView>),
}

/// The view of what the panel shows, or `None` for LIVE, which the Drain draws with its own painter.
///
/// A section whose screens come with a later task is the bare lane. A screen whose thing is not in the
/// world (the nav pops it at its next `refresh`) is the bare lane too, for the moment between.
pub fn view_of(nav: &Nav, world: &World) -> Option<View> {
    let root = nav.root();
    let bare = || View::Bare(lane::LaneView::of(root, world));
    Some(match nav.top() {
        Some(&Screen::Alert(id)) => alerts::AlertView::of(world, id).map_or_else(bare, View::Alert),
        Some(&Screen::Device(mac)) => devices::DeviceView::of(world, &mac)
            .map_or_else(bare, |view| View::Device(Box::new(view))),
        Some(&Screen::Confirm { mac, pass }) => devices::ConfirmView::of(world, &mac, pass)
            .map_or_else(bare, |view| View::Confirm(Box::new(view))),
        Some(&Screen::ItsGate(mac)) => devices::ItsGateView::of(world, &mac)
            .map_or_else(bare, |view| View::ItsGate(Box::new(view))),
        None => match root {
            Section::Live => return None,
            Section::Home => View::Home(Box::new(home::HomeView::of(world))),
            Section::Gate => View::Gate(gate::GateView::of(world)),
            Section::System => View::About(system::AboutView::of(world)),
            Section::Devices => {
                View::Wall(Box::new(devices::WallView::of(world, nav.devices_page())))
            }
            Section::Alerts | Section::Names => bare(),
        },
    })
}

/// Draws a view on a canvas whose frame is black.
pub fn draw(canvas: &mut Canvas, view: &View) {
    match view {
        View::Bare(lane) => lane::draw(canvas, lane),
        View::Home(home) => home::draw(canvas, home),
        View::Gate(gate) => gate::draw(canvas, gate),
        View::About(about) => system::draw(canvas, about),
        View::Alert(alert) => alerts::draw(canvas, alert),
        View::Wall(wall) => devices::draw_wall(canvas, wall),
        View::Device(device) => devices::draw_device(canvas, device),
        View::Confirm(confirm) => devices::draw_confirm(canvas, confirm),
        View::ItsGate(gate) => devices::draw_its_gate(canvas, gate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::ui::painter::ScreenPainter;

    #[test]
    fn each_p1_screen_has_its_view_and_live_has_none() {
        let kinds = |still: Still| {
            let w = world(still);
            view_of(&still.nav(&w), &w).map(|view| match view {
                View::Bare(_) => "bare",
                View::Home(_) => "home",
                View::Gate(_) => "gate",
                View::About(_) => "about",
                View::Alert(_) => "alert",
                View::Wall(_) => "wall",
                View::Device(_) => "device",
                View::Confirm(_) => "confirm",
                View::ItsGate(_) => "its-gate",
            })
        };
        assert_eq!(kinds(Still::Home), Some("home"));
        assert_eq!(kinds(Still::HomeAlert), Some("home"));
        assert_eq!(kinds(Still::HomeOpen), Some("home"));
        assert_eq!(kinds(Still::Gate), Some("gate"));
        assert_eq!(kinds(Still::About), Some("about"));
        assert_eq!(kinds(Still::Alert), Some("alert"));
        assert_eq!(kinds(Still::Live), None);
        assert_eq!(kinds(Still::LiveGate), None);
        assert_eq!(kinds(Still::Devices), Some("wall"));
        assert_eq!(kinds(Still::Device), Some("device"));
        assert_eq!(kinds(Still::Confirm), Some("confirm"));
        assert_eq!(kinds(Still::Pause), Some("its-gate"));
    }

    #[test]
    fn an_alert_screen_whose_alert_is_gone_is_the_bare_lane_until_the_nav_pops_it() {
        let w = world(Still::Alert);
        let nav = Still::Alert.nav(&w);
        let mut gone = w.clone();
        gone.top_alert = None;
        assert!(matches!(view_of(&nav, &gone), Some(View::Bare(_))));
    }

    #[test]
    fn switching_screens_is_one_painter_and_one_write_each() {
        let mut painter = ScreenPainter::new();
        let w = world(Still::Home);
        let home = view_of(&Nav::new(), &w).expect("HOME has a view");
        assert!(painter.paint(home.clone(), draw).is_some());
        assert_eq!(painter.paint(home, draw), None);
        let w = world(Still::Gate);
        let gate = view_of(&Still::Gate.nav(&w), &w).expect("GATE has a view");
        assert!(painter.paint(gate, draw).is_some());
    }
}
