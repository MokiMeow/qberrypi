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
pub mod names;
pub mod parts;
pub mod system;
pub mod whole;

use crate::os::nav::{Nav, Screen, Section, Whole};
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
    /// SYSTEM: the mimic of the router, the link and the box.
    System(Box<system::SystemView>),
    /// WI-FI: the link drawn large.
    Wifi(Box<system::WifiView>),
    /// VERSION, UP or LOCK: a plain reading with a label, a figure, two lines and the floor.
    Reading(Box<system::ReadingView>),
    /// NIGHT HOURS: the two hours and their `-` and `+` keys.
    Night(Box<system::NightView>),
    /// ABOUT, over SYSTEM.
    About(system::AboutView),
    /// ALERTS: the watch log, or the open watch.
    Alerts(Box<alerts::LogView>),
    /// ALERT: one alert's slip.
    Alert(Box<alerts::AlertView>),
    /// NAMES: the manifold of the day's most refused.
    Names(Box<names::NamesView>),
    /// YOUR LIST: the valves the owner set.
    Yours(Box<names::YoursView>),
    /// NAME: one name as a mimic.
    Name(Box<names::NameView>),
    /// DEVICES, a page of the wall.
    Wall(Box<devices::WallView>),
    /// DEVICE.
    Device(Box<devices::DeviceView>),
    /// CONFIRM, over a device.
    Confirm(Box<devices::ConfirmView>),
    /// ITS GATE, over a device.
    ItsGate(Box<devices::ItsGateView>),
    /// STARTING: the checklist, over the whole screen.
    Starting(Box<whole::StartingView>),
    /// UPDATE: the changeover of the two slots, over the whole screen.
    Update(Box<whole::UpdateView>),
    /// NIGHT: the dark clock, over the whole screen.
    Watch(Box<whole::WatchView>),
    /// The PIN pad, over the screen the action came from.
    Pin(Box<whole::PinView>),
}

/// The view of what the panel shows, or `None` for LIVE, which the Drain draws with its own painter.
///
/// A section whose screens come with a later task is the bare lane. A screen whose thing is not in the
/// world (the nav pops it at its next `refresh`) is the bare lane too, for the moment between. The screens
/// over a root wear the lane of the root the panel came by: DEVICE over ALERT says `ALERTS`, and NAME over
/// an alert's slip too.
pub fn view_of(nav: &Nav, world: &World) -> Option<View> {
    // A state of the whole system covers everything under it, whatever the panel was showing.
    if let Some(whole) = nav.whole() {
        return Some(match whole {
            Whole::Starting => View::Starting(Box::new(whole::StartingView::of(world))),
            Whole::Update => View::Update(Box::new(whole::UpdateView::of(world))),
            Whole::Night => View::Watch(Box::new(whole::WatchView::of(world))),
            Whole::Pin => View::Pin(Box::new(whole::PinView::of(nav.pin(), world))),
        });
    }
    let root = nav.root();
    let bare = || View::Bare(lane::LaneView::of(root, world));
    Some(match nav.top() {
        Some(&Screen::Alert(id)) => {
            alerts::AlertView::of(world, id).map_or_else(bare, |view| View::Alert(Box::new(view)))
        }
        Some(Screen::Yours) => View::Yours(Box::new(names::YoursView::of(world))),
        Some(Screen::Version) => View::Reading(Box::new(system::ReadingView::version(world))),
        Some(Screen::Uptime) => View::Reading(Box::new(system::ReadingView::uptime(world))),
        Some(Screen::Lock) => View::Reading(Box::new(system::ReadingView::lock(world))),
        Some(Screen::Wifi) => View::Wifi(Box::new(system::WifiView::of(world))),
        Some(Screen::NightHours) => {
            View::Night(Box::new(system::NightView::of(world, nav.night_draft())))
        }
        Some(Screen::About) => View::About(system::AboutView::of(world)),
        Some(Screen::Name(name)) => names::NameView::of(world, root, name)
            .map_or_else(bare, |view| View::Name(Box::new(view))),
        Some(&Screen::Device(mac)) => {
            devices::DeviceView::of(world, &mac).map_or_else(bare, |mut view| {
                view.lane = lane::LaneView::of(root, world);
                View::Device(Box::new(view))
            })
        }
        Some(&Screen::Confirm { mac, pass }) => devices::ConfirmView::of(world, &mac, pass)
            .map_or_else(bare, |mut view| {
                view.lane = lane::LaneView::of(root, world);
                View::Confirm(Box::new(view))
            }),
        Some(&Screen::ItsGate(mac)) => {
            devices::ItsGateView::of(world, &mac).map_or_else(bare, |mut view| {
                view.lane = lane::LaneView::of(root, world);
                View::ItsGate(Box::new(view))
            })
        }
        None => match root {
            Section::Live => return None,
            Section::Home => View::Home(Box::new(home::HomeView::of(world))),
            Section::Gate => View::Gate(gate::GateView::of(world)),
            Section::System => View::System(Box::new(system::SystemView::of(world))),
            Section::Devices => {
                View::Wall(Box::new(devices::WallView::of(world, nav.devices_page())))
            }
            Section::Alerts => View::Alerts(Box::new(alerts::LogView::of(world))),
            Section::Names => View::Names(Box::new(names::NamesView::of(world))),
        },
    })
}

/// Draws a view on a canvas whose frame is black.
pub fn draw(canvas: &mut Canvas, view: &View) {
    match view {
        View::Bare(lane) => lane::draw(canvas, lane),
        View::Home(home) => home::draw(canvas, home),
        View::Gate(gate) => gate::draw(canvas, gate),
        View::System(system) => system::draw_system(canvas, system),
        View::Wifi(wifi) => system::draw_wifi(canvas, wifi),
        View::Reading(reading) => system::draw_reading(canvas, reading),
        View::Night(night) => system::draw_night(canvas, night),
        View::About(about) => system::draw_about(canvas, about),
        View::Alerts(log) => alerts::draw_log(canvas, log),
        View::Alert(alert) => alerts::draw_alert(canvas, alert),
        View::Names(names) => names::draw_names(canvas, names),
        View::Yours(yours) => names::draw_yours(canvas, yours),
        View::Name(name) => names::draw_name(canvas, name),
        View::Wall(wall) => devices::draw_wall(canvas, wall),
        View::Device(device) => devices::draw_device(canvas, device),
        View::Confirm(confirm) => devices::draw_confirm(canvas, confirm),
        View::ItsGate(gate) => devices::draw_its_gate(canvas, gate),
        View::Starting(starting) => whole::draw_starting(canvas, starting),
        View::Update(update) => whole::draw_update(canvas, update),
        View::Watch(watch) => whole::draw_watch(canvas, watch),
        View::Pin(pin) => whole::draw_pin(canvas, pin),
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
                View::System(_) => "system",
                View::Wifi(_) => "wifi",
                View::Reading(_) => "reading",
                View::Night(_) => "night",
                View::About(_) => "about",
                View::Alerts(_) => "alerts",
                View::Alert(_) => "alert",
                View::Names(_) => "names",
                View::Yours(_) => "yours",
                View::Name(_) => "name",
                View::Wall(_) => "wall",
                View::Device(_) => "device",
                View::Confirm(_) => "confirm",
                View::ItsGate(_) => "its-gate",
                View::Starting(_) => "starting",
                View::Update(_) => "update",
                View::Watch(_) => "watch",
                View::Pin(_) => "pin",
            })
        };
        assert_eq!(kinds(Still::Home), Some("home"));
        assert_eq!(kinds(Still::HomeAlert), Some("home"));
        assert_eq!(kinds(Still::HomeOpen), Some("home"));
        assert_eq!(kinds(Still::Gate), Some("gate"));
        assert_eq!(kinds(Still::About), Some("about"));
        assert_eq!(kinds(Still::Alert), Some("alert"));
        assert_eq!(kinds(Still::Alerts), Some("alerts"));
        assert_eq!(kinds(Still::AlertsNone), Some("alerts"));
        assert_eq!(kinds(Still::Names), Some("names"));
        assert_eq!(kinds(Still::Yours), Some("yours"));
        assert_eq!(kinds(Still::Name), Some("name"));
        assert_eq!(kinds(Still::NameLocked), Some("name"));
        assert_eq!(kinds(Still::NameThreat), Some("name"));
        assert_eq!(kinds(Still::Live), None);
        assert_eq!(kinds(Still::LiveGate), None);
        assert_eq!(kinds(Still::Devices), Some("wall"));
        assert_eq!(kinds(Still::Device), Some("device"));
        assert_eq!(kinds(Still::Confirm), Some("confirm"));
        assert_eq!(kinds(Still::Pause), Some("its-gate"));
        // The states of the whole system cover whatever is under them: the pad over NAME is the pad.
        assert_eq!(kinds(Still::Boot), Some("starting"));
        assert_eq!(kinds(Still::Night), Some("watch"));
        assert_eq!(kinds(Still::Pin), Some("pin"));
        for still in [
            Still::UpdateReceiving,
            Still::UpdateTrial,
            Still::UpdateConfirmed,
            Still::UpdateRolledBack,
        ] {
            assert_eq!(kinds(still), Some("update"), "{still:?}");
        }
    }

    #[test]
    fn an_alert_screen_whose_alert_is_gone_is_the_bare_lane_until_the_nav_pops_it() {
        let w = world(Still::Alert);
        let nav = Still::Alert.nav(&w);
        let mut gone = w.clone();
        gone.alerts.clear();
        assert!(matches!(view_of(&nav, &gone), Some(View::Bare(_))));
    }

    #[test]
    fn a_device_opened_from_an_alert_wears_the_lane_of_alerts_and_one_from_the_wall_devices() {
        let w = world(Still::Alert);
        let mut nav = Still::Alert.nav(&w);
        nav.open(Screen::Device(crate::os::fixture::mac_of(23)));
        let Some(View::Device(device)) = view_of(&nav, &w) else {
            panic!("DEVICE");
        };
        assert_eq!(
            device.lane.words, "ALERTS",
            "SEE DEVICE: the panel came by ALERTS"
        );
        let wall = world(Still::Device);
        let Some(View::Device(device)) = view_of(&Still::Device.nav(&wall), &wall) else {
            panic!("DEVICE");
        };
        assert_eq!(device.lane.words, "DEVICES");
        // NAME over a threat's slip says ALERTS, and over NAMES says NAMES.
        let threat = world(Still::NameThreat);
        let Some(View::Name(name)) = view_of(&Still::NameThreat.nav(&threat), &threat) else {
            panic!("NAME");
        };
        assert_eq!(name.lane.words, "ALERTS");
    }

    #[test]
    fn a_name_whose_facts_are_not_read_yet_is_the_bare_lane_under_its_root() {
        let w = world(Still::Name);
        let nav = Still::Name.nav(&w);
        let mut unread = w.clone();
        unread.name_facts = None;
        let Some(View::Bare(lane)) = view_of(&nav, &unread) else {
            panic!("the bare lane");
        };
        assert_eq!(lane.words, "NAMES");
        // YOUR LIST has no such moment: it draws what the world has, however little.
        let mut nav = Nav::at(Section::Names);
        nav.open(Screen::Yours);
        assert!(matches!(
            view_of(&nav, &World::default()),
            Some(View::Yours(_))
        ));
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
