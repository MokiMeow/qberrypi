//! Where the panel is, and what a tap does (spec §4).
//!
//! The panel is at a root, one of the sections (or the gate), with a stack of screens opened above it
//! (`stage.js`: `go` replaces the stack, `open` pushes, `back` pops). A screen carries its argument by
//! identity (an alert by the store's id), never by index, so a screen whose thing is gone can tell, and
//! `refresh` pops it.
//!
//! A tap is decided by `decide`, which looks and changes nothing: it is the `Tap` of the spec, what the
//! press would do. `tap` decides and does it to the `Nav`, and gives back the `Intent` the system must
//! carry out, if the tap is one. The touch tracker decides at the press (`touch::TapTracker::feed_with`
//! takes `|x, y| target(...)`) so that a finger that leaves its key is not a tap, and the caller then
//! calls `tap` with the press point.
//!
//! Phase 1 (rulings R1 to R3 of the track lead): NAMES does not exist yet, so the doors to it are
//! inert; SYSTEM is the ABOUT reading, as the root of the section, and its `BACK` and the lane go HOME;
//! ALERTS has no list, so the ALERTS tile and the lane's `n ALERT(S)` open the top alert's ALERT screen
//! directly, whose `DISMISS` acks it and goes HOME; no strong action asks for a PIN, since there is no
//! lock.
//!
//! DEVICES is the wall of places (`screens::devices::wall_tap`: a tile opens DEVICE, `MORE · n` turns the
//! wall's page, which the `Nav` keeps while a screen is open over it and forgets at `go`), DEVICE, CONFIRM
//! and ITS GATE, each carrying its device by hardware address. CONFIRM carries the state it would take as
//! well, so what the screen asked is what a tap does whatever the device has become since. A name row of
//! DEVICE would open NAME, which is P2's. DEVICE is the screen whose device's names the loop watches
//! (`Nav::watched_mac`); `refresh` pops a screen whose device is gone or no longer fits it.
//!
//! LIVE is the Drain, which draws itself; its taps (the key, the gate ask over its field, the lane's
//! `HOME`, the clock column) are `screens::live::decide`, called from `decide`, and the gate ask's state, open
//! or shut with the time it has left, is the `Nav`'s own (`Nav::gate_ask`), where the routing and the caller
//! that draws the Drain both read it.

use crate::os::intent::{GateMove, Intent};
use crate::os::screens::devices;
use crate::os::screens::gate;
use crate::os::screens::home;
use crate::os::screens::lane::Says;
use crate::os::screens::live;
use crate::os::world::{Mac, World};
use crate::ui::look::{FLOOR_SPLIT_X, FLOOR_Y, LANE_CLOCK_X, LANE_HOME_X, LANE_TARGET_Y};

/// The most screens above the root: the deepest path is ALERTS, ALERT, DEVICE, CONFIRM, PIN (spec §4).
pub const MAX_STACK: usize = 6;

/// How long without a touch before the panel goes HOME by itself: a minute.
pub const IDLE_MS: u64 = 60_000;

/// How long the gate ask stays over LIVE's field before it folds away by itself: 8 s (the mockup's 64 frames at 8 a second).
pub const GATE_ASK_MS: u64 = 8_000;

/// The root of the screen the panel shows: HOME, one of the five sections, or the gate, which
/// opens from HOME and is a root of its own (the lane says `GATE` and its word goes back HOME).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// HOME: the six tiles.
    Home,
    /// LIVE: the Drain.
    Live,
    /// DEVICES: the wall.
    Devices,
    /// ALERTS.
    Alerts,
    /// NAMES.
    Names,
    /// SYSTEM.
    System,
    /// The gate's choices, from HOME.
    Gate,
}

impl Section {
    /// The word the lane says for the section, in capitals: `HOME` for HOME.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Home => "HOME",
            Self::Live => "LIVE",
            Self::Devices => "DEVICES",
            Self::Alerts => "ALERTS",
            Self::Names => "NAMES",
            Self::System => "SYSTEM",
            Self::Gate => "GATE",
        }
    }
}

/// A screen opened above a root. Its argument is the identity of the thing it is about.
///
/// Phase 1 has ALERT and the three screens over a device (by hardware address). NAME (by its text) and the
/// readings of SYSTEM come with their tasks; a screen that holds a device's address or a name must
/// not print it in its `Debug`, which is written by hand for that.
#[derive(Clone, PartialEq, Eq)]
pub enum Screen {
    /// ALERT: one alert, by the store's id.
    Alert(u64),
    /// DEVICE: one device.
    Device(Mac),
    /// CONFIRM: before one device's ads change, with the state they would take: `pass` lets them pass,
    /// otherwise they are blocked.
    Confirm {
        /// The device.
        mac: Mac,
        /// Whether the change lets ads pass (`ALLOW ADS`, strong) rather than blocks them.
        pass: bool,
    },
    /// ITS GATE: the choices for opening one device's own gate.
    ItsGate(Mac),
}

impl std::fmt::Debug for Screen {
    /// The screen's name and nothing it is about but what is no secret (which way a confirm goes).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alert(_) => f.write_str("Alert"),
            Self::Device(_) => f.write_str("Device"),
            Self::Confirm { pass, .. } => f
                .debug_struct("Confirm")
                .field("pass", pass)
                .finish_non_exhaustive(),
            Self::ItsGate(_) => f.write_str("ItsGate"),
        }
    }
}

/// A state of the whole system (spec §5.4): it takes the whole screen, draws its own lane and offers no
/// HOME. They come with P3; here the nav only knows that they are not timed out of (the PIN pad is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Whole {
    /// STARTING: the checklist while the panel comes up.
    Starting,
    /// UPDATE: receiving, on trial, confirmed or rolled back.
    Update,
    /// NIGHT: the dark clock. A touch wakes it.
    Night,
    /// The PIN pad: it is over the screen the action came from, and goes back to it.
    Pin,
}

/// What a tap does to where the panel is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Go to the section's root: the stack is replaced (`go`).
    Go(Section),
    /// Go to the section's root and open a screen on it at once (HOME's ALERTS door).
    GoOpen(Section, Screen),
    /// Open a screen above the one that shows (`open`).
    Open(Screen),
    /// Go back one screen (`back`).
    Back,
    /// Unfold the gate's choices over LIVE's field, for `GATE_ASK_MS` (the Drain's key, shut).
    OpenGateAsk,
    /// Fold the gate's choices away.
    CloseGateAsk,
    /// Show this page of the wall of devices (the `MORE · n` place turns it).
    DevicesPage(usize),
}

/// What a press would do (spec §4's `Tap`).
///
/// The spec has `None`, `Nav(change)` and `Act(Intent)`; an act can also move the panel (a gate key
/// opens the gate and goes back, `DISMISS` acks and goes HOME), so `Act` carries the move too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tap {
    /// Nothing: a press on nothing, or on a key that is inert.
    None,
    /// The panel moves.
    Nav(Change),
    /// The system must do something, and the panel may move as well.
    Act(Intent, Option<Change>),
}

/// Where the panel is: a root, the screens above it, and the state of the whole system over it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nav {
    /// The section the panel is under.
    root: Section,
    /// The screens opened above the root, the last on top; at most `MAX_STACK`.
    stack: Vec<Screen>,
    /// A state of the whole system, which covers all of it.
    whole: Option<Whole>,
    /// The gate ask over LIVE's field: the milliseconds it has left, `None` while it is folded away. It is
    /// part of where the panel is, so that a tap is decided by it and the Drain is drawn from it.
    gate_ask: Option<u64>,
    /// The page of the wall of devices that DEVICES shows, from 0. A screen opened over the wall leaves it as
    /// it is, so that back is the page it came from; going to a root starts the wall over at its first page.
    devices_page: usize,
}

impl Default for Nav {
    fn default() -> Self {
        Self::new()
    }
}

impl Nav {
    /// The panel at HOME.
    pub fn new() -> Self {
        Self::at(Section::Home)
    }

    /// The panel at a section's root.
    pub fn at(root: Section) -> Self {
        Self {
            root,
            stack: Vec::with_capacity(MAX_STACK),
            whole: None,
            gate_ask: None,
            devices_page: 0,
        }
    }

    /// The section the panel is under.
    pub fn root(&self) -> Section {
        self.root
    }

    /// The screens above the root, the one that shows last.
    pub fn stack(&self) -> &[Screen] {
        &self.stack
    }

    /// The screen that shows, if one is open above the root.
    pub fn top(&self) -> Option<&Screen> {
        self.stack.last()
    }

    /// The state of the whole system that covers the screen, if there is one.
    pub fn whole(&self) -> Option<Whole> {
        self.whole
    }

    /// Whether the gate ask is unfolded over LIVE's field: the Drain is drawn with it (`Stills::gate_ask`) and
    /// any press folds it away or picks a choice (`screens::live::decide`).
    pub fn gate_ask(&self) -> bool {
        self.gate_ask.is_some()
    }

    /// How many milliseconds the gate ask has left before it folds away; `None` while it is folded away.
    pub fn gate_ask_left_ms(&self) -> Option<u64> {
        self.gate_ask
    }

    /// Whether the panel is at HOME itself, with nothing open and nothing over it.
    pub fn is_home(&self) -> bool {
        self.root == Section::Home && self.stack.is_empty() && self.whole.is_none()
    }

    /// The page of the wall of devices DEVICES shows, from 0; the screen clamps it to the pages there are.
    pub fn devices_page(&self) -> usize {
        self.devices_page
    }

    /// The device whose names the loop is to watch: the one DEVICE shows, while it is the screen that shows
    /// (spec §6.2: names stream only while that device's screen is open; CONFIRM, ITS GATE and a state of
    /// the whole system over it stop the watch, and its ring with it). The loop turns this into the lane
    /// feed's `watch_client` on each change, `None` clearing it.
    pub fn watched_mac(&self) -> Option<Mac> {
        match (self.whole, self.stack.last()) {
            (None, Some(&Screen::Device(mac))) => Some(mac),
            _ => None,
        }
    }

    /// Goes to a section's root: the stack is emptied, no state of the whole system covers it, the gate ask
    /// is folded away (it belongs to LIVE) and the wall of devices is back at its first page.
    pub fn go(&mut self, section: Section) {
        self.root = section;
        self.stack.clear();
        self.whole = None;
        self.gate_ask = None;
        self.devices_page = 0;
    }

    /// Opens a screen above the one that shows (which folds the gate ask away: it is LIVE's own). A stack that
    /// is full has its top replaced.
    pub fn open(&mut self, screen: Screen) {
        self.gate_ask = None;
        if self.stack.len() >= MAX_STACK {
            self.stack.pop();
        }
        self.stack.push(screen);
    }

    /// Goes back one screen. At a root there is none to go back to: the panel goes HOME, which is where
    /// the `BACK` of a section's only screen and of the gate lead.
    pub fn back(&mut self) {
        if self.whole == Some(Whole::Pin) {
            self.whole = None;
        } else if self.stack.pop().is_none() {
            self.go(Section::Home);
        }
    }

    /// Covers the screen with a state of the whole system, or lifts it. A PIN pad covers what it came
    /// from and goes back to it; the others leave to HOME.
    pub fn set_whole(&mut self, whole: Option<Whole>) {
        if whole.is_none() && self.whole.is_some_and(|was| was != Whole::Pin) {
            self.go(Section::Home);
        }
        if whole.is_some() {
            self.gate_ask = None;
        }
        self.whole = whole;
    }

    /// Does what a tap decided.
    pub fn apply(&mut self, change: Change) {
        match change {
            Change::Go(section) => self.go(section),
            Change::GoOpen(section, screen) => {
                self.go(section);
                self.open(screen);
            }
            Change::Open(screen) => self.open(screen),
            Change::Back => self.back(),
            // The ask is LIVE's: nowhere else is there a field to unfold it over.
            Change::OpenGateAsk => {
                self.gate_ask = (self.is_live() && self.stack.is_empty() && self.whole.is_none())
                    .then_some(GATE_ASK_MS);
            }
            Change::CloseGateAsk => self.gate_ask = None,
            Change::DevicesPage(page) => self.devices_page = page,
        }
    }

    /// Whether LIVE is the root.
    fn is_live(&self) -> bool {
        self.root == Section::Live
    }
}

/// What a press at `(x, y)` would do on the screen that shows; nothing is changed. It is also the
/// press's target: two presses with equal results are on the same key, so a finger that slides to
/// another key is not a tap.
pub fn decide(nav: &Nav, world: &World, x: u16, y: u16) -> Tap {
    if nav.whole().is_some() {
        // The states of the whole system answer only their own keys (P3's).
        return Tap::None;
    }
    if nav.is_live() && nav.stack().is_empty() {
        // The Drain's own screen: its taps are its own (the key, the ask, the readings that are doors).
        return live::decide(nav, world, x, y);
    }
    if y < LANE_TARGET_Y {
        return lane(nav, world, x);
    }
    let (floor, left) = (y >= FLOOR_Y, x < FLOOR_SPLIT_X);
    match (nav.root(), nav.top()) {
        (Section::Home, None) => home_door(world, x, y),
        (Section::Gate, None) => gate_key(x, floor),
        // SYSTEM is the ABOUT reading in P1; its one key is on the floor's left.
        (Section::System, None) if floor && left => Tap::Nav(Change::Back),
        (Section::Alerts, Some(&Screen::Alert(id))) if floor && left => alert_dismiss(world, id),
        (Section::Devices, None) => devices::wall_tap(nav.devices_page(), world, x, y),
        // A device's screens do not depend on how the panel came to them (P2's SEE DEVICE comes from ALERTS).
        (_, Some(&Screen::Device(mac))) => devices::device_tap(world, mac, x, y),
        (_, Some(&Screen::Confirm { mac, pass })) => devices::confirm_tap(world, mac, pass, x, y),
        (_, Some(&Screen::ItsGate(mac))) => devices::its_gate_tap(world, mac, x, y),
        _ => Tap::None,
    }
}

/// Decides what a press at `(x, y)` does, does it to the panel, and gives back what the system must do.
///
/// A strong intent is returned as it is: P1 has no lock (ruling R3). P3 holds it in the PIN state when
/// `world.lock_on`, and the fourth digit gives it back.
pub fn tap(nav: &mut Nav, world: &World, x: u16, y: u16) -> Option<Intent> {
    match decide(nav, world, x, y) {
        Tap::None => None,
        Tap::Nav(change) => {
            nav.apply(change);
            None
        }
        Tap::Act(intent, change) => {
            if let Some(change) = change {
                nav.apply(change);
            }
            Some(intent)
        }
    }
}

/// Lets `elapsed_ms` pass for the gate ask, which folds away by itself once it has been open `GATE_ASK_MS`.
/// Says whether it folded away now (the Drain is to be drawn without it). The caller runs it each frame it
/// is on LIVE, with the time since its last call; a touch does not start it over, since any touch ends it.
pub fn age_ask(nav: &mut Nav, elapsed_ms: u64) -> bool {
    let Some(left) = nav.gate_ask else {
        return false;
    };
    if elapsed_ms >= left {
        nav.gate_ask = None;
        true
    } else {
        nav.gate_ask = Some(left - elapsed_ms);
        false
    }
}

/// Takes the panel HOME after a minute without a touch, from any screen but HOME and the states of the
/// whole system; a PIN pad that is left alone goes back to where it came from. `idle_ms` is how long it
/// has been since the last touch. Says whether the panel moved.
pub fn idle(nav: &mut Nav, idle_ms: u64) -> bool {
    if idle_ms < IDLE_MS {
        return false;
    }
    match nav.whole() {
        Some(Whole::Pin) => nav.set_whole(None),
        Some(Whole::Starting | Whole::Update | Whole::Night) => return false,
        None if nav.is_home() => return false,
        None => nav.go(Section::Home),
    }
    true
}

/// Whether the thing a screen is about is gone, or no longer fits the screen.
fn gone(world: &World, screen: &Screen) -> bool {
    match *screen {
        Screen::Alert(id) => world.top_alert.as_ref().is_none_or(|alert| alert.id != id),
        Screen::Device(mac) => world.device(&mac).is_none(),
        Screen::Confirm { mac, pass } => !devices::confirm_valid(world, &mac, pass),
        Screen::ItsGate(mac) => !devices::gate_valid(world, &mac),
    }
}

/// Pops the screens whose thing is gone, and says whether the panel moved. The caller runs it each time it
/// has a new `World`.
///
/// - An ALERT whose alert is no longer the one waiting (it was dismissed on the desktop, or a newer one came).
///   P1's World carries the top alert only, so the screen cannot show another one, and the panel goes
///   HOME, where the tile and the lane say what waits.
/// - A DEVICE whose address left the world (the device was forgotten) goes back to the wall, and so does
///   everything opened over it.
/// - A CONFIRM that is not true any more (the desktop changed the device's ads, the configuration took
///   over, it went) and an ITS GATE with no gate to open (its ads pass, the gate is open) go back to the screen under
///   them, which shows the device as it is.
/// - The wall's page, if the wall shrank under it, moves to the last page there is.
pub fn refresh(nav: &mut Nav, world: &World) -> bool {
    let mut moved = false;
    if let Some(at) = nav.stack.iter().position(|screen| gone(world, screen)) {
        moved = true;
        if matches!(nav.stack[at], Screen::Alert(_)) {
            // P2 has the list to go back to; here the section has no page of its own.
            nav.go(Section::Home);
        } else {
            nav.stack.truncate(at);
        }
    }
    let page = devices::clamp_page(nav.devices_page, world.devices.len());
    if page != nav.devices_page {
        nav.devices_page = page;
        moved = true;
    }
    moved
}

/// The lane (spec §3.1): HOME on the left, SYSTEM on the clock, and between them the section's word.
fn lane(nav: &Nav, world: &World, x: u16) -> Tap {
    if x < LANE_HOME_X {
        return Tap::Nav(Change::Go(Section::Home));
    }
    let root = nav.root();
    if x >= LANE_CLOCK_X {
        return Tap::Nav(Change::Go(Section::System));
    }
    match root {
        // The words say the system, and are the door to what they say.
        Section::Home => match Says::of(world) {
            Says::Alerts(_) => top_alert_door(world),
            Says::GateOpen => Tap::Act(Intent::Gate(GateMove::Shut), None),
            Says::AllWell => Tap::Nav(Change::Go(Section::Live)),
        },
        // The gate is a root of its own, from HOME; ALERTS has no page of its own in P1 (no list).
        Section::Gate | Section::Alerts => Tap::Nav(Change::Go(Section::Home)),
        section => Tap::Nav(Change::Go(section)),
    }
}

/// The ALERT screen of the top alert, from HOME; nothing when the world does not hold one.
fn top_alert_door(world: &World) -> Tap {
    world.top_alert.as_ref().map_or(Tap::None, |alert| {
        Tap::Nav(Change::GoOpen(Section::Alerts, Screen::Alert(alert.id)))
    })
}

/// HOME's six tiles (`wallHit`): LIVE, DEVICES, ALERTS; the gate key, NAMES, SYSTEM.
fn home_door(world: &World, x: u16, y: u16) -> Tap {
    match home::wall_hit(x, y) {
        Some(home::LIVE) => Tap::Nav(Change::Go(Section::Live)),
        Some(home::DEVICES) => Tap::Nav(Change::Go(Section::Devices)),
        // With none waiting the tile is inert (ruling R2).
        Some(home::ALERTS) => top_alert_door(world),
        // The key: shut, it opens the gate's choices; open, it shuts the gate at once.
        Some(home::GATE) if world.gate_open() => Tap::Act(Intent::Gate(GateMove::Shut), None),
        Some(home::GATE) => Tap::Nav(Change::Go(Section::Gate)),
        // NAMES does not exist in P1 (ruling R1).
        Some(home::NAMES) => Tap::None,
        Some(home::SYSTEM) => Tap::Nav(Change::Go(Section::System)),
        _ => Tap::None,
    }
}

/// GATE's four keys: `KEEP SHUT` goes back; 5 MIN, 30 MIN and 1 HOUR open the gate and go back.
fn gate_key(x: u16, floor: bool) -> Tap {
    if !floor {
        return Tap::None;
    }
    match gate::key_secs(x) {
        Some(secs) => Tap::Act(Intent::Gate(GateMove::Open(secs)), Some(Change::Back)),
        None => Tap::Nav(Change::Back),
    }
}

/// ALERT's `DISMISS`: acks the alert the screen is about and goes HOME (P1 has no list to go back to).
fn alert_dismiss(world: &World, id: u64) -> Tap {
    // The key is drawn only while the screen's alert is the one the world holds.
    if world.top_alert.as_ref().is_none_or(|alert| alert.id != id) {
        return Tap::None;
    }
    Tap::Act(Intent::Ack(id), Some(Change::Go(Section::Home)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, mac_of, world};
    use crate::os::world::Pause;

    /// Taps once at a point on a nav, and says what the system is asked to do.
    fn press(nav: &mut Nav, world: &World, x: u16, y: u16) -> Option<Intent> {
        tap(nav, world, x, y)
    }

    /// A nav at HOME that has gone to a root and opened what the fixture's still has open.
    fn at(still: Still) -> (Nav, World) {
        let world = world(still);
        (still.nav(&world), world)
    }

    #[test]
    fn the_words_fit_the_lane_between_home_and_the_clock() {
        for section in [
            Section::Home,
            Section::Live,
            Section::Devices,
            Section::Alerts,
            Section::Names,
            Section::System,
            Section::Gate,
        ] {
            // Cells 5 to 11 hold a section's word: seven letters at most.
            assert!(section.word().len() <= 7, "{section:?}");
            assert!(section.word().chars().all(|c| c.is_ascii_uppercase()));
        }
        assert_eq!(Section::Devices.word(), "DEVICES");
    }

    #[test]
    fn go_replaces_the_stack_open_pushes_and_back_pops() {
        let mut nav = Nav::new();
        assert!(nav.is_home() && nav.stack().is_empty());
        nav.go(Section::Alerts);
        nav.open(Screen::Alert(1));
        nav.open(Screen::Alert(2));
        assert_eq!(nav.root(), Section::Alerts);
        assert_eq!(nav.stack(), [Screen::Alert(1), Screen::Alert(2)]);
        assert_eq!(nav.top(), Some(&Screen::Alert(2)));
        nav.back();
        assert_eq!(nav.stack(), [Screen::Alert(1)]);
        nav.go(Section::Devices);
        assert_eq!((nav.root(), nav.stack().len()), (Section::Devices, 0));
        // At a root, back is HOME.
        nav.back();
        assert!(nav.is_home());
        nav.back();
        assert!(nav.is_home(), "HOME has nowhere to go back to");
    }

    #[test]
    fn a_push_past_the_bound_replaces_the_top() {
        let mut nav = Nav::at(Section::Alerts);
        for id in 0..MAX_STACK as u64 {
            nav.open(Screen::Alert(id));
        }
        assert_eq!(nav.stack().len(), MAX_STACK);
        nav.open(Screen::Alert(100));
        assert_eq!(nav.stack().len(), MAX_STACK, "no deeper");
        assert_eq!(nav.top(), Some(&Screen::Alert(100)));
        assert_eq!(
            nav.stack()[MAX_STACK - 2],
            Screen::Alert(MAX_STACK as u64 - 2)
        );
        for _ in 0..MAX_STACK {
            nav.back();
        }
        assert!(nav.stack().is_empty() && nav.root() == Section::Alerts);
    }

    #[test]
    fn a_screen_is_about_an_identity_and_debug_says_none_of_it() {
        let said = format!(
            "{:?} {:?}",
            Screen::Alert(987_654),
            Nav::at(Section::Alerts)
        );
        assert!(!said.contains("987654"), "{said}");
        let tap = Tap::Act(
            Intent::Ack(987_654),
            Some(Change::Open(Screen::Alert(987_654))),
        );
        assert!(
            format!("{tap:?}").contains("987654"),
            "an alert's id is no secret"
        );
    }

    // ---- the tap table ----

    /// Where the middle of HOME's tile `i` is.
    fn tile_centre(i: u16) -> (u16, u16) {
        (8 + (i % 3) * 157 + 75, 80 + (i / 3) * 116 + 54)
    }

    #[test]
    fn homes_six_doors_lead_where_the_map_says() {
        let (home, world) = at(Still::Home);
        let go = |tile: u16| {
            let mut nav = home.clone();
            let (x, y) = tile_centre(tile);
            let intent = press(&mut nav, &world, x, y);
            (nav.root(), nav.stack().len(), intent)
        };
        assert_eq!(go(0), (Section::Live, 0, None));
        assert_eq!(go(1), (Section::Devices, 0, None));
        assert_eq!(
            go(2),
            (Section::Home, 0, None),
            "ALERTS, none waiting: inert"
        );
        assert_eq!(
            go(3),
            (Section::Gate, 0, None),
            "the gate key, shut: the choices"
        );
        assert_eq!(go(4), (Section::Home, 0, None), "NAMES is inert in P1");
        assert_eq!(go(5), (Section::System, 0, None));
        // None of them changed the nav they were cloned from.
        assert!(home.is_home());
    }

    #[test]
    fn the_alerts_tile_opens_the_top_alert_when_one_waits() {
        let (mut nav, world) = at(Still::HomeAlert);
        let (x, y) = tile_centre(2);
        assert_eq!(press(&mut nav, &world, x, y), None);
        assert_eq!(nav.root(), Section::Alerts);
        assert_eq!(nav.stack(), [Screen::Alert(4)], "the store's top, by id");
    }

    #[test]
    fn the_gate_key_opens_the_choices_when_shut_and_shuts_when_open() {
        let (mut nav, world) = at(Still::HomeOpen);
        let (x, y) = tile_centre(3);
        assert_eq!(
            press(&mut nav, &world, x, y),
            Some(Intent::Gate(GateMove::Shut))
        );
        assert!(nav.is_home(), "it shuts at once, and the panel stays");
        let (mut nav, world) = at(Still::Home);
        assert_eq!(press(&mut nav, &world, x, y), None);
        assert_eq!(nav.root(), Section::Gate);
    }

    #[test]
    fn a_door_is_its_tile_and_a_margin_and_the_gaps_are_nothing() {
        let (nav, world) = at(Still::Home);
        // `wallHit`: rows from y 76 (the second from 192), columns from x 4 (the second from 161, the third
        // from 318); the last row and column go on to the screen's edge.
        let live = Tap::Nav(Change::Go(Section::Live));
        let devices = Tap::Nav(Change::Go(Section::Devices));
        for (x, y) in [(8, 80), (157, 187), (4, 76), (160, 191), (0, 100)] {
            assert_eq!(decide(&nav, &world, x, y), live, "({x}, {y})");
        }
        for (x, y) in [(161, 100), (317, 191), (200, 76)] {
            assert_eq!(decide(&nav, &world, x, y), devices, "({x}, {y})");
        }
        // The rows between the lane and the tiles are nothing.
        assert_eq!(decide(&nav, &world, 100, 75), Tap::None);
        assert_eq!(decide(&nav, &world, 100, 72), Tap::None);
        assert_eq!(
            decide(&nav, &world, 479, 319),
            Tap::Nav(Change::Go(Section::System))
        );
        // The mockup's `wallHit` has x < 4 on the second row reach the third tile (column -1 plus 3); here
        // it is the first column, the gate key.
        assert_eq!(
            decide(&nav, &world, 0, 300),
            Tap::Nav(Change::Go(Section::Gate))
        );
    }

    #[test]
    fn the_lane_on_home_says_a_door_and_the_clock_is_system() {
        // ALL WELL goes to LIVE; GATE OPEN shuts the gate; n ALERTS opens the top alert.
        let (mut nav, world) = at(Still::Home);
        assert_eq!(press(&mut nav, &world, 200, 40), None);
        assert_eq!(nav.root(), Section::Live);
        let (mut nav, world) = at(Still::HomeOpen);
        assert_eq!(
            press(&mut nav, &world, 200, 40),
            Some(Intent::Gate(GateMove::Shut))
        );
        assert!(nav.is_home());
        let (mut nav, world) = at(Still::HomeAlert);
        assert_eq!(press(&mut nav, &world, 200, 40), None);
        assert_eq!(
            (nav.root(), nav.stack()),
            (Section::Alerts, &[Screen::Alert(4)][..])
        );
        // Alerts come before the gate, as the lane's words do.
        let (mut nav, mut world) = at(Still::HomeAlert);
        world.blocking = Some(Pause {
            left_secs: 100,
            total_secs: 300,
        });
        assert_eq!(press(&mut nav, &world, 200, 40), None);
        assert_eq!(nav.root(), Section::Alerts);
        // The clock opens SYSTEM, the first cells HOME (which is where it is).
        let (mut nav, world) = at(Still::Home);
        assert_eq!(press(&mut nav, &world, 400, 40), None);
        assert_eq!(nav.root(), Section::System);
        assert_eq!(press(&mut nav, &world, 50, 40), None);
        assert!(nav.is_home());
    }

    #[test]
    fn the_lanes_three_targets_are_where_the_spec_puts_them() {
        let (nav, world) = at(Still::Gate);
        let home = Tap::Nav(Change::Go(Section::Home));
        let system = Tap::Nav(Change::Go(Section::System));
        for (x, y, tap) in [
            (0, 0, &home),
            (107, 71, &home),
            (108, 71, &home), // the gate's own word goes HOME too
            (347, 0, &home),
            (348, 0, &system),
            (479, 71, &system),
        ] {
            assert_eq!(&decide(&nav, &world, x, y), tap, "({x}, {y})");
        }
        // Row 72 is no longer the lane.
        assert_eq!(decide(&nav, &world, 400, 72), Tap::None);
    }

    #[test]
    fn the_lane_on_every_p1_screen() {
        // (the screen, what its section's word does)
        let cases = [
            (Still::Gate, Tap::Nav(Change::Go(Section::Home))),
            // ALERTS has no page of its own in P1: its word goes HOME.
            (Still::Alert, Tap::Nav(Change::Go(Section::Home))),
            // ABOUT is SYSTEM's root: its word is where it is.
            (Still::About, Tap::Nav(Change::Go(Section::System))),
        ];
        for (still, word) in cases {
            let (nav, world) = at(still);
            assert_eq!(
                decide(&nav, &world, 10, 40),
                Tap::Nav(Change::Go(Section::Home)),
                "{still:?}: HOME"
            );
            assert_eq!(decide(&nav, &world, 200, 40), word, "{still:?}: the word");
            assert_eq!(
                decide(&nav, &world, 400, 40),
                Tap::Nav(Change::Go(Section::System)),
                "{still:?}: the clock"
            );
        }
    }

    #[test]
    fn the_lane_on_live_is_only_home_until_the_live_task() {
        let (nav, world) = at(Still::Live);
        assert_eq!(
            decide(&nav, &world, 10, 40),
            Tap::Nav(Change::Go(Section::Home))
        );
        assert_eq!(decide(&nav, &world, 200, 40), Tap::None);
        assert_eq!(decide(&nav, &world, 400, 40), Tap::None);
    }

    #[test]
    fn the_gates_four_keys() {
        // The floor is y >= 236; four keys, floor((x - 4) / 118).
        let (nav, world) = at(Still::Gate);
        let key = |x: u16, y: u16| decide(&nav, &world, x, y);
        let open = |secs| Tap::Act(Intent::Gate(GateMove::Open(secs)), Some(Change::Back));
        let keep = Tap::Nav(Change::Back);
        assert_eq!(key(8, 240), keep);
        assert_eq!(key(121, 300), keep);
        assert_eq!(key(122, 240), open(300));
        assert_eq!(key(239, 319), open(300));
        assert_eq!(key(240, 236), open(1800));
        assert_eq!(key(357, 300), open(1800));
        assert_eq!(key(358, 300), open(3600));
        assert_eq!(key(479, 319), open(3600));
        assert_eq!(key(0, 300), keep, "left of the first key's margin");
        // Above the floor there is the slab, which is nothing.
        assert_eq!(key(300, 235), Tap::None);
        assert_eq!(key(200, 100), Tap::None);
    }

    #[test]
    fn a_gate_key_opens_the_gate_and_goes_back() {
        for (x, secs) in [
            (60, None),
            (180, Some(300)),
            (300, Some(1800)),
            (420, Some(3600)),
        ] {
            let (mut nav, world) = at(Still::Home);
            let (gx, gy) = tile_centre(3);
            press(&mut nav, &world, gx, gy);
            assert_eq!(nav.root(), Section::Gate);
            let intent = press(&mut nav, &world, x, 280);
            assert_eq!(intent, secs.map(|secs| Intent::Gate(GateMove::Open(secs))));
            assert!(nav.is_home(), "back from the gate is HOME");
        }
    }

    #[test]
    fn about_has_one_key_back_which_is_home() {
        let (mut nav, world) = at(Still::About);
        assert_eq!((nav.root(), nav.stack().len()), (Section::System, 0));
        // The right half of the floor has no key; the slab area is nothing.
        assert_eq!(press(&mut nav, &world, 400, 280), None);
        assert_eq!(press(&mut nav, &world, 100, 150), None);
        assert_eq!(nav.root(), Section::System);
        assert_eq!(decide(&nav, &world, 239, 236), Tap::Nav(Change::Back));
        assert_eq!(decide(&nav, &world, 240, 236), Tap::None);
        assert_eq!(press(&mut nav, &world, 100, 280), None);
        assert!(nav.is_home());
    }

    #[test]
    fn alert_has_dismiss_which_acks_and_goes_home_and_nothing_else() {
        let (mut nav, world) = at(Still::Alert);
        assert_eq!(nav.stack(), [Screen::Alert(4)]);
        // The right half of the floor stays black and inert (no SEE DEVICE in P1), as does the slab.
        assert_eq!(press(&mut nav, &world, 400, 280), None);
        assert_eq!(press(&mut nav, &world, 100, 150), None);
        assert_eq!(nav.stack(), [Screen::Alert(4)]);
        assert_eq!(press(&mut nav, &world, 100, 280), Some(Intent::Ack(4)));
        assert!(nav.is_home());
        // A screen whose alert is not the one the world holds has no key.
        let mut nav = Nav::at(Section::Alerts);
        nav.open(Screen::Alert(3));
        assert_eq!(press(&mut nav, &world, 100, 280), None);
    }

    #[test]
    fn the_idle_panel_goes_home_after_a_minute_from_every_screen_but_home() {
        for still in [
            Still::Gate,
            Still::Alert,
            Still::About,
            Still::Live,
            Still::Devices,
        ] {
            let (mut nav, world) = at(still);
            let _ = &world;
            assert!(!idle(&mut nav, 0), "{still:?}");
            assert!(!idle(&mut nav, IDLE_MS - 1), "{still:?}: 59.999 s");
            assert!(!nav.is_home());
            assert!(idle(&mut nav, IDLE_MS), "{still:?}: 60 s");
            assert!(nav.is_home() && nav.stack().is_empty());
        }
        let mut nav = Nav::new();
        assert!(!idle(&mut nav, u64::MAX), "HOME stays HOME");
    }

    #[test]
    fn the_states_of_the_whole_system_are_not_timed_out_of_and_ignore_taps() {
        let world = world(Still::Home);
        for whole in [Whole::Starting, Whole::Update, Whole::Night] {
            let mut nav = Nav::at(Section::Live);
            nav.set_whole(Some(whole));
            assert!(!idle(&mut nav, u64::MAX), "{whole:?}");
            assert_eq!(nav.whole(), Some(whole));
            assert_eq!(tap(&mut nav, &world, 10, 40), None, "no HOME on the lane");
            assert_eq!(nav.whole(), Some(whole));
            nav.set_whole(None);
            assert!(nav.is_home(), "{whole:?} ends at HOME");
        }
        // The pad is timed out, back to what it came from, and cancelling goes there too.
        let mut nav = Nav::at(Section::Gate);
        nav.set_whole(Some(Whole::Pin));
        assert!(idle(&mut nav, IDLE_MS));
        assert_eq!((nav.root(), nav.whole()), (Section::Gate, None));
        nav.set_whole(Some(Whole::Pin));
        nav.back();
        assert_eq!((nav.root(), nav.whole()), (Section::Gate, None));
    }

    #[test]
    fn a_screen_whose_alert_is_gone_pops_to_home() {
        let (mut nav, mut world) = at(Still::Alert);
        assert!(!refresh(&mut nav, &world));
        assert_eq!(nav.stack(), [Screen::Alert(4)]);
        // Dismissed on the desktop: none waits.
        world.top_alert = None;
        world.alerts_waiting = 0;
        assert!(refresh(&mut nav, &world));
        assert!(nav.is_home());
        // A newer alert is the top now: this screen cannot show the older one.
        let (mut nav, mut world) = at(Still::Alert);
        if let Some(top) = world.top_alert.as_mut() {
            top.id = 5;
        }
        assert!(refresh(&mut nav, &world));
        assert!(nav.is_home());
        // HOME and the roots have nothing to pop.
        let (mut nav, world) = at(Still::Gate);
        assert!(!refresh(&mut nav, &world));
        assert_eq!(nav.root(), Section::Gate);
    }

    // ---- DEVICES: the wall, DEVICE, CONFIRM and ITS GATE ----

    /// The wall's tile `i` of the house, tapped at its middle, from the wall.
    fn tile_tap(i: usize) -> Tap {
        let (nav, world) = at(Still::Devices);
        let r = devices::place_rect(i);
        decide(&nav, &world, r.x + 55, r.y + 35)
    }

    #[test]
    fn a_device_screen_is_where_the_map_puts_it_and_each_key_is_a_door() {
        // The wall's tile → DEVICE; its two keys → CONFIRM and ITS GATE; their keys → back, or the change and back.
        let mut nav = Nav::at(Section::Devices);
        let world = world(Still::Devices);
        let sam = mac_of(23);
        assert_eq!(press(&mut nav, &world, 180, 110), None);
        assert_eq!(nav.stack(), [Screen::Device(sam)]);
        assert_eq!(press(&mut nav, &world, 100, 280), None);
        assert_eq!(
            nav.stack(),
            [
                Screen::Device(sam),
                Screen::Confirm {
                    mac: sam,
                    pass: false
                }
            ]
        );
        // KEEP: back to DEVICE, nothing asked of the system.
        assert_eq!(press(&mut nav, &world, 100, 280), None);
        assert_eq!(nav.stack(), [Screen::Device(sam)]);
        // The lane's HOME from anywhere is HOME, and its word the wall.
        assert_eq!(press(&mut nav, &world, 200, 40), None);
        assert_eq!((nav.root(), nav.stack().len()), (Section::Devices, 0));
        assert_eq!(press(&mut nav, &world, 180, 110), None);
        assert_eq!(press(&mut nav, &world, 10, 40), None);
        assert!(nav.is_home());
    }

    #[test]
    fn the_wall_taps_are_the_tile_table() {
        let want = |octet: u8| Tap::Nav(Change::Open(Screen::Device(mac_of(octet))));
        let octets = [12, 23, 31, 40, 57, 64, 143, 216, 1, 2];
        for (i, octet) in octets.into_iter().enumerate() {
            assert_eq!(tile_tap(i), want(octet), "place {i}");
        }
        assert_eq!(tile_tap(10), Tap::None, "NEW ONES");
        assert_eq!(tile_tap(11), Tap::None);
    }

    #[test]
    fn a_device_screens_lane_goes_home_the_wall_or_system() {
        for still in [Still::Device, Still::Confirm, Still::Pause] {
            let (nav, world) = at(still);
            assert_eq!(
                decide(&nav, &world, 10, 40),
                Tap::Nav(Change::Go(Section::Home))
            );
            assert_eq!(
                decide(&nav, &world, 200, 40),
                Tap::Nav(Change::Go(Section::Devices)),
                "{still:?}: DEVICES goes back to the wall"
            );
            assert_eq!(
                decide(&nav, &world, 400, 40),
                Tap::Nav(Change::Go(Section::System))
            );
        }
    }

    #[test]
    fn the_wall_keeps_its_page_under_a_screen_and_starts_over_at_a_root() {
        let mut crowd = world(Still::Devices);
        for octet in 100..120 {
            let (name, named) = crate::os::world::display_name(None, None, octet);
            crowd.push_device(crate::os::world::DeviceRow {
                mac: mac_of(octet),
                name,
                named,
                octet,
                state: crate::os::world::DeviceState::AdsPass,
                fixed: false,
                asking: false,
            });
        }
        let mut nav = Nav::at(Section::Devices);
        assert_eq!(nav.devices_page(), 0);
        // The twelfth place turns the page; again, and the third page's turner is back to the first.
        let turner = devices::place_rect(11);
        let (x, y) = (turner.x + 55, turner.y + 35);
        assert_eq!(press(&mut nav, &crowd, x, y), None);
        assert_eq!(nav.devices_page(), 1);
        // A device of the second page opens, and back is the second page.
        assert_eq!(press(&mut nav, &crowd, 20, 100), None);
        assert_eq!(nav.top(), Some(&Screen::Device(mac_of(101))));
        assert_eq!(nav.devices_page(), 1);
        nav.back();
        assert_eq!((nav.devices_page(), nav.stack().len()), (1, 0));
        assert_eq!(press(&mut nav, &crowd, x, y), None);
        assert_eq!(press(&mut nav, &crowd, x, y), None);
        assert_eq!(nav.devices_page(), 0, "the third page's turner goes round");
        // The lane's word, HOME and SYSTEM start the wall over.
        assert_eq!(press(&mut nav, &crowd, x, y), None);
        assert_eq!(nav.devices_page(), 1);
        nav.go(Section::Devices);
        assert_eq!(nav.devices_page(), 0);
        // The wall shrank under a page that was open: the last page there is, and the panel says it moved.
        nav.apply(Change::DevicesPage(2));
        assert!(!refresh(&mut nav, &crowd));
        assert_eq!(nav.devices_page(), 2);
        let small = world(Still::Devices);
        assert!(refresh(&mut nav, &small));
        assert_eq!(nav.devices_page(), 0);
        assert!(!refresh(&mut nav, &small));
    }

    #[test]
    fn only_the_device_screen_is_watched() {
        let sam = mac_of(23);
        let world = world(Still::Devices);
        let mut nav = Nav::at(Section::Devices);
        assert_eq!(nav.watched_mac(), None, "the wall watches nothing");
        press(&mut nav, &world, 180, 110);
        assert_eq!(nav.watched_mac(), Some(sam));
        // CONFIRM and ITS GATE are over the device: its names stop.
        press(&mut nav, &world, 100, 280);
        assert_eq!(nav.watched_mac(), None);
        nav.back();
        assert_eq!(nav.watched_mac(), Some(sam), "back to DEVICE watches again");
        // A state of the whole system over it covers it.
        nav.set_whole(Some(Whole::Pin));
        assert_eq!(nav.watched_mac(), None);
        nav.set_whole(None);
        assert_eq!(nav.watched_mac(), Some(sam));
        nav.go(Section::Home);
        assert_eq!(nav.watched_mac(), None);
        for still in [
            Still::Home,
            Still::Gate,
            Still::Alert,
            Still::Confirm,
            Still::Pause,
        ] {
            let (nav, _) = at(still);
            assert_eq!(nav.watched_mac(), None, "{still:?}");
        }
        assert_eq!(at(Still::Device).0.watched_mac(), Still::Device.device());
    }

    #[test]
    fn a_device_that_left_the_world_pops_its_screens_to_the_wall() {
        // DEVICE, and everything opened over it, goes with the device.
        for still in [Still::Device, Still::Confirm, Still::Pause] {
            let (mut nav, mut world) = at(still);
            assert!(!refresh(&mut nav, &world), "{still:?}: nothing moved yet");
            let mac = still.device().expect("a device");
            world.devices.retain(|row| row.mac != mac);
            assert!(refresh(&mut nav, &world), "{still:?}");
            assert_eq!(
                (nav.root(), nav.stack().len()),
                (Section::Devices, 0),
                "{still:?}"
            );
            assert!(!refresh(&mut nav, &world));
        }
        // Another device leaving changes nothing.
        let (mut nav, mut world) = at(Still::Device);
        world.devices.retain(|row| row.octet != 216);
        assert!(!refresh(&mut nav, &world));
        assert_eq!(nav.stack().len(), 1);
    }

    #[test]
    fn a_confirm_or_a_gate_that_is_not_true_any_more_goes_back_to_the_device() {
        use crate::os::world::DeviceState;
        // The desktop let Sam's phone's ads pass already: the question has no point.
        let (mut nav, mut world) = at(Still::Confirm);
        world.devices[1].state = DeviceState::Blocked;
        assert!(refresh(&mut nav, &world));
        assert_eq!(nav.stack(), [Screen::Device(mac_of(23))]);
        // The configuration took its state over.
        let (mut nav, mut world) = at(Still::Confirm);
        world.devices[1].fixed = true;
        assert!(refresh(&mut nav, &world));
        assert_eq!(nav.stack(), [Screen::Device(mac_of(23))]);
        // The living room TV's gate was opened elsewhere (or its ads were let pass): nothing to open.
        let (mut nav, mut world) = at(Still::Pause);
        world.devices[2].state = DeviceState::Open { left_secs: 100 };
        assert!(refresh(&mut nav, &world));
        assert_eq!(nav.stack(), [Screen::Device(mac_of(31))]);
        let (mut nav, mut world) = at(Still::Pause);
        world.devices[2].state = DeviceState::AdsPass;
        assert!(refresh(&mut nav, &world));
        assert_eq!(nav.stack(), [Screen::Device(mac_of(31))]);
        // DEVICE itself stays: it shows whatever the device is.
        let (mut nav, mut world) = at(Still::Device);
        world.devices[2].state = DeviceState::Open { left_secs: 100 };
        assert!(!refresh(&mut nav, &world));
        // And ALERT's own rule is untouched by the device screens.
        let (mut nav, world) = at(Still::Alert);
        assert!(!refresh(&mut nav, &world));
    }

    #[test]
    fn the_screens_confirm_target_is_what_a_tap_asks_for() {
        // The confirm asked to let ads pass for the TV (blocked). Had the desktop let them pass in between,
        // the tap on `ALLOW ADS` would ask for nothing, and `refresh` takes the screen away.
        let mut world = world(Still::Devices);
        let tv = mac_of(31);
        let mut nav = Nav::at(Section::Devices);
        nav.open(Screen::Device(tv));
        nav.open(Screen::Confirm {
            mac: tv,
            pass: true,
        });
        assert_eq!(
            decide(&nav, &world, 400, 280),
            Tap::Act(
                Intent::DeviceAds {
                    mac: tv,
                    pass: true
                },
                Some(Change::Back)
            )
        );
        world.devices[2].state = crate::os::world::DeviceState::AdsPass;
        assert_eq!(decide(&nav, &world, 400, 280), Tap::None);
        assert_eq!(decide(&nav, &world, 100, 280), Tap::None, "and KEEP too");
        assert!(refresh(&mut nav, &world));
        assert_eq!(nav.top(), Some(&Screen::Device(tv)));
    }

    #[test]
    fn a_device_screens_debug_says_no_address() {
        let said = format!(
            "{:?} {:?} {:?} {:?}",
            Screen::Device([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
            Screen::ItsGate([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
            Screen::Confirm {
                mac: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                pass: true
            },
            at(Still::Confirm).0
        );
        for secret in ["170", "187", "238", "255", "0xaa", "[2, 0", "aa"] {
            assert!(!said.contains(secret), "{said} holds {secret}");
        }
        assert!(said.contains("Confirm") && said.contains("pass: true"));
    }

    #[test]
    fn the_tracker_yields_the_press_point_and_a_finger_that_slides_off_its_key_taps_nothing() {
        use crate::touch::{
            TapTracker,
            TouchEvent::{Down, Move, Up},
        };
        let (nav, world) = at(Still::Home);
        let target = |x, y| match decide(&nav, &world, x, y) {
            Tap::None => None,
            tap => Some(tap),
        };
        let mut tracker: TapTracker<Tap> = TapTracker::default();
        let mut feed = |at_ms, event| tracker.feed_with(at_ms, event, target);
        // A press of 50 ms on SYSTEM's tile: the tap is where it came down, and `tap` takes it from there.
        assert_eq!(feed(0, Down { x: 400, y: 250 }), None);
        let (pressed, point) = feed(50, Up).expect("a tap");
        assert_eq!(point, (400, 250));
        assert_eq!(pressed, Tap::Nav(Change::Go(Section::System)));
        // Too short, or slid onto another tile, or begun on a gap: nothing.
        assert_eq!(feed(100, Down { x: 400, y: 250 }), None);
        assert_eq!(feed(120, Up), None);
        assert_eq!(feed(200, Down { x: 400, y: 250 }), None);
        assert_eq!(feed(210, Move { x: 250, y: 250 }), None);
        assert_eq!(feed(300, Up), None);
        assert_eq!(feed(400, Down { x: 100, y: 74 }), None);
        assert_eq!(feed(500, Up), None, "the gap under the lane is no target");
        // Moving inside the same key keeps the press, and the point is where it came down.
        assert_eq!(feed(600, Down { x: 340, y: 200 }), None);
        assert_eq!(feed(610, Move { x: 470, y: 300 }), None);
        let (_, point) = feed(700, Up).expect("still on its key");
        assert_eq!(point, (340, 200));
    }
}
