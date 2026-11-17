//! The front panel's side of the system: finding the screen, what to show on it, and what a tap does.
//!
//! [`run`] is the panel's thread. It loads the screen's drivers, finds the framebuffer, and shows the
//! panel OS on it (spec `2026-11-12-qberrypi-panel-os-design.md`): HOME and its five sections, which
//! [`Panel`] shows with the painters of `qb_panel`, and LIVE, the Drain, whose names go by on the lane
//! eight times a second and whose slow figures (the percentage, the clock, the count, the key, the
//! gate's refill and the lane's words) are written one region at a time between those frames.
//!
//! The thread reads the touch screen between draws; a tap is decided by `qb_panel::os::nav`, which says
//! what the panel does next and, for a tap that is an action, what the system must do (an `Intent`):
//! the gate, one device's ads or its own gate, a dismissed alert. A boot that asks for it first shows
//! four crosses to tap and measures the touch screen from the taps. A board without the screen is
//! [`PanelEnd::NoScreen`], which is no reason for the rest of the system to stop. The thread only logs and
//! reads `/sys` and `/dev`: it never touches the card, the watchdog or the LED, and it never logs a
//! name, a label or an address; what it logs of an action is what kind it was.
//!
//! What the screens read is copied into a `World` on the cadences of spec §5.2: the snapshot every
//! second; the control of blocking every turn; the lane's names every frame; the device inventory every
//! two seconds and who asks every second, only while HOME, DEVICES, DEVICE, CONFIRM or ITS GATE shows
//! (the first, DEVICE, also the watched device's names every frame); the waiting alerts every second.
//! The resolver's lock is taken only for the copies of the snapshot and of the control, as before.
//!
//! **Writes.** The surface takes a whole frame and writes the rows that differ from what the glass shows.
//! LIVE keeps the Drain's painter and its schedule: a frame's motion, and half a frame later one figure.
//! Every other screen is painted whole from its view into the `ScreenPainter`; a switch of screens, a
//! whole-system change and the gate's toggle are one full write, and inside a screen each band of changed
//! rows (`ScreenPainter::paint_bands`) is a write of its own, at most 80 rows, one to a slot, so that no
//! two share the driver's flush window.

use std::fs::File;
use std::io::{self, Read};
use std::net::Ipv4Addr;
use std::ops::Range;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use qb_panel::drain::{Drain, Stills};
use qb_panel::os::nav::{self, Nav, Screen as Opened, Section, Tap};
use qb_panel::os::screens::{self, View};
use qb_panel::os::world::{Outcome, Pause, WatchedName, World};
use qb_panel::os::{GateMove, Intent, Whole};
use qb_panel::rows::BAND_GAP;
use qb_panel::ui::ScreenPainter;
use qb_panel::{Calibration, HEIGHT, Painter, TapTracker, WIDTH};

use crate::lane::{LaneFeed, LaneName, WatchedName as LaneWatched, WatchedOutcome};
use crate::live::{BlockingControl, Source};
use crate::modules::{self, Insert, MODULE_LIST, ModuleTable, PANEL_MODULES};
use crate::sys;
use crate::watch::devices::WallCopy;
use alerts::PendingAlerts;
use devices::DeviceControl;
use fb::{Framebuffer, SCREEN_DRIVER, SYS_GRAPHICS, SYS_VTCONSOLE, Screen, ScreenError};
use touch::{Decoded, Decoder, RawEvent, SYS_INPUT, find_touch, read_events};
use view::{WallEntry, blocking, gate_total, refill, wanted_stills};

pub mod alerts;
pub mod calibrate;
pub mod devices;
pub mod fb;
pub mod touch;
pub mod view;

/// How long a frame of the Drain lasts, in milliseconds: eight frames a second.
///
/// The Drain moves one step each frame while anything on the lane is moving. A panel with nothing
/// moving writes nothing, and looks at the lane's feed and the control of blocking once a frame.
pub const FRAME_MS: u64 = 125;

/// How long after a frame the slow figures may be written, in milliseconds.
///
/// The screen's driver (fbtft, with deferred io) sends the span from the first to the last row
/// touched since its last flush, and flushes [`FLUSH_GUARD_MS`] or less after a write. A figure
/// written half a frame after the lane's frame goes out in a flush of its own, so that the two
/// spans are not one.
pub const STILL_OFFSET_MS: u64 = 62;

/// How often the panel asks for the system's numbers, in milliseconds.
pub const SNAPSHOT_MS: u64 = 1_000;

/// How often the panel copies the device inventory, in milliseconds, while a screen that shows devices is up.
pub const DEVICES_MS: u64 = 2_000;

/// How often the panel asks who asked something lately, in milliseconds, while HOME or DEVICES shows.
pub const ACTIVITY_MS: u64 = 1_000;

/// How often the panel asks for the waiting alerts, in milliseconds.
pub const ALERTS_MS: u64 = 1_000;

/// How many frames the LIVE tile's lamp stays white after a name went by in the lane.
pub const BEAT_FRAMES: u8 = 3;

/// The most rows one write of a band holds: the budget of a write inside a screen.
pub const BAND_ROWS_MAX: usize = 80;

/// How many bytes a row of the frame holds.
const ROW_BYTES: usize = WIDTH * 2;

/// How long the screen's driver may wait before it sends what was written, in milliseconds.
///
/// A figure is written only while at least this long is left before the next frame's write. One
/// that is written later is in the same flush window as that write, and the driver merges the two
/// into one span of rows, about 300 of them. A figure that is late waits for the next frame's slot.
pub const FLUSH_GUARD_MS: u64 = 50;

// A figure's slot falls inside its frame, outside the flush window of the frame's own write and
// of the next one's.
const _: () =
    assert!(STILL_OFFSET_MS > FLUSH_GUARD_MS && STILL_OFFSET_MS + FLUSH_GUARD_MS < FRAME_MS);

/// What the funnel's first dots are made from; any number gives a funnel of its own.
const DRAIN_SEED: u32 = 7;

/// How long the screen's framebuffer is waited for once its drivers are loaded.
///
/// The framebuffer appears a little after the module that makes it is inserted.
pub const SCREEN_WAIT: Duration = Duration::from_secs(2);

/// How often a wait for the screen, or for the touch screen, looks whether it is there.
const SCREEN_POLL: Duration = Duration::from_millis(50);

/// Where the kernel makes the device files of the framebuffers.
const DEV: &str = "/dev";

/// Where the kernel makes the device files of the input devices.
const DEV_INPUT: &str = "/dev/input";

/// Where a frame goes to be seen.
pub trait Surface {
    /// Replaces the picture with `frame`, which holds the panel as RGB565.
    fn show(&mut self, frame: &[u8]) -> io::Result<()>;
}

impl Surface for Framebuffer {
    fn show(&mut self, frame: &[u8]) -> io::Result<()> {
        Framebuffer::show(self, frame)
    }
}

/// What the panel is given to work with besides the screen.
pub struct Deps {
    /// Where what the panel shows comes from.
    pub source: Arc<dyn Source>,
    /// What the switch on the panel controls.
    pub control: Arc<dyn BlockingControl>,
    /// How the touch screen's readings lie on the screen.
    pub calibration: Calibration,
    /// Whether to measure `calibration` before the panel is drawn: four crosses to tap, logged and used for this boot only.
    pub calibrate: bool,
    /// The names that the resolver lets the lane show.
    pub lane: Arc<LaneFeed>,
    /// Where the panel's alerts come from, and where a dismissal is told: the ALERT screen shows the top one.
    pub alerts: Arc<dyn PendingAlerts>,
    /// The devices: what the wall and DEVICE read of them, and the edits a tap makes to them.
    pub devices: Arc<dyn DeviceControl>,
}

/// How the panel's thread ended; it ends only when the panel cannot go on.
#[derive(Debug, PartialEq)]
pub enum PanelEnd {
    /// There is no usable screen; says why. The rest of the system runs without a panel.
    NoScreen(String),
    /// The panel was running and broke; says how.
    Failed(String),
}

/// The places the panel looks in, so that tests can look elsewhere.
pub struct Roots {
    /// The table of the kernel modules that can be loaded.
    pub module_list: PathBuf,
    /// The directory in which the kernel lists its framebuffers.
    pub sys_graphics: PathBuf,
    /// The directory in which the kernel lists its text consoles.
    pub sys_vtconsole: PathBuf,
    /// The directory of the device files.
    pub dev: PathBuf,
}

impl Roots {
    /// The places on a running system.
    pub fn real() -> Self {
        Self {
            module_list: MODULE_LIST.into(),
            sys_graphics: SYS_GRAPHICS.into(),
            sys_vtconsole: SYS_VTCONSOLE.into(),
            dev: DEV.into(),
        }
    }
}

/// Loads the drivers of the screen and the touch controller, and finds the screen.
///
/// The framebuffer is looked for at least once, and then every 50 ms until it
/// is there or `wait` has passed. When the screen is the only framebuffer the
/// kernel has put its text console on it, so the console is taken off.
pub fn bring_up(roots: &Roots, insert: Insert, wait: Duration) -> Result<Screen, PanelEnd> {
    let table = std::fs::read_to_string(&roots.module_list)
        .map_err(|error| error.to_string())
        .and_then(|text| ModuleTable::parse(&text))
        .map_err(|error| PanelEnd::NoScreen(format!("{}: {error}", roots.module_list.display())))?;
    let mut inserted = Vec::new();
    for name in PANEL_MODULES {
        let loaded = modules::load(&table, name, "", &mut *insert)
            .map_err(|error| PanelEnd::NoScreen(format!("loading {name}: {error}")))?;
        inserted.extend(loaded);
    }
    log::info!("panel: modules: inserted {}", inserted.join(", "));

    let asked = Instant::now();
    let found = loop {
        match fb::find_screen(&roots.sys_graphics, &roots.dev) {
            Err(ScreenError::Absent) if asked.elapsed() < wait => {
                thread::sleep(SCREEN_POLL.min(wait.saturating_sub(asked.elapsed())));
            }
            other => break other,
        }
    };
    let screen = found.map_err(|error| {
        PanelEnd::NoScreen(match error {
            ScreenError::Absent => format!("no {SCREEN_DRIVER} framebuffer"),
            ScreenError::Wrong(found) => {
                format!("{SCREEN_DRIVER} is {found}, not {WIDTH},{HEIGHT} at 16 bpp")
            }
        })
    })?;
    if screen.sole {
        match fb::detach_console(&roots.sys_vtconsole) {
            Ok(true) => log::info!("panel: the text console was on the screen; detached"),
            Ok(false) => {}
            // The panel is drawn anyway; the console's lines will scroll over it.
            Err(error) => log::warn!("panel: taking the text console off the screen: {error}"),
        }
    }
    log::info!(
        "panel: {SCREEN_DRIVER} is {}, {WIDTH} x {HEIGHT}, 16 bpp",
        screen.device.display()
    );
    Ok(screen)
}

/// Where the panel is, as far as what is on the glass goes: a change of it is a switch of screens, which
/// is one full write. The gate ask over LIVE is no place of its own: the Drain draws it.
#[derive(Clone, PartialEq, Eq)]
struct Place {
    root: Section,
    stack: Vec<Opened>,
    whole: Option<Whole>,
    page: usize,
}

impl Place {
    fn of(nav: &Nav) -> Self {
        Self {
            root: nav.root(),
            stack: nav.stack().to_vec(),
            whole: nav.whole(),
            page: nav.devices_page(),
        }
    }
}

/// Whether LIVE, the Drain, is what shows: the root, with nothing opened over it.
fn is_live(nav: &Nav) -> bool {
    nav.root() == Section::Live && nav.stack().is_empty() && nav.whole().is_none()
}

/// What the screen that shows needs read (spec §5.2): the device inventory while HOME, DEVICES, DEVICE,
/// CONFIRM or ITS GATE shows; who asked lately only while HOME or DEVICES shows; the watched device's
/// names only while DEVICE shows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Reads {
    devices: bool,
    activity: bool,
    watched: bool,
}

impl Reads {
    fn of(nav: &Nav) -> Self {
        if nav.whole().is_some() {
            return Self::default();
        }
        match nav.top() {
            None if matches!(nav.root(), Section::Home | Section::Devices) => Self {
                devices: true,
                activity: true,
                watched: false,
            },
            Some(Opened::Device(_)) => Self {
                devices: true,
                activity: false,
                watched: true,
            },
            Some(Opened::Confirm { .. } | Opened::ItsGate(_)) => Self {
                devices: true,
                activity: false,
                watched: false,
            },
            _ => Self::default(),
        }
    }
}

/// Puts `value` in `slot` and notes it in `changed` when it is not what the slot holds.
fn put<T: PartialEq>(slot: &mut T, value: T, changed: &mut bool) {
    if *slot != value {
        *slot = value;
        *changed = true;
    }
}

/// The panel on its screen: what to draw and when, and what a tap does.
///
/// The `World` is what the OS screens read and `Nav` is where the panel is. LIVE is the Drain: names on a
/// lane, refused ones melting into a funnel, and the `Stills`, the figures that change slowly. What the
/// screen shows of them is `shown`, and what it is to come to show is `wanted`; the panel copies one
/// field at a time from `wanted` to `shown`, each in a write of its own, half a frame after the lane's
/// frame. Every other screen is a `View` of the world, painted whole by the `ScreenPainter`; `glass` is
/// what the glass shows when one of those is up, so that a band of it is written alone.
pub struct Panel<S: Surface> {
    surface: S,
    deps: Deps,
    /// Turns the touch screen's events into touches on the screen.
    decoder: Decoder,
    /// Turns touches into taps; its target is what `nav::decide` says a press would do.
    tracker: TapTracker<Tap>,
    /// Where the panel is.
    nav: Nav,
    /// What the screens read.
    world: World,
    /// The inventory as the panel copies it for the wall: its own buffer, which `Devices::wall_copy` fills every time
    /// it is read, so that the read allocates nothing under the inventory's lock.
    inventory: WallCopy,
    /// The wall as the inventory gave it last, with each place's addresses: what a device is looked up by.
    wall: Vec<WallEntry>,
    /// The addresses that asked in the last second or two, as the last read said.
    asking: Vec<Ipv4Addr>,
    /// The names of the watched device, taken from the lane feed. Kept between frames so that it is not made again.
    watched_buf: Vec<LaneWatched>,
    /// The address whose names the lane feed keeps for DEVICE; `None` while DEVICE does not show, or shows a
    /// device with no address yet.
    watching: Option<Ipv4Addr>,
    /// The scene.
    drain: Drain,
    /// Draws the scene and the figures on a frame it keeps.
    painter: Painter,
    /// The figures that the screen shows.
    shown: Stills,
    /// The figures that the screen is to show, as of the last turn.
    wanted: Stills,
    /// The names taken from the lane's feed at a frame. Kept between frames so that it is not made again.
    arrivals: Vec<LaneName>,
    /// Paints the screens that are not LIVE, whole, from a view.
    screens: ScreenPainter<View>,
    /// The frame the glass shows while a screen of `screens` is up: what is written band by band.
    glass: Vec<u8>,
    /// Bands of `screens`' frame that are not on the glass yet, one is written to a slot.
    pending: Vec<Range<usize>>,
    /// Where the panel was when the glass was last written whole; `None` before the first frame.
    drawn: Option<Place>,
    /// Whether the world changed since a screen of `screens` last looked at it.
    world_changed: bool,
    /// Whether the first frame has been drawn.
    begun: bool,
    /// The time, in milliseconds since the panel began, at which the next frame is due.
    next_frame_ms: u64,
    /// The time at which the last frame was made.
    last_frame_ms: u64,
    /// The time, in milliseconds since the panel began, at which the panel last took a name from the
    /// lane's feed; the first frame's time until it takes one. It times the lane's `QUIET` words.
    last_name_ms: u64,
    /// Whether the figures' write of the frame that was last made has been done.
    still_done: bool,
    /// The time from which the next snapshot of the system is due.
    next_snapshot_ms: u64,
    /// The seconds of a pause that the control last said were left; `None` when it said blocking is on.
    left: Option<u16>,
    /// The total of the gate that is open: what the panel opened it for, or what a pause that came from
    /// elsewhere is taken to be (`view::gate_total`); `None` while blocking is on.
    gate_total: Option<u16>,
    /// Whether a tap asked for a draw that has not been made.
    tapped: bool,
    /// Whether the touch screen was touched since the last turn.
    touched: bool,
    /// The time at which the touch screen was last touched.
    last_touch_ms: u64,
    /// How long without a touch takes the panel HOME, in milliseconds.
    idle_after_ms: u64,
    /// The clock, in Unix seconds, as the last turn had it.
    unix: u64,
    /// The local time's distance from UTC, in seconds, as the last snapshot said.
    utc_offset_secs: i32,
    /// Whether the clock was set, as the last snapshot said.
    clock_set: bool,
    /// How many frames the LIVE tile's lamp is still to stay white.
    beat_left: u8,
    /// What the last frame needed read.
    reads: Reads,
    /// The times from which the next reads are due.
    next_devices_ms: u64,
    next_activity_ms: u64,
    next_alerts_ms: u64,
    /// Whether an edit asked for the inventory, or a dismissal for the alerts, to be read at once.
    force_devices: bool,
    force_alerts: bool,
}

impl<S: Surface> Panel<S> {
    /// A panel that draws on `surface`, at HOME.
    pub fn new(surface: S, deps: Deps) -> Self {
        // A panel that failed, or was restarted, may have left the lane feed keeping the names of one
        // device (spec §6.2: names only while that device's screen is open). This one has no screen open.
        deps.devices.watch_client(None);
        Self {
            surface,
            deps,
            decoder: Decoder::default(),
            tracker: TapTracker::default(),
            nav: Nav::new(),
            world: World {
                version: qb_panel::format::fit_text(
                    crate::VERSION,
                    qb_panel::os::world::VERSION_CELLS,
                ),
                ..World::default()
            },
            inventory: WallCopy::new(),
            wall: Vec::new(),
            asking: Vec::new(),
            watched_buf: Vec::new(),
            watching: None,
            drain: Drain::new(DRAIN_SEED),
            painter: Painter::new(),
            shown: Stills::default(),
            wanted: Stills::default(),
            arrivals: Vec::new(),
            screens: ScreenPainter::new(),
            glass: vec![0; ROW_BYTES * HEIGHT],
            pending: Vec::new(),
            drawn: None,
            world_changed: false,
            begun: false,
            next_frame_ms: 0,
            last_frame_ms: 0,
            last_name_ms: 0,
            still_done: false,
            next_snapshot_ms: 0,
            left: None,
            gate_total: None,
            tapped: false,
            touched: false,
            last_touch_ms: 0,
            idle_after_ms: nav::IDLE_MS,
            unix: 0,
            utc_offset_secs: 0,
            clock_set: false,
            beat_left: 0,
            reads: Reads::default(),
            next_devices_ms: 0,
            next_activity_ms: 0,
            next_alerts_ms: 0,
            force_devices: false,
            force_alerts: false,
        }
    }

    /// Takes the events the touch screen reported, in the order they were read.
    ///
    /// A press that begins on something a tap works, stays on it and lasts 50 ms is a tap; it is decided by
    /// `nav::tap`, which moves the panel and may give back what the system must do, which is done here.
    /// The next turn draws whatever it changed. A tap is timed by the events that make it, not by when
    /// they were read.
    pub fn touch(&mut self, events: &[RawEvent]) {
        for &event in events {
            let Some((at_ms, decoded)) = self.decoder.feed(event, &self.deps.calibration) else {
                continue;
            };
            match decoded {
                Decoded::Touch(touch) => {
                    self.touched = true;
                    let (nav, world) = (&self.nav, &self.world);
                    let tapped = self.tracker.feed_with(at_ms, touch, |x, y| {
                        match nav::decide(nav, world, x, y) {
                            Tap::None => None,
                            tap => Some(tap),
                        }
                    });
                    if let Some((_, (x, y))) = tapped {
                        self.on_tap(x, y);
                    }
                }
                // Whatever was known of the press is not to be trusted, nor is it needed:
                // the press may be one whose `Down` was never given out.
                Decoded::Lost => self.tracker = TapTracker::default(),
            }
        }
    }

    /// Does a tap at `(x, y)`: the panel moves, and the system does what the tap asked of it.
    fn on_tap(&mut self, x: u16, y: u16) {
        if let Some(intent) = nav::tap(&mut self.nav, &self.world, x, y) {
            self.carry_out(intent);
        }
        self.tapped = true;
        self.sync_watch();
    }

    /// Does what an intent asks of the system, and logs which kind it was: never whom it was about. P1's
    /// strong actions run directly, as there is no lock (ruling R3).
    fn carry_out(&mut self, intent: Intent) {
        match intent {
            Intent::Gate(GateMove::Open(secs)) => {
                self.deps.control.pause_for(secs);
                self.gate_total = Some(secs);
                log::info!("panel: gate opened for {secs} s");
            }
            Intent::Gate(GateMove::Shut) => {
                self.deps.control.resume();
                self.gate_total = None;
                log::info!("panel: gate shut");
            }
            Intent::DeviceAds { mac, pass } => {
                match self.deps.devices.set_ads_pass(mac, pass) {
                    Ok(()) if pass => log::info!("panel: device ads pass set"),
                    Ok(()) => log::info!("panel: device ads blocked"),
                    Err(error) => {
                        log::warn!("panel: device ads edit refused: {}", error.as_str());
                    }
                }
                self.force_devices = true;
            }
            Intent::DeviceGate { mac, secs } => {
                match self.deps.devices.device_gate(mac, secs) {
                    Ok(()) if secs == 0 => log::info!("panel: device gate shut"),
                    Ok(()) => log::info!("panel: device gate opened for {secs} s"),
                    Err(error) => {
                        log::warn!("panel: device gate edit refused: {}", error.as_str());
                    }
                }
                self.force_devices = true;
            }
            Intent::Ack(id) => {
                self.deps.alerts.ack(id);
                log::info!("panel: alert dismissed");
                self.force_alerts = true;
            }
            // The names, the night hours, the lock, the rejoin and the restart are the later phases'.
            Intent::Name { .. }
            | Intent::NightHours { .. }
            | Intent::SleepNow
            | Intent::Rejoin
            | Intent::Restart
            | Intent::SetPin(_)
            | Intent::ClearPin => {
                log::warn!("panel: an intent of a later phase was not carried out")
            }
        }
    }

    /// Does what is due at `now_ms`, and returns how many milliseconds may pass before the next call.
    ///
    /// `now_ms` is the time since the panel began and `now_unix` is the clock, in Unix seconds.
    /// A frame is due every `FRAME_MS`: it takes the names the feed holds, asks the system for its numbers
    /// and the screen's reads when they are due, and, while anything is moving, advances the Drain one step
    /// and, when LIVE shows, draws it; any other screen draws what changed in the world, one band of rows.
    /// Half a frame later, if the figures on the screen are not those that are wanted (on LIVE), or a band
    /// is still to be written (elsewhere), one is drawn: a write to a slot, and only until `FLUSH_GUARD_MS`
    /// before the next frame, since a write later would share its flush with that frame's; one that is
    /// late waits for the next frame's slot. A tap draws at once, and so does a switch of screens, which
    /// is one full write, as is the gate's toggle. The first turn draws everything. A panel with nothing
    /// to move and nothing to change draws nothing, whatever the time. A turn that comes late makes one
    /// frame, and the frames are counted from it. A minute without a touch takes the panel HOME.
    pub fn turn(&mut self, now_ms: u64, now_unix: u64) -> io::Result<u64> {
        self.unix = now_unix;
        if std::mem::take(&mut self.touched) {
            self.last_touch_ms = now_ms;
        }
        let tapped = std::mem::take(&mut self.tapped);

        // The gate: the control is asked at every turn. The total of a pause is what the panel opened it
        // for, or taken from what is left of one that came from elsewhere.
        let left = self.deps.control.paused_left_secs();
        if left.is_none() && self.left.is_some() && !tapped {
            log::info!("panel: gate: the pause ended");
        }
        let toggled = left.is_some() != self.left.is_some();
        self.left = left;
        self.gate_total = left.map(|left| gate_total(self.gate_total, left));
        self.wanted.blocking = blocking(left);
        self.wanted.gate_fill = refill(left, self.gate_total.unwrap_or(0));
        let pause = left
            .zip(self.gate_total)
            .map(|(left_secs, total_secs)| Pause {
                left_secs,
                total_secs,
            });
        put(&mut self.world.blocking, pause, &mut self.world_changed);
        // The Drain knows of a pause as soon as it changes, so that the listed names on the lane are
        // restyled in the key's own write, which is a large one by right, and not in a motion write
        // that may share a step of the funnel.
        if toggled {
            self.drain.set_paused(left.is_some());
        }

        // A minute without a touch takes the panel HOME.
        let quiet_ms = now_ms.saturating_sub(self.last_touch_ms);
        if self.begun && quiet_ms >= self.idle_after_ms {
            nav::idle(&mut self.nav, quiet_ms);
        }
        self.sync_watch();
        self.sync_ask();

        // A switch of screens is one full write, now.
        let mut wrote = false;
        if self.begun && self.drawn.as_ref() != Some(&Place::of(&self.nav)) {
            self.arrive(now_ms)?;
            wrote = true;
        }
        let live = is_live(&self.nav);
        if !live && toggled && self.begun && !wrote {
            // The gate's toggle changes HOME's lane and tile at once: one full write.
            self.present_toggle()?;
            wrote = true;
        }
        let early =
            live && toggled && tapped && self.begun && !wrote && now_ms >= self.next_frame_ms;
        if early {
            // A tap that comes in the turn of a frame: the key's write goes ahead of the frame's.
            self.still(true)?;
        }
        if !self.begun || now_ms >= self.next_frame_ms {
            self.frame(now_ms, now_unix)?;
        }
        if is_live(&self.nav) {
            if (tapped && !early && !wrote) || self.still_is_due(now_ms) {
                self.still(tapped)?;
            }
        } else if self.still_is_due(now_ms) {
            self.still_done = true;
            self.draw_screen()?;
        }
        let mut next_ms = self.next_frame_ms;
        if !self.still_done && self.still_window_open(now_ms) && self.has_still() {
            next_ms = next_ms.min(self.last_frame_ms.saturating_add(STILL_OFFSET_MS));
        }
        Ok(next_ms.saturating_sub(now_ms))
    }

    /// Whether a figure may still be written in this frame: `FLUSH_GUARD_MS` or more is left before the next frame.
    fn still_window_open(&self, now_ms: u64) -> bool {
        now_ms < self.last_frame_ms.saturating_add(FRAME_MS - FLUSH_GUARD_MS)
    }

    /// Whether there is a write to make in the frame's second slot: on LIVE a figure that is not as wanted,
    /// elsewhere a change of the world, or a band that is not written yet.
    fn has_still(&self) -> bool {
        if is_live(&self.nav) {
            self.shown != self.wanted
        } else {
            self.world_changed || !self.pending.is_empty()
        }
    }

    /// Whether a write is to be made now: this frame's second slot is not used, there is one to make, half a frame has passed, and it is not late in the frame.
    fn still_is_due(&self, now_ms: u64) -> bool {
        !self.still_done
            && now_ms >= self.last_frame_ms.saturating_add(STILL_OFFSET_MS)
            && self.still_window_open(now_ms)
            && self.has_still()
    }

    /// Makes a frame: takes the names, asks for the numbers and the screen's reads when they are due, moves
    /// the Drain, and draws what the screen that shows needs.
    fn frame(&mut self, now_ms: u64, now_unix: u64) -> io::Result<()> {
        let first = !self.begun;
        self.begun = true;
        if first {
            // The lane's quiet is counted from the panel's first frame, not from the thread's start; the
            // minute without a touch too.
            self.last_name_ms = now_ms;
            self.last_touch_ms = now_ms;
        }
        let elapsed_ms = now_ms.saturating_sub(self.last_frame_ms);
        self.last_frame_ms = now_ms;
        self.still_done = false;
        // The frames keep their beat, unless the thread was held up for a frame or more.
        self.next_frame_ms = match self.next_frame_ms.saturating_add(FRAME_MS) {
            next if next > now_ms => next,
            _ => now_ms.saturating_add(FRAME_MS),
        };

        // The pause is known first, so that a refused name that arrives during one is shown as listed.
        let paused = self.left.is_some();
        self.drain.set_paused(paused);
        self.arrivals.clear();
        self.deps.lane.take(&mut self.arrivals);
        if !self.arrivals.is_empty() {
            self.last_name_ms = now_ms;
            self.beat_left = BEAT_FRAMES;
        }
        for arrival in self.arrivals.drain(..) {
            self.drain.offer(&arrival.name, arrival.kind);
        }
        // The LIVE tile's lamp is white for a few frames after a name.
        let beat = self.beat_left > 0;
        self.beat_left = self.beat_left.saturating_sub(1);
        put(&mut self.world.beat, beat, &mut self.world_changed);

        if first || now_ms >= self.next_snapshot_ms {
            self.refresh(now_ms, now_unix);
            self.next_snapshot_ms = now_ms.saturating_add(SNAPSHOT_MS);
        }
        self.read_cadences(now_ms);
        self.tick_gates(now_ms);
        let days = u32::try_from(now_ms / 86_400_000).unwrap_or(u32::MAX);
        put(
            &mut self.world.uptime_days,
            Some(days),
            &mut self.world_changed,
        );
        if nav::refresh(&mut self.nav, &self.world) {
            self.sync_watch();
        }

        let moving = self.drain.moving();
        if moving {
            self.drain.tick(paused);
        }
        // The count is the Drain's: it holds back the refused names that have not melted yet. It is a
        // dash while the Wi-Fi is not joined: `refresh` gave the Drain no figure then, and a Drain
        // with no figure has no count, whatever names it carries. HOME's tile shows the same figure.
        self.wanted.count = self.drain.count();
        put(
            &mut self.world.refused_today,
            self.wanted.count,
            &mut self.world_changed,
        );
        if is_live(&self.nav) {
            nav::age_ask(&mut self.nav, elapsed_ms);
        }
        self.sync_ask();
        if first || self.drawn.as_ref() != Some(&Place::of(&self.nav)) {
            // The screen has nothing yet, or a pop moved the panel: it gets everything known at once.
            return self.present_full();
        }
        if is_live(&self.nav) {
            if moving { self.draw() } else { Ok(()) }
        } else {
            self.draw_screen()
        }
    }

    /// Asks the system for its numbers and works out the figures that are wanted, and what of them the
    /// world holds.
    ///
    /// `now_ms` times how long the lane has been without a name, for its `QUIET` words.
    fn refresh(&mut self, now_ms: u64, now_unix: u64) {
        let snapshot = self.deps.source.snapshot();
        let idle_secs = now_ms.saturating_sub(self.last_name_ms) / 1_000;
        self.wanted = wanted_stills(
            &snapshot,
            self.left,
            self.gate_total.unwrap_or(0),
            now_unix,
            idle_secs,
        );
        self.drain.set_blocked_today(self.wanted.count);
        self.sync_ask();

        self.utc_offset_secs = snapshot.utc_offset_secs;
        self.clock_set = snapshot.clock_set;
        let joined = view::joined(&snapshot);
        let today = snapshot.today.filter(|_| joined);
        let time = snapshot
            .clock_set
            .then(|| view::time_of_day(now_unix, snapshot.utc_offset_secs));
        let names = snapshot
            .list_names
            .map(|names| view::held(u64::try_from(names).unwrap_or(u64::MAX)));
        let changed = &mut self.world_changed;
        put(&mut self.world.time, time, changed);
        put(&mut self.world.wifi_joined, joined, changed);
        put(
            &mut self.world.queries_today,
            today.map(|today| view::held(today.queries)),
            changed,
        );
        put(&mut self.world.share, self.wanted.percent, changed);
        put(&mut self.world.list_names, names, changed);
    }

    /// Does the reads the screen that shows needs, when they are due (spec §5.2): the inventory every
    /// two seconds, who asked every second, the watched device's names every frame, and the waiting
    /// alerts every second. A read whose screen has just come up is made at once.
    fn read_cadences(&mut self, now_ms: u64) {
        let reads = Reads::of(&self.nav);
        if reads.activity && (!self.reads.activity || now_ms >= self.next_activity_ms) {
            self.deps
                .devices
                .asking(view::ASKING_SECS, &mut self.asking);
            self.next_activity_ms = now_ms.saturating_add(ACTIVITY_MS);
            for entry in &mut self.wall {
                entry.row.asking = view::is_asking(&entry.ipv4, &self.asking);
            }
            self.publish_wall();
        }
        if reads.devices
            && (!self.reads.devices || self.force_devices || now_ms >= self.next_devices_ms)
        {
            let unix = self.clock_set.then_some(self.unix);
            // The copy is made under the inventory's lock and is short; the wall's own work (its order,
            // its names, their fitting) is done here, after the lock is let go.
            if self
                .deps
                .devices
                .wall(&|last| view::recent(last, unix), &mut self.inventory)
            {
                let mut wall = view::wall(&self.inventory, &self.asking, now_ms, unix);
                view::keep_countdowns(&self.wall, &mut wall, now_ms);
                self.wall = wall;
                put(
                    &mut self.world.newcomer,
                    view::newcomer_of(&self.inventory),
                    &mut self.world_changed,
                );
            }
            self.force_devices = false;
            self.next_devices_ms = now_ms.saturating_add(DEVICES_MS);
            self.publish_wall();
            self.sync_watch();
        }
        if reads.watched {
            self.deps.devices.watched_names(&mut self.watched_buf);
            let names = self
                .watched_buf
                .iter()
                .map(|name| WatchedName::new(&name.name, outcome_of(name.outcome)))
                .collect();
            put(&mut self.world.watched, names, &mut self.world_changed);
        }
        if self.force_alerts || now_ms >= self.next_alerts_ms {
            self.read_alerts();
            self.force_alerts = false;
            self.next_alerts_ms = now_ms.saturating_add(ALERTS_MS);
        }
        self.reads = reads;
    }

    /// Reads the waiting alerts: how many, when the one that ALERT opens last happened, and that one, with its
    /// lines made from what the store carries and the names of the wall.
    fn read_alerts(&mut self) {
        let top = self.deps.alerts.top();
        let waiting = match &top {
            Some(_) => self.deps.alerts.waiting_count().max(1),
            None => 0,
        };
        let newest = top
            .as_ref()
            .and_then(|alert| view::today_clock(alert.last_unix, self.unix, self.utc_offset_secs));
        let row = top.as_ref().map(|alert| {
            view::alert_row(alert, &self.wall, self.world.newcomer, self.utc_offset_secs)
        });
        let changed = &mut self.world_changed;
        put(&mut self.world.alerts_waiting, waiting, changed);
        put(&mut self.world.alert_newest, newest, changed);
        put(&mut self.world.top_alert, row, changed);
    }

    /// Makes the world's wall what the panel's copy of it is, if it is not.
    fn publish_wall(&mut self) {
        let same = self.world.devices.len() == self.wall.len()
            && self
                .world
                .devices
                .iter()
                .zip(&self.wall)
                .all(|(row, entry)| *row == entry.row);
        if !same {
            self.world.devices = self.wall.iter().map(|entry| entry.row.clone()).collect();
            self.world_changed = true;
        }
    }

    /// Steps the open gates' countdowns to `now_ms`: the inventory is read every two seconds, and a
    /// countdown is to step every second.
    fn tick_gates(&mut self, now_ms: u64) {
        let mut stepped = false;
        for entry in &mut self.wall {
            let before = entry.row.state;
            entry.tick(now_ms);
            stepped |= entry.row.state != before;
        }
        if stepped {
            self.publish_wall();
        }
    }

    /// Keeps the lane feed's watch on the device whose screen shows, and on nothing else: the watch opens
    /// when DEVICE does, with the device's newest address, and is cleared as soon as DEVICE is left, by a
    /// tap, the idle minute or the device going. What was kept of its names goes with it. A screen over
    /// DEVICE (CONFIRM, ITS GATE) is no DEVICE: the watch is let go while it shows.
    fn sync_watch(&mut self) {
        let shown = self.nav.watched_mac();
        let address = shown.and_then(|mac| {
            self.wall
                .iter()
                .find(|entry| entry.row.mac == mac)
                .and_then(|entry| entry.ipv4.first().copied())
        });
        if address != self.watching {
            self.deps.devices.watch_client(address);
            self.watching = address;
        }
        if shown.is_none() {
            self.watched_buf.clear();
            put(&mut self.world.watched, Vec::new(), &mut self.world_changed);
        }
    }

    /// The gate ask over LIVE is the nav's; the Drain is to be drawn with it.
    fn sync_ask(&mut self) {
        self.wanted.gate_ask = self.nav.gate_ask();
    }

    /// Moves to a screen that is not the one on the glass: reads what it needs, lets the nav pop what is
    /// gone, and writes it whole.
    fn arrive(&mut self, now_ms: u64) -> io::Result<()> {
        self.read_cadences(now_ms);
        if nav::refresh(&mut self.nav, &self.world) {
            self.sync_watch();
            self.read_cadences(now_ms);
        }
        self.sync_ask();
        self.present_full()
    }

    /// Writes the screen that shows, whole: the surface writes the rows that differ from the glass. A
    /// screen switch, the gate's toggle and the first frame are the only callers.
    fn present_full(&mut self) -> io::Result<()> {
        self.pending.clear();
        self.world_changed = false;
        self.drawn = Some(Place::of(&self.nav));
        if is_live(&self.nav) {
            // The Drain was run all the while: it paints what changed since it was last painted.
            self.shown.clone_from(&self.wanted);
            let frame = self.painter.paint(&self.drain, &self.shown);
            return self.surface.show(frame);
        }
        self.screens.invalidate();
        let Some(view) = screens::view_of(&self.nav, &self.world) else {
            return Ok(());
        };
        self.screens.paint(view, screens::draw);
        self.glass.copy_from_slice(self.screens.frame());
        self.surface.show(&self.glass)
    }

    /// Writes what the gate's toggle changed on the screen that shows as one write, the span of rows that
    /// differ from the glass, and with it every band that was waiting; nothing when it changed nothing (the
    /// wall of devices does not show the gate). Not LIVE: the Drain's toggle is its own.
    fn present_toggle(&mut self) -> io::Result<()> {
        self.world_changed = false;
        let Some(view) = screens::view_of(&self.nav, &self.world) else {
            return Ok(());
        };
        let changed = self.screens.paint(view, screens::draw).is_some();
        if !changed && self.pending.is_empty() {
            return Ok(());
        }
        self.pending.clear();
        self.glass.copy_from_slice(self.screens.frame());
        self.surface.show(&self.glass)
    }

    /// Paints what changed in the world on the screen that shows, and writes one band of it.
    fn draw_screen(&mut self) -> io::Result<()> {
        if std::mem::take(&mut self.world_changed)
            && let Some(view) = screens::view_of(&self.nav, &self.world)
        {
            let bands = self.screens.paint_bands(view, screens::draw);
            self.queue(bands);
        }
        self.write_band()
    }

    /// Adds bands of rows to those that are to be written, joining those that are near.
    fn queue(&mut self, bands: Vec<Range<usize>>) {
        self.pending.extend(bands);
        self.pending.sort_by_key(|band| band.start);
        let mut joined: Vec<Range<usize>> = Vec::with_capacity(self.pending.len());
        for band in self.pending.drain(..) {
            match joined.last_mut() {
                Some(last) if band.start <= last.end + BAND_GAP => {
                    last.end = last.end.max(band.end)
                }
                _ => joined.push(band),
            }
        }
        self.pending = joined;
    }

    /// Writes the first of the bands that wait, at most `BAND_ROWS_MAX` rows of it: rows of the screen's
    /// frame put on the glass's, as a write of their own.
    fn write_band(&mut self) -> io::Result<()> {
        let Some(first) = self.pending.first().cloned() else {
            return Ok(());
        };
        let end = first.end.min(first.start + BAND_ROWS_MAX);
        if end >= first.end {
            self.pending.remove(0);
        } else if let Some(band) = self.pending.first_mut() {
            band.start = end;
        }
        let rows = first.start * ROW_BYTES..end * ROW_BYTES;
        self.glass[rows.clone()].copy_from_slice(&self.screens.frame()[rows]);
        self.surface.show(&self.glass)
    }

    /// Draws one figure that is not as wanted, or, when a tap asked for a draw, whatever there is. LIVE only.
    fn still(&mut self, urgent: bool) -> io::Result<()> {
        self.still_done = true;
        if urgent {
            // The ask folds, or unfolds, in the write of the tap that did it.
            self.shown.gate_ask = self.wanted.gate_ask;
        }
        let stepped = self.shown.step_toward(&self.wanted);
        if stepped || urgent {
            self.draw()?;
        }
        Ok(())
    }

    /// Paints the scene with the figures that are shown, and writes the frame if it differs from the one on the screen. LIVE only.
    fn draw(&mut self) -> io::Result<()> {
        match self.painter.paint_changed(&self.drain, &self.shown) {
            Some(frame) => self.surface.show(frame),
            None => Ok(()),
        }
    }
}

/// What a name of the watched device came to, as its screen says it.
fn outcome_of(outcome: WatchedOutcome) -> Outcome {
    match outcome {
        WatchedOutcome::Refused => Outcome::Refused,
        WatchedOutcome::Passed => Outcome::Passed,
        WatchedOutcome::Answered => Outcome::Answered,
    }
}

/// Finds the touch screen's device file, waiting up to `wait` for its driver to make it.
///
/// The device is looked for at least once, and then every 50 ms until it is there or `wait` has passed.
fn await_touch(sys_input: &Path, dev_input: &Path, wait: Duration) -> Option<PathBuf> {
    let asked = Instant::now();
    loop {
        match find_touch(sys_input, dev_input) {
            None if asked.elapsed() < wait => {
                thread::sleep(SCREEN_POLL.min(wait.saturating_sub(asked.elapsed())));
            }
            found => return found,
        }
    }
}

/// Measures the touch screen if the boot asks for it, and goes on with what was measured.
///
/// Without a touch device there is nothing to tap with, and nothing is asked
/// of the screen. A calibration that is not made, because a cross was not
/// tapped or the taps do not fit the crosses, leaves the configured one in
/// place; the result is logged and written nowhere. Fails only when the screen
/// or the device does.
fn measure_touch<S: Surface, D: Read + AsFd>(
    panel: &mut Panel<S>,
    device: Option<&mut D>,
) -> Result<(), PanelEnd> {
    let Some(device) = device.filter(|_| panel.deps.calibrate) else {
        return Ok(());
    };
    let began = Instant::now();
    let (calibration, decoder) = (panel.deps.calibration, &mut panel.decoder);
    let measured = calibrate::calibrate(
        &mut panel.surface,
        &mut |wait_ms| {
            let events = read_events(&mut *device, Duration::from_millis(wait_ms))?;
            // The decoder is told what the calibration reads too, though nothing it says is taken:
            // the kernel sends an axis only when its value changes, so the panel's first tap can
            // depend on a reading that only a tap of the calibration carried. A tap of the
            // calibration is no tap on the switch, so the tracker is not told.
            for &event in &events {
                decoder.feed(event, &calibration);
            }
            Ok(events)
        },
        &|| u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX),
    )
    .map_err(|error| PanelEnd::Failed(format!("calibrating the touch screen: {error}")))?;
    if let Some(calibration) = measured {
        panel.deps.calibration = calibration;
    }
    Ok(())
}

/// The clock in Unix seconds; 0 if it is before 1970.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Draws the panel and works its key for as long as the screen and the touch device allow.
///
/// Each round gives the panel what the touch device said, lets it draw if a
/// draw is due, and then waits for the touch device for as long as the panel
/// can go without a turn: never longer than a frame. Without a device it only
/// waits. Returns only when the screen or the device breaks.
fn serve<S: Surface, D: Read + AsFd>(
    panel: &mut Panel<S>,
    device: Option<D>,
    began: Instant,
) -> PanelEnd {
    let end = serve_until_broken(panel, device, began);
    // However the loop ended, no screen of this panel is open any more: the lane feed stops keeping the names
    // of the device whose screen was (spec §6.2).
    panel.deps.devices.watch_client(None);
    end
}

/// [`serve`]'s loop, which ends only when the screen or the touch device breaks.
fn serve_until_broken<S: Surface, D: Read + AsFd>(
    panel: &mut Panel<S>,
    mut device: Option<D>,
    began: Instant,
) -> PanelEnd {
    let mut events = Vec::new();
    loop {
        panel.touch(&events);
        let now_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
        let wait = match panel.turn(now_ms, unix_now()) {
            Ok(wait_ms) => Duration::from_millis(wait_ms),
            Err(error) => return PanelEnd::Failed(format!("writing the screen: {error}")),
        };
        match device.as_mut() {
            Some(device) => match read_events(device, wait) {
                Ok(read) => events = read,
                Err(error) => {
                    return PanelEnd::Failed(format!("reading the touch device: {error}"));
                }
            },
            None => thread::sleep(wait),
        }
    }
}

/// The panel's thread: brings the screen up, draws on it for ever and works its key.
///
/// Returns only when the panel cannot go on, and says why. A board whose
/// touch screen is not found keeps its panel, with one warning: it is only
/// the switch that does not work.
pub fn run(deps: Deps) -> PanelEnd {
    let devices = deps.devices.clone();
    let end = run_panel(deps);
    // Any end, an early one too: the lane feed keeps the names of no device (spec §6.2).
    devices.watch_client(None);
    end
}

/// [`run`], which clears the watch of a device's names when it returns.
fn run_panel(deps: Deps) -> PanelEnd {
    let screen = match bring_up(&Roots::real(), &mut sys::insert_module, SCREEN_WAIT) {
        Ok(screen) => screen,
        Err(end) => return end,
    };
    let framebuffer = match Framebuffer::open(&screen.device) {
        Ok(framebuffer) => framebuffer,
        Err(error) => {
            return PanelEnd::Failed(format!("opening {}: {error}", screen.device.display()));
        }
    };
    let mut device = match await_touch(Path::new(SYS_INPUT), Path::new(DEV_INPUT), SCREEN_WAIT) {
        None => {
            log::warn!("panel: no ADS7846 touch device; the switch will not work");
            None
        }
        Some(path) => File::open(&path)
            .inspect_err(|error| {
                log::warn!(
                    "panel: opening {}: {error}; the switch will not work",
                    path.display()
                );
            })
            .ok(),
    };
    let mut panel = Panel::new(framebuffer, deps);
    if let Err(end) = measure_touch(&mut panel, device.as_mut()) {
        return end;
    }
    serve(&mut panel, device, Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::{self, Write};
    use std::ops::Deref;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use std::net::IpAddr;

    use qb_panel::drain::{QUEUE_MAX, TileStyle};
    use qb_panel::os::world::{DeviceState, Newcomer};
    use qb_panel::rows::changed_rows;
    use qb_panel::{Blocking, Calibration, LaneWords, Share, TimeOfDay};
    use tempfile::TempDir;

    use crate::live::{BlockingControl, Counts, LogEntry, Nothing, Snapshot, Source, WifiStatus};
    use crate::modules::DEFAULT_TABLE;
    use crate::watch::devices::{DeviceView, DevicesView, EditError, KnownList, Label};
    use crate::watch::frames::Mac;
    use alerts::{NoAlerts, PanelAlert, Severity};
    use calibrate::Reading;
    use calibrate::tests::{assert_close, kernel_press, press, raw_at};
    use devices::NoDevices;
    use fb::tests::{Graphics, HDMI, graphics};
    use touch::tests::{abs_x, abs_y, bytes_of, dropped, key, pressure, syn};
    use view::tests::{TEN_TWENTY_UTC, reference_snapshot};

    /// Every frame a `Recorder` was shown, readable from where the `Recorder` was handed over.
    #[derive(Clone, Default)]
    pub(crate) struct Frames(Arc<Mutex<Vec<Vec<u8>>>>);

    impl Frames {
        pub(crate) fn count(&self) -> usize {
            self.0.lock().unwrap().len()
        }

        pub(crate) fn last(&self) -> Vec<u8> {
            self.0.lock().unwrap().last().unwrap().clone()
        }

        /// Every frame shown so far, the first first.
        pub(crate) fn all(&self) -> Vec<Vec<u8>> {
            self.0.lock().unwrap().clone()
        }
    }

    /// A surface that keeps every frame it is shown.
    pub(crate) struct Recorder(Frames);

    impl Surface for Recorder {
        fn show(&mut self, frame: &[u8]) -> io::Result<()> {
            (self.0).0.lock().unwrap().push(frame.to_vec());
            Ok(())
        }
    }

    pub(crate) fn recorder() -> (Recorder, Frames) {
        let frames = Frames::default();
        (Recorder(frames.clone()), frames)
    }

    /// A surface that cannot be written to.
    struct Broken;

    impl Surface for Broken {
        fn show(&mut self, _frame: &[u8]) -> io::Result<()> {
            Err(io::Error::other("the screen is gone"))
        }
    }

    /// A source that knows one snapshot.
    struct Fixed(Snapshot);

    impl Source for Fixed {
        fn snapshot(&self) -> Snapshot {
            self.0.clone()
        }

        fn recent(&self, _n: usize) -> Vec<LogEntry> {
            Vec::new()
        }
    }

    /// What the panel's switch controls, as a test sees it: the calls it was given, and the time it says is left.
    #[derive(Clone, Default)]
    struct FakeControl(Arc<Mutex<FakeControlState>>);

    #[derive(Default)]
    struct FakeControlState {
        calls: Vec<String>,
        left: Option<u16>,
    }

    impl FakeControl {
        fn calls(&self) -> Vec<String> {
            self.0.lock().unwrap().calls.clone()
        }

        /// Makes the control say that `left` seconds of a pause are left.
        fn set_left(&self, left: Option<u16>) {
            self.0.lock().unwrap().left = left;
        }
    }

    impl BlockingControl for FakeControl {
        fn paused_left_secs(&self) -> Option<u16> {
            self.0.lock().unwrap().left
        }

        fn pause_for(&self, secs: u16) {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("pause {secs}"));
            state.left = Some(secs);
        }

        fn resume(&self) {
            let mut state = self.0.lock().unwrap();
            state.calls.push("resume".to_owned());
            state.left = None;
        }
    }

    /// The clock in these tests, in Unix seconds: 10:20 at UTC.
    const UNIX: u64 = TEN_TWENTY_UTC;

    /// The panel's dependencies with a control nobody looks at, the nominal calibration, a lane nobody
    /// feeds, no alerts and no devices.
    fn deps(source: Arc<dyn Source>) -> Deps {
        Deps {
            source,
            control: Arc::new(FakeControl::default()),
            calibration: Calibration::NOMINAL,
            calibrate: false,
            lane: LaneFeed::new(),
            alerts: Arc::new(NoAlerts),
            devices: Arc::new(NoDevices),
        }
    }

    /// A panel that is on LIVE, the Drain, and stays there however long nobody touches it: most of
    /// these tests are about the Drain's own writes.
    fn on_live<S: Surface>(mut panel: Panel<S>) -> Panel<S> {
        panel.nav.go(Section::Live);
        panel.idle_after_ms = u64::MAX;
        panel
    }

    /// A panel on LIVE that knows nothing, the frames it writes, and the control its key works.
    fn panel() -> (Panel<Recorder>, Frames, FakeControl) {
        let (surface, frames) = recorder();
        let control = FakeControl::default();
        let panel = on_live(Panel::new(
            surface,
            Deps {
                control: Arc::new(control.clone()),
                ..deps(Arc::new(Nothing))
            },
        ));
        (panel, frames, control)
    }

    /// The house as the panel's own tests have it: the owner's labels in the label order, then a device with
    /// a hostname and one with no name, then the router. Addresses are `192.0.2.n`, hardware addresses
    /// `02:00:00:00:00:n`, `n` the octet.
    fn house() -> DevicesView {
        let device = |octet: u8, label: Option<(&str, usize)>, host: Option<&str>, exempt: bool| {
            DeviceView {
                mac: Mac([2, 0, 0, 0, 0, octet]),
                ipv4: vec![Ipv4Addr::new(192, 0, 2, octet)],
                hostname: host.map(str::to_owned),
                first_unix: 1_799_000_000 + u64::from(octet),
                last_unix: UNIX,
                known: true,
                exempt,
                blocked: !exempt,
                label: label.and_then(|(text, _)| Label::new(text)),
                label_rank: label.map(|(_, rank)| rank),
                gate_left_secs: None,
                fixed: false,
                always: false,
            }
        };
        let mut router = device(1, None, None, false);
        router.always = true;
        router.blocked = false;
        DevicesView {
            rev: 1,
            known_list: KnownList::Received,
            devices: vec![
                device(12, Some(("Desk PC", 0)), None, true),
                device(23, Some(("Sam's phone", 1)), None, true),
                device(31, Some(("Living room TV", 2)), None, false),
                device(57, None, Some("printer"), true),
                device(143, None, None, true),
                router,
            ],
            exempt_macs: Vec::new(),
            exempt_pushed: Vec::new(),
            exempt_default: false,
            block_macs: Vec::new(),
            block_pushed: Vec::new(),
            own_mac: None,
            own_ip: None,
        }
    }

    /// The hardware address of the house's device with octet `octet`.
    fn hw(octet: u8) -> [u8; 6] {
        [2, 0, 0, 0, 0, octet]
    }

    /// What a fake inventory has been asked, and what it holds.
    #[derive(Default)]
    struct FakeDevicesState {
        view: Option<DevicesView>,
        asking: Vec<Ipv4Addr>,
        watched: Vec<LaneWatched>,
        /// How many times each was read.
        views: usize,
        askings: usize,
        watched_reads: usize,
        /// The addresses `watch_client` was given, in order.
        watches: Vec<Option<Ipv4Addr>>,
        /// Whose names the lane feed keeps now: the last address `watch_client` was given.
        watching: Option<Ipv4Addr>,
        /// The edits, as `ads 31 true`, `gate 31 300`: the octet of the device and what was asked.
        edits: Vec<String>,
    }

    /// An inventory that counts the calls it is made, keeps the edits, and applies them to its view as the
    /// real one does.
    #[derive(Clone, Default)]
    struct FakeDevices(Arc<Mutex<FakeDevicesState>>);

    impl FakeDevices {
        fn with(view: DevicesView) -> Self {
            let fake = Self::default();
            fake.0.lock().unwrap().view = Some(view);
            fake
        }

        fn state(&self) -> std::sync::MutexGuard<'_, FakeDevicesState> {
            self.0.lock().unwrap()
        }

        /// Changes the inventory a read gives.
        fn edit(&self, change: impl FnOnce(&mut DevicesView)) {
            change(self.state().view.as_mut().unwrap());
        }

        /// (inventory reads, activity reads, watched-names reads) so far.
        fn reads(&self) -> (usize, usize, usize) {
            let state = self.state();
            (state.views, state.askings, state.watched_reads)
        }
    }

    impl DeviceControl for FakeDevices {
        fn wall(&self, _recent: &dyn Fn(u64) -> bool, out: &mut WallCopy) -> bool {
            let mut state = self.state();
            state.views += 1;
            // Every device of the view: `view::wall` cuts it to the wall, as it does the real copy.
            match &state.view {
                Some(view) => {
                    *out = WallCopy::of_view(view);
                    true
                }
                None => false,
            }
        }

        fn asking(&self, _window_secs: u64, out: &mut Vec<Ipv4Addr>) {
            let mut state = self.state();
            state.askings += 1;
            out.clone_from(&state.asking);
        }

        fn set_ads_pass(&self, mac: [u8; 6], pass: bool) -> Result<(), EditError> {
            let mut state = self.state();
            let device = state
                .view
                .as_mut()
                .and_then(|view| view.devices.iter_mut().find(|device| device.mac.0 == mac))
                .ok_or(EditError::Unknown)?;
            if device.fixed {
                return Err(EditError::Fixed);
            }
            device.exempt = pass;
            state.edits.push(format!("ads {} {pass}", mac[5]));
            Ok(())
        }

        fn device_gate(&self, mac: [u8; 6], secs: u16) -> Result<(), EditError> {
            let mut state = self.state();
            let device = state
                .view
                .as_mut()
                .and_then(|view| view.devices.iter_mut().find(|device| device.mac.0 == mac))
                .ok_or(EditError::Unknown)?;
            device.gate_left_secs = (secs > 0).then_some(secs);
            state.edits.push(format!("gate {} {secs}", mac[5]));
            Ok(())
        }

        fn watch_client(&self, client: Option<Ipv4Addr>) {
            let mut state = self.state();
            state.watches.push(client);
            state.watching = client;
        }

        fn watched_names(&self, out: &mut Vec<LaneWatched>) {
            let mut state = self.state();
            state.watched_reads += 1;
            out.clear();
            out.extend(state.watched.iter().map(|name| LaneWatched {
                name: name.name.clone(),
                outcome: name.outcome,
            }));
        }
    }

    /// Alerts that the store would hold: those not acknowledged, and how often the panel asked.
    #[derive(Clone, Default)]
    struct FakeAlerts(Arc<Mutex<FakeAlertsState>>);

    #[derive(Default)]
    struct FakeAlertsState {
        alerts: Vec<(PanelAlert, bool)>,
        asked: usize,
        acks: Vec<u64>,
    }

    impl FakeAlerts {
        fn raise(&self, alert: PanelAlert) {
            self.0.lock().unwrap().alerts.push((alert, false));
        }

        /// Acknowledges as the API does: not by the panel.
        fn api_ack(&self, id: u64) {
            for (alert, acked) in &mut self.0.lock().unwrap().alerts {
                *acked |= alert.id == id;
            }
        }

        fn asked(&self) -> usize {
            self.0.lock().unwrap().asked
        }

        fn acks(&self) -> Vec<u64> {
            self.0.lock().unwrap().acks.clone()
        }
    }

    impl PendingAlerts for FakeAlerts {
        fn top(&self) -> Option<PanelAlert> {
            let mut state = self.0.lock().unwrap();
            state.asked += 1;
            state
                .alerts
                .iter()
                .filter(|(_, acked)| !acked)
                .max_by_key(|(alert, _)| (alert.severity == Severity::High, alert.id))
                .map(|(alert, _)| alert.clone())
        }

        fn waiting_count(&self) -> u32 {
            let state = self.0.lock().unwrap();
            u32::try_from(state.alerts.iter().filter(|(_, acked)| !acked).count()).unwrap()
        }

        fn ack(&self, id: u64) {
            let mut state = self.0.lock().unwrap();
            state.acks.push(id);
            for (alert, acked) in &mut state.alerts {
                *acked |= alert.id == id;
            }
        }
    }

    /// A threat alert about the device at `192.0.2.23`, as the store would carry it.
    fn threat_alert(id: u64) -> PanelAlert {
        PanelAlert {
            id,
            kind: "threat".to_owned(),
            severity: Severity::High,
            client: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 23))),
            subject: "secure-login-verify.example".to_owned(),
            count: 3,
            first_unix: UNIX - 1_000,
            last_unix: UNIX - 60,
        }
    }

    /// A panel with every hand a test reaches it by: what it is shown, what its key controls, the feed
    /// of names, the devices and the alerts.
    struct Rig {
        panel: Panel<Recorder>,
        frames: Frames,
        control: FakeControl,
        lane: Arc<LaneFeed>,
        devices: FakeDevices,
        alerts: FakeAlerts,
    }

    impl Rig {
        /// A rig whose panel is on LIVE.
        fn new(source: Arc<dyn Source>) -> Self {
            let mut rig = Self::home(source);
            rig.panel = on_live(rig.panel);
            rig
        }

        /// A rig whose panel is at HOME, where a boot begins, with the house for its devices.
        fn home(source: Arc<dyn Source>) -> Self {
            let (surface, frames) = recorder();
            let (control, lane) = (FakeControl::default(), LaneFeed::new());
            let (devices, alerts) = (FakeDevices::with(house()), FakeAlerts::default());
            let panel = Panel::new(
                surface,
                Deps {
                    source,
                    control: Arc::new(control.clone()),
                    calibration: Calibration::NOMINAL,
                    calibrate: false,
                    lane: lane.clone(),
                    alerts: Arc::new(alerts.clone()),
                    devices: Arc::new(devices.clone()),
                },
            );
            // What the new panel did to the watch is the tests' own (`a_new_panel_lets_go_...`).
            devices.state().watches.clear();
            Self {
                panel,
                frames,
                control,
                lane,
                devices,
                alerts,
            }
        }

        /// Turns the panel at `now_ms`, with the clock at 10:20, and returns how long it may wait.
        fn turn(&mut self, now_ms: u64) -> u64 {
            self.panel.turn(now_ms, UNIX).unwrap()
        }

        /// The resolver refuses `name`.
        fn refuse(&self, name: &str) {
            self.lane.observe(name, true, 0, |_| false);
        }

        /// Taps the screen at a pixel at `*now`, lets the panel draw what it changed, and moves on: the
        /// press lasts 60 ms and the draw is the turn after it.
        fn tap_at(&mut self, now: &mut u64, x: u16, y: u16) {
            self.panel.touch(&tap(*now, 60, raw_of(x, y)));
            *now += 100;
            self.turn(*now);
            *now += FRAME_MS;
        }

        /// Turns the panel as its own waits ask, for `ms` milliseconds from `*now`; returns what was written.
        fn run_for(&mut self, now: &mut u64, ms: u64) -> Vec<Written> {
            let until = *now + ms;
            self.drive(now, until)
        }

        /// Turns the panel as its own waits ask, from `*now` up to `until`; returns what was written.
        fn drive(&mut self, now: &mut u64, until: u64) -> Vec<Written> {
            let before = self.frames.count();
            let mut times = Vec::new();
            let mut seen = before;
            while *now <= until {
                let wait = self.turn(*now);
                assert!(wait <= FRAME_MS, "waits {wait} ms at {now}");
                for _ in seen..self.frames.count() {
                    times.push(*now);
                }
                seen = self.frames.count();
                *now += wait.max(1);
            }
            let all = self.frames.all();
            times
                .into_iter()
                .enumerate()
                .map(|(n, at)| {
                    let index = before + n;
                    let rows = index.checked_sub(1).map(|previous| {
                        changed_rows(&all[previous], &all[index], WIDTH * 2)
                            .map_or(0, |rows| rows.len())
                    });
                    Written { at, rows }
                })
                .collect()
        }
    }

    /// A frame that was written: when, and how many rows from the first changed to the last it differs
    /// from the frame before by; `None` for the very first.
    #[derive(Debug)]
    struct Written {
        at: u64,
        rows: Option<usize>,
    }

    /// The panel as it draws an empty lane, with the Wi-Fi not joined and nothing known but blocking
    /// `blocking`, and the gate refilled `gate_fill` steps.
    fn frame_with_fill(blocking: Blocking, gate_fill: u8) -> Vec<u8> {
        let stills = Stills {
            blocking,
            gate_fill,
            words: Some(LaneWords::Joining),
            ..Stills::default()
        };
        Painter::new()
            .paint(&Drain::new(DRAIN_SEED), &stills)
            .to_vec()
    }

    /// The panel as it draws an empty lane and nothing known but blocking `blocking`, and the gate not refilled.
    fn frame_of(blocking: Blocking) -> Vec<u8> {
        frame_with_fill(blocking, 0)
    }

    /// The events of one press at the raw reading `raw`: down at `at`, lifted `held_ms` later.
    fn tap(at: u64, held_ms: u64, raw: (i32, i32)) -> Vec<RawEvent> {
        vec![
            key(at, 1),
            abs_x(at, raw.0),
            abs_y(at, raw.1),
            syn(at),
            key(at + held_ms, 0),
            syn(at + held_ms),
        ]
    }

    /// The raw reading that `Calibration::NOMINAL` maps onto the switch, to (40, 290).
    const ON_SWITCH: (i32, i32) = (509, 3564);
    /// The raw reading that `Calibration::NOMINAL` maps onto the middle of the screen, to (240, 100).
    const ELSEWHERE: (i32, i32) = (2054, 1360);

    /// The raw reading that `Calibration::NOMINAL` maps onto the pixel `(x, y)`.
    fn raw_of(x: u16, y: u16) -> (i32, i32) {
        let find = |pixel: u16, span: i32, at: &dyn Fn(u16) -> u16| -> i32 {
            let near = 200 + i32::from(pixel) * 3_700 / span;
            (near - 12..=near + 12)
                .find(|&raw| at(u16::try_from(raw).unwrap()) == pixel)
                .expect("a reading on the pixel")
        };
        (
            find(x, 479, &|raw| {
                qb_panel::to_screen(&Calibration::NOMINAL, raw, 200).0
            }),
            find(y, 319, &|raw| {
                qb_panel::to_screen(&Calibration::NOMINAL, 200, raw).1
            }),
        )
    }

    /// The events of a tap on the pixel `(x, y)`: 60 ms down, from `at`.
    fn tap_on(at: u64, x: u16, y: u16) -> Vec<RawEvent> {
        tap(at, 60, raw_of(x, y))
    }

    /// Where a tap works the Drain's key, and the keys of the gate ask that it unfolds.
    const KEY: (u16, u16) = (40, 290);
    const ASK_5_MIN: (u16, u16) = (180, 270);
    const ASK_30_MIN: (u16, u16) = (300, 270);

    /// Temporary `Roots` and the directories that hold them, which go when this does.
    struct Fixture {
        roots: Roots,
        _dirs: Vec<TempDir>,
    }

    impl Deref for Fixture {
        type Target = Roots;

        fn deref(&self) -> &Roots {
            &self.roots
        }
    }

    /// Roots of a board with the given framebuffers: the image's module table, an empty `/dev`,
    /// and a text console on a framebuffer that is bound.
    fn roots(framebuffers: &[Graphics]) -> Fixture {
        let graphics = graphics(framebuffers);
        let files = tempfile::tempdir().unwrap();
        let module_list = files.path().join("modules.list");
        fs::write(&module_list, DEFAULT_TABLE).unwrap();
        let vtconsole = tempfile::tempdir().unwrap();
        for (dir, name, bind) in [
            ("vtcon0", "(S) dummy device\n", "0\n"),
            ("vtcon1", "(M) frame buffer device\n", "1\n"),
        ] {
            fs::create_dir(vtconsole.path().join(dir)).unwrap();
            fs::write(vtconsole.path().join(dir).join("name"), name).unwrap();
            fs::write(vtconsole.path().join(dir).join("bind"), bind).unwrap();
        }
        let dev = tempfile::tempdir().unwrap();
        Fixture {
            roots: Roots {
                module_list,
                sys_graphics: graphics.path().to_owned(),
                sys_vtconsole: vtconsole.path().to_owned(),
                dev: dev.path().to_owned(),
            },
            _dirs: vec![graphics, files, vtconsole, dev],
        }
    }

    /// The screen as the kernel lists it.
    const SCREEN: Graphics = ("fb1", "fb_ili9486", "480,320", "16");

    #[test]
    fn names_from_the_lane_move_at_eight_frames_a_second() {
        let mut rig = Rig::new(Arc::new(Nothing));
        rig.refuse("doubleclick.net");
        let mut now = 0;
        let writes = rig.drive(&mut now, 8_000);
        // The panel is turned at 0, 125, 250, ...: nothing else is due, for no figure changes.
        assert_eq!(writes[0].at, 0);
        for write in &writes {
            assert_eq!(write.at % FRAME_MS, 0, "{write:?}");
            // Each write fits a quarter of the screen, as the budget has it.
            assert!(write.rows.is_none_or(|rows| rows <= 80), "{write:?}");
        }
        // The name is in motion for some forty frames: the lane, then seventeen steps of the refusal.
        assert!(writes.len() >= 25, "{} writes", writes.len());
        let gaps: Vec<u64> = writes
            .windows(2)
            .map(|pair| pair[1].at - pair[0].at)
            .collect();
        assert!(gaps.iter().all(|gap| gap % FRAME_MS == 0), "{gaps:?}");
        // Steps of the refusal that move nothing on the screen write nothing, and leave a gap of two
        // frames: the hold, and the count (the figure is not on the screen here). The drop at the
        // spout is painted, so it writes.
        assert_eq!(
            gaps.iter().filter(|&&gap| gap == 2 * FRAME_MS).count(),
            2,
            "{gaps:?}"
        );
        assert!(gaps.iter().all(|&gap| gap <= 2 * FRAME_MS), "{gaps:?}");
        // When the name is done the panel is idle: no write for the last seconds.
        let last = writes.last().unwrap().at;
        assert!(last < 6_000, "{last}");
        let rows: Vec<usize> = writes.iter().filter_map(|write| write.rows).collect();
        eprintln!(
            "rows per write, one refused name: max {}, mean {:.1} over {} writes: {rows:?}",
            rows.iter().max().unwrap(),
            rows.iter().sum::<usize>() as f64 / rows.len() as f64,
            rows.len()
        );
    }

    #[test]
    fn an_idle_panel_paints_nothing() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        let writes = rig.drive(&mut now, 10_000);
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(writes[0].at, 0);
        // The panel did look: it was turned every frame for ten seconds.
        assert!(now > 10_000);
    }

    #[test]
    fn a_turn_that_comes_late_makes_one_frame_and_counts_the_beat_from_it() {
        let mut rig = Rig::new(Arc::new(Nothing));
        rig.refuse("doubleclick.net");
        assert_eq!(rig.turn(0), FRAME_MS);
        assert_eq!(rig.turn(124), 1);
        assert_eq!(rig.frames.count(), 1);
        // The thread was held up for half a minute: one frame, not two hundred and forty.
        assert_eq!(rig.turn(30_000), FRAME_MS);
        assert_eq!(rig.frames.count(), 2);
        assert_eq!(rig.turn(30_124), 1);
        assert_eq!(rig.frames.count(), 2);
    }

    #[test]
    fn the_first_frame_is_the_panel_of_what_is_known_now() {
        let (mut panel, frames, _) = panel();
        panel.turn(0, UNIX).unwrap();
        assert_eq!(frames.count(), 1);
        assert!(frames.last() == frame_of(Blocking::On));

        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        rig.turn(0);
        let mut drain = Drain::new(DRAIN_SEED);
        drain.set_blocked_today(Some(4_780));
        let stills = Stills {
            percent: Some(Share::Percent(44)),
            clock: Some(TimeOfDay {
                hour: 10,
                minute: 20,
            }),
            blocking: Blocking::On,
            gate_fill: 0,
            count: Some(4_780),
            gate_ask: false,
            words: None,
        };
        assert_eq!(rig.frames.count(), 1);
        assert!(rig.frames.last() == Painter::new().paint(&drain, &stills));
    }

    #[test]
    fn wanted_stills_from_the_reference_snapshot() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        // A refused name is on its way: the count is the Drain's, and holds it back.
        rig.refuse("doubleclick.net");
        rig.turn(0);
        let shown = rig.panel.shown.clone();
        assert_eq!(shown.percent, Some(Share::Percent(44)));
        assert_eq!(
            shown.clock,
            Some(TimeOfDay {
                hour: 10,
                minute: 20
            })
        );
        assert_eq!(
            (shown.count, rig.panel.drain.count()),
            (Some(4_779), Some(4_779))
        );
        assert_eq!(rig.panel.wanted, shown);

        // Without the clock set there is no time of day.
        let unset = Snapshot {
            clock_set: false,
            ..reference_snapshot()
        };
        let mut rig = Rig::new(Arc::new(Fixed(unset)));
        rig.turn(0);
        assert_eq!(rig.panel.shown.clock, None);
        assert_eq!(rig.panel.shown.percent, Some(Share::Percent(44)));
    }

    #[test]
    fn a_count_that_rises_is_written_on_its_own_half_a_frame_after_the_lane() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        rig.refuse("doubleclick.net");
        let mut now = 0;
        let writes = rig.drive(&mut now, 6_000);
        let late: Vec<&Written> = writes
            .iter()
            .filter(|write| write.at % FRAME_MS == STILL_OFFSET_MS)
            .collect();
        // One figure changed: the count, 4,779 to 4,780, when the name melted away.
        assert_eq!(late.len(), 1, "{writes:?}");
        assert!(late[0].rows.is_some_and(|rows| rows <= 80), "{late:?}");
        assert!(
            writes
                .iter()
                .all(|write| write.at % FRAME_MS == 0 || write.at % FRAME_MS == STILL_OFFSET_MS),
            "{writes:?}"
        );
        assert!(
            writes
                .iter()
                .all(|write| write.rows.is_none_or(|rows| rows <= 80)),
            "{writes:?}"
        );
        assert_eq!(rig.panel.shown.count, Some(4_780));
    }

    #[test]
    fn a_screen_that_cannot_be_written_ends_the_turn_with_its_error() {
        let error = Panel::new(Broken, deps(Arc::new(Nothing)))
            .turn(0, UNIX)
            .unwrap_err();
        assert_eq!(error.to_string(), "the screen is gone");
    }

    #[test]
    fn a_board_without_the_screen_is_no_screen_not_a_failure() {
        let r = roots(&[HDMI]);
        assert_eq!(
            bring_up(&r, &mut |_, _| Ok(()), Duration::ZERO),
            Err(PanelEnd::NoScreen("no fb_ili9486 framebuffer".into()))
        );
    }

    #[test]
    fn a_driver_that_does_not_load_is_no_screen_and_names_the_module() {
        let r = roots(&[HDMI, SCREEN]);
        let mut insert = |file: &Path, _: &str| {
            if file.ends_with("fbtft.ko") {
                Err(io::Error::other("bad signature"))
            } else {
                Ok(())
            }
        };
        assert_eq!(
            bring_up(&r, &mut insert, Duration::ZERO),
            Err(PanelEnd::NoScreen(
                "loading fb_ili9486: bad signature".into()
            ))
        );
    }

    #[test]
    fn a_module_list_that_cannot_be_read_is_no_screen_and_names_the_file() {
        let r = roots(&[HDMI, SCREEN]);
        fs::remove_file(&r.module_list).unwrap();
        match bring_up(&r, &mut |_, _| Ok(()), Duration::ZERO) {
            Err(PanelEnd::NoScreen(why)) => assert!(why.contains("modules.list"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_modules_of_the_screen_and_the_touch_controller_are_loaded_in_order() {
        let r = roots(&[HDMI, SCREEN]);
        let mut files = Vec::new();
        let mut insert = |file: &Path, _: &str| {
            files.push(file.file_name().unwrap().to_str().unwrap().to_owned());
            Ok(())
        };
        bring_up(&r, &mut insert, Duration::ZERO).unwrap();
        assert_eq!(
            files,
            [
                "spi-bcm2835.ko",
                "backlight.ko",
                "fbtft.ko",
                "fb_ili9486.ko",
                "ads7846.ko"
            ]
        );
    }

    #[test]
    fn a_screen_in_another_rotation_is_refused() {
        let r = roots(&[HDMI, ("fb1", "fb_ili9486", "320,480", "16")]);
        assert!(matches!(
            bring_up(&r, &mut |_, _| Ok(()), Duration::ZERO),
            Err(PanelEnd::NoScreen(why)) if why.contains("320,480")
        ));
    }

    #[test]
    fn the_console_is_taken_off_the_screen_only_when_it_is_the_only_framebuffer() {
        let alone = roots(&[("fb0", "fb_ili9486", "480,320", "16")]);
        assert_eq!(
            bring_up(&alone, &mut |_, _| Ok(()), Duration::ZERO)
                .unwrap()
                .device,
            alone.dev.join("fb0")
        );
        assert_eq!(
            fs::read_to_string(alone.sys_vtconsole.join("vtcon1/bind")).unwrap(),
            "0"
        );
        let beside_hdmi = roots(&[HDMI, SCREEN]);
        bring_up(&beside_hdmi, &mut |_, _| Ok(()), Duration::ZERO).unwrap();
        assert_eq!(
            fs::read_to_string(beside_hdmi.sys_vtconsole.join("vtcon1/bind")).unwrap(),
            "1\n"
        );
    }

    #[test]
    fn a_screen_that_appears_while_its_driver_comes_up_is_waited_for() {
        let r = roots(&[HDMI]);
        let appearing = graphics(&[SCREEN]);
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(150));
                // Whole, so that it is never seen half made.
                fs::rename(appearing.path().join("fb1"), r.sys_graphics.join("fb1")).unwrap();
            });
            let screen = bring_up(&r, &mut |_, _| Ok(()), Duration::from_secs(30)).unwrap();
            assert_eq!(screen.device, r.dev.join("fb1"));
        });
    }

    #[test]
    fn a_screen_that_never_appears_is_waited_for_as_long_as_asked_and_no_longer() {
        let r = roots(&[HDMI]);
        let began = Instant::now();
        let outcome = bring_up(&r, &mut |_, _| Ok(()), Duration::from_millis(200));
        assert_eq!(
            outcome,
            Err(PanelEnd::NoScreen("no fb_ili9486 framebuffer".into()))
        );
        assert!(began.elapsed() >= Duration::from_millis(200));
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    /// The panel as it draws an empty lane and nothing known but the gate's choices unfolded.
    fn frame_asking() -> Vec<u8> {
        let stills = Stills {
            gate_ask: true,
            words: Some(LaneWords::Joining),
            ..Stills::default()
        };
        Painter::new()
            .paint(&Drain::new(DRAIN_SEED), &stills)
            .to_vec()
    }

    #[test]
    fn a_tap_on_the_key_unfolds_the_choices_a_choice_opens_the_gate_and_the_countdown_steps() {
        let (mut panel, frames, control) = panel();
        panel.turn(0, UNIX).unwrap();
        // Both halves in one read: timed by the events, not by the read. With the gate shut the key unfolds
        // its choices in place, and nothing is paused yet.
        panel.touch(&tap_on(1_000, KEY.0, KEY.1));
        assert!(control.calls().is_empty());
        assert!(panel.turn(1_010, UNIX).unwrap() <= FRAME_MS);
        assert_eq!(frames.count(), 2);
        assert!(frames.last() == frame_asking());
        // A choice opens the gate for its time, and folds the choices in the same write.
        panel.touch(&tap_on(2_000, ASK_5_MIN.0, ASK_5_MIN.1));
        assert_eq!(control.calls(), ["pause 300"]);
        assert!(panel.turn(2_010, UNIX).unwrap() <= FRAME_MS);
        assert_eq!(frames.count(), 3);
        assert!(frames.last() == frame_of(Blocking::Paused { left_secs: 300 }));
        // The control's seconds step, on its own clock: the next frame notices, and half a frame
        // later the new figure is written, and nothing else.
        control.set_left(Some(299));
        assert_eq!(panel.turn(2_135, UNIX).unwrap(), STILL_OFFSET_MS);
        assert_eq!(frames.count(), 3);
        assert_eq!(
            panel.turn(2_135 + STILL_OFFSET_MS, UNIX).unwrap(),
            FRAME_MS - STILL_OFFSET_MS
        );
        assert_eq!(frames.count(), 4);
        assert!(frames.last() == frame_of(Blocking::Paused { left_secs: 299 }));
        // The same seconds again: nothing to write.
        for at in [2_260, 2_322, 2_385, 2_447] {
            panel.turn(at, UNIX).unwrap();
        }
        assert_eq!(frames.count(), 4);
    }

    #[test]
    fn the_choices_open_the_gate_for_half_an_hour_and_an_hour_and_a_tap_elsewhere_only_folds_them()
    {
        let (mut panel, _, control) = panel();
        panel.turn(0, UNIX).unwrap();
        for (secs, (x, y)) in [(1_800, ASK_30_MIN), (3_600, (420, 270))] {
            control.resume();
            panel.turn(500, UNIX).unwrap();
            panel.touch(&tap_on(1_000, KEY.0, KEY.1));
            panel.turn(1_100, UNIX).unwrap();
            assert!(panel.nav.gate_ask());
            panel.touch(&tap_on(2_000, x, y));
            assert!(!panel.nav.gate_ask());
            assert_eq!(control.calls().last().unwrap(), &format!("pause {secs}"));
            panel.turn(2_100, UNIX).unwrap();
        }
        // Anywhere else on the screen only folds the choices.
        control.resume();
        panel.turn(3_000, UNIX).unwrap();
        panel.touch(&tap_on(4_000, KEY.0, KEY.1));
        panel.turn(4_100, UNIX).unwrap();
        let calls = control.calls().len();
        panel.touch(&tap_on(5_000, 300, 150));
        assert!(!panel.nav.gate_ask());
        assert_eq!(control.calls().len(), calls);
    }

    #[test]
    fn a_tap_while_paused_resumes() {
        let (mut panel, frames, control) = panel();
        // Five minutes left of a pause of five minutes: the gate is open and not yet refilled.
        control.set_left(Some(300));
        panel.turn(0, UNIX).unwrap();
        panel.touch(&tap_on(1_000, KEY.0, KEY.1));
        assert_eq!(control.calls(), ["resume"]);
        assert!(!panel.nav.gate_ask());
        assert_eq!(panel.turn(1_100, UNIX).unwrap(), FRAME_MS);
        assert!(frames.last() == frame_of(Blocking::On));
    }

    #[test]
    fn a_brush_and_a_tap_elsewhere_change_nothing() {
        let (mut panel, frames, control) = panel();
        panel.turn(0, UNIX).unwrap();
        panel.touch(&tap(1_000, 30, ON_SWITCH));
        // Away from the key a tap does nothing.
        panel.touch(&tap(2_000, 200, ELSEWHERE));
        panel.turn(2_500, UNIX).unwrap();
        assert!(control.calls().is_empty());
        assert_eq!(frames.count(), 1);
    }

    #[test]
    fn a_finger_left_on_the_key_unfolds_the_choices_once_when_it_lifts() {
        let (mut panel, _, control) = panel();
        panel.touch(&tap(1_000, 240_000, ON_SWITCH));
        assert!(panel.nav.gate_ask());
        assert!(control.calls().is_empty());
    }

    #[test]
    fn a_press_cut_off_by_lost_events_does_nothing() {
        let (mut panel, _, control) = panel();
        let press = tap(1_000, 200, ON_SWITCH);
        // `tap` is four events down, two up.
        let (down, up) = press.split_at(4);
        panel.touch(down);
        panel.touch(&[dropped(1_100)]);
        panel.touch(up);
        assert!(control.calls().is_empty());
        assert!(!panel.nav.gate_ask());
    }

    #[test]
    fn a_press_whose_first_sample_was_lost_does_nothing_either() {
        let (mut panel, _, control) = panel();
        // A touch too light to count: the glass is down, but no touch was ever given out.
        panel.touch(&[
            key(1_000, 1),
            abs_x(1_000, ON_SWITCH.0),
            abs_y(1_000, ON_SWITCH.1),
            pressure(1_000, 5),
            syn(1_000),
        ]);
        // Then the kernel loses events, which is `Lost` for a press that never had a `Down`.
        panel.touch(&[dropped(1_040), key(1_300, 0), syn(1_300)]);
        assert!(control.calls().is_empty());
        assert!(!panel.nav.gate_ask());
        // The next press is whole again.
        panel.touch(&[pressure(1_900, 180)]);
        panel.touch(&tap(2_000, 60, ON_SWITCH));
        assert!(panel.nav.gate_ask());
    }

    #[test]
    fn a_pause_that_ends_by_itself_is_drawn_within_a_beat_and_a_half() {
        let (mut panel, frames, control) = panel();
        control.set_left(Some(1));
        assert_eq!(panel.turn(0, UNIX).unwrap(), FRAME_MS);
        // Everything known is on the first frame, the gate nearly refilled.
        assert!(frames.last() == frame_with_fill(Blocking::Paused { left_secs: 1 }, 18));
        control.set_left(None);
        // The next frame notices, and half a frame later the key is on again.
        assert_eq!(panel.turn(FRAME_MS, UNIX).unwrap(), STILL_OFFSET_MS);
        assert_eq!(frames.count(), 1);
        assert_eq!(
            panel.turn(FRAME_MS + STILL_OFFSET_MS, UNIX).unwrap(),
            FRAME_MS - STILL_OFFSET_MS
        );
        assert_eq!(frames.count(), 2);
        assert!(frames.last() == frame_of(Blocking::On));
        // The refill is its own figure and goes back to nothing in the next slot, which changes
        // nothing a shut gate shows: no write. Then the panel is calm again.
        assert_eq!(panel.turn(2 * FRAME_MS, UNIX).unwrap(), STILL_OFFSET_MS);
        assert_eq!(
            panel.turn(2 * FRAME_MS + STILL_OFFSET_MS, UNIX).unwrap(),
            FRAME_MS - STILL_OFFSET_MS
        );
        assert_eq!(panel.turn(3 * FRAME_MS, UNIX).unwrap(), FRAME_MS);
        assert_eq!(frames.count(), 2);
    }

    #[test]
    fn a_pause_that_began_without_a_tap_is_drawn_in_the_figures_slot() {
        let (mut panel, frames, control) = panel();
        assert_eq!(panel.turn(0, UNIX).unwrap(), FRAME_MS);
        control.set_left(Some(120));
        // Half a frame after the frame, when the figures are written, and not before.
        assert_eq!(panel.turn(10, UNIX).unwrap(), STILL_OFFSET_MS - 10);
        assert_eq!(frames.count(), 1);
        assert_eq!(
            panel.turn(STILL_OFFSET_MS, UNIX).unwrap(),
            FRAME_MS - STILL_OFFSET_MS
        );
        assert_eq!(frames.count(), 2);
        assert!(frames.last() == frame_of(Blocking::Paused { left_secs: 120 }));
    }

    #[test]
    fn a_figure_that_is_late_in_the_frame_waits_for_the_next_frames_slot() {
        let (mut panel, frames, control) = panel();
        assert_eq!(panel.turn(0, UNIX).unwrap(), FRAME_MS);
        control.set_left(Some(120));
        // Touch events that are no tap wake the loop at +120 ms: past the figures' slot, and in
        // the flush window of the next frame's write. The figure is not written, and the next
        // frame is the next thing to wait for.
        panel.touch(&tap(100, 10, ELSEWHERE));
        assert_eq!(panel.turn(120, UNIX).unwrap(), FRAME_MS - 120);
        assert_eq!(frames.count(), 1);
        // The frame has nothing to write either, and its slot comes half a frame later.
        assert_eq!(panel.turn(FRAME_MS, UNIX).unwrap(), STILL_OFFSET_MS);
        assert_eq!(frames.count(), 1);
        assert_eq!(
            panel.turn(FRAME_MS + STILL_OFFSET_MS, UNIX).unwrap(),
            FRAME_MS - STILL_OFFSET_MS
        );
        assert_eq!(frames.count(), 2);
        assert!(frames.last() == frame_of(Blocking::Paused { left_secs: 120 }));
    }

    /// A Wi-Fi link that is up.
    fn joined() -> WifiStatus {
        WifiStatus {
            associated: true,
            signal_dbm: Some(-63),
            ap: None,
            freq_mhz: None,
        }
    }

    /// A source whose figures rise at every look, as a resolver's do under traffic.
    struct Counting(Mutex<(u64, u64)>);

    impl Source for Counting {
        fn snapshot(&self) -> Snapshot {
            let mut counts = self.0.lock().unwrap();
            counts.0 += 7;
            counts.1 += 3;
            Snapshot {
                wifi: Some(joined()),
                today: Some(Counts {
                    queries: counts.0,
                    blocked: counts.1,
                    blocked_threat: 0,
                    cache_hits: 0,
                    forwarded: 0,
                }),
                clock_set: true,
                ..Snapshot::default()
            }
        }

        fn recent(&self, _n: usize) -> Vec<LogEntry> {
            Vec::new()
        }
    }

    /// A surface that measures each frame against the one before, in rows from the first changed to
    /// the last, and keeps only the numbers.
    struct Measure {
        previous: Vec<u8>,
        rows: Arc<Mutex<Vec<usize>>>,
    }

    impl Surface for Measure {
        fn show(&mut self, frame: &[u8]) -> io::Result<()> {
            if !self.previous.is_empty() {
                let rows =
                    changed_rows(&self.previous, frame, WIDTH * 2).map_or(0, |rows| rows.len());
                self.rows.lock().unwrap().push(rows);
            }
            self.previous.clear();
            self.previous.extend_from_slice(frame);
            Ok(())
        }
    }

    #[test]
    fn steady_traffic_writes_at_most_eighty_rows_at_a_time() {
        let rows = Arc::new(Mutex::new(Vec::new()));
        let lane = LaneFeed::new();
        let mut panel = on_live(Panel::new(
            Measure {
                previous: Vec::new(),
                rows: rows.clone(),
            },
            Deps {
                lane: lane.clone(),
                ..deps(Arc::new(Counting(Mutex::new((0, 0)))))
            },
        ));
        // Half a minute before the minute turns, so that the clock's figure changes during the run.
        let start = TEN_TWENTY_UTC + 30;
        let mut asked = 0;
        // How many frames the Drain's queue was full at, and how many refusals ran to the door reopening.
        let (mut full, mut refusals, mut door_was_shut) = (0, 0, false);
        for frame in 0..400_u64 {
            let at = frame * FRAME_MS;
            // A page load that never ends: a dozen refused names and a plain one in every frame, all
            // different, which is more than the feed holds. Each stays different after fitting to the
            // 15 cells of a refused name and the 20 of a plain one: `fit_site` drops host labels, so a
            // name made of a host in front of one site would be the same site for all of them.
            for n in 0..12 {
                lane.observe(&format!("a{frame:03}{n:02}.example"), true, frame, |_| {
                    false
                });
            }
            lane.observe(&format!("site{frame}.example.org"), false, frame, |_| false);
            asked += 13;
            for at in [at, at + STILL_OFFSET_MS] {
                panel.turn(at, start + at / 1_000).unwrap();
                refusals += usize::from(door_was_shut && !panel.drain.door_shut());
                door_was_shut = panel.drain.door_shut();
            }
            full += usize::from(panel.drain.queued() == QUEUE_MAX);
        }
        let rows = rows.lock().unwrap().clone();
        let (peak, total) = (rows.iter().max().unwrap(), rows.iter().sum::<usize>());
        let mut sorted = rows.clone();
        sorted.sort_unstable();
        eprintln!(
            "rows per write, 50 s of heavy traffic: {} writes, peak {peak}, p95 {}, mean {:.1}",
            rows.len(),
            sorted[sorted.len() * 95 / 100],
            total as f64 / rows.len() as f64
        );
        assert!(*peak <= 80, "peak {peak}");
        assert!(rows.len() > 300, "{} writes", rows.len());
        // The flood floods: the Drain's queue was full in nearly every frame, and refusal after refusal
        // ran at the lane's own pace, 15 frames to the gate, 17 steps and one to leave.
        eprintln!("the queue was full in {full} of 400 frames, {refusals} refusals ran");
        assert!(full >= 360, "the queue was full in {full} of 400 frames");
        assert!(refusals >= 10, "{refusals} refusals ran");
        // The lane took what it could and the rest were dropped, not queued.
        assert!(lane.dropped() > 0 && asked > 5_000);
        // The clock's minute turned (10:20 to 10:21) while the lane ran.
        assert_eq!(
            panel.shown.clock,
            Some(TimeOfDay {
                hour: 10,
                minute: 21
            })
        );
    }

    /// A refusal that is landing when the key is tapped, with a listed name on the lane: the lane's
    /// restyle goes out in the key's own write, and no motion write after it is over 80 rows. `early`
    /// has the tap come in the very turn of a frame.
    fn toggle_while_a_refusal_lands(early: bool) {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        // A listed name follows a refused one of 11 letters: the refused one stops at the gate and melts
        // while the listed one waits on the lane behind it. (One ahead of it would have left the lane, which
        // ends at the gate's cell now that `HOME` takes the cells left of it.) The refusal's dots land at
        // step 11, which is a motion write to itself.
        rig.refuse("ads.example");
        rig.lane.observe("listed.example", false, 1, |_| true);
        // The gate's choices are unfolded first, over the field and the funnel (the refusal runs on
        // under them); the toggle is the tap on a choice.
        rig.turn(0);
        rig.panel.touch(&tap_on(10, KEY.0, KEY.1));
        let (mut now, land) = (20, 11);
        loop {
            let wait = rig.turn(now);
            if rig.panel.drain.refusal_step() == Some(land - 1) {
                break;
            }
            assert!(now < 20_000, "the refusal never got near its landing");
            now += wait.max(1);
        }
        assert!(
            rig.panel
                .drain
                .lane()
                .all(|tile| tile.style != TileStyle::Listed),
            "nothing is outlined while blocking is on"
        );
        // The tap comes half a frame after the frame, or in the turn of the next one.
        let at = if early { now + FRAME_MS } else { now + 30 };
        rig.panel.touch(&tap_on(at - 100, ASK_5_MIN.0, ASK_5_MIN.1));
        assert_eq!(rig.control.calls(), ["pause 300"]);
        let before = rig.frames.count();
        rig.turn(at);
        let outlined = rig
            .panel
            .drain
            .lane()
            .any(|tile| tile.style == TileStyle::Listed);
        let mut now = at;
        while rig.panel.drain.moving() || now < at + 4 * FRAME_MS {
            now += rig.turn(now).max(1);
            assert!(now < 30_000, "the refusal never ended");
        }
        let all = rig.frames.all();
        let rows: Vec<usize> = (before..all.len())
            .map(|n| changed_rows(&all[n - 1], &all[n], WIDTH * 2).map_or(0, |rows| rows.len()))
            .collect();
        eprintln!("rows of the writes from a pause toggle on, a refusal landing: {rows:?}");
        // Every write but the key's own is a motion write or a figure, and fits. The landing is one of them.
        assert!(rows.iter().skip(1).all(|&rows| rows <= 80), "{rows:?}");
        // The first write is the key's: the gate and the key flip, and the lane's restyle is in it.
        assert!(
            rows[0] > 80,
            "the toggle's own write is a large one: {rows:?}"
        );
        assert!(
            outlined,
            "a listed name is on the lane, outlined now that blocking is paused"
        );
        assert!(
            rig.panel
                .drain
                .funnel()
                .iter()
                .filter(|cell| cell.shade.is_some())
                .count()
                >= 11,
            "the refusal landed its dots after the toggle"
        );
    }

    #[test]
    fn a_pause_toggle_restyles_the_lane_in_its_own_write_not_a_motion_write() {
        toggle_while_a_refusal_lands(false);
    }

    #[test]
    fn a_tap_in_the_turn_of_a_frame_puts_the_key_ahead_of_the_motion_write() {
        toggle_while_a_refusal_lands(true);
    }

    /// A source whose snapshot a test changes while the panel runs.
    #[derive(Clone, Default)]
    struct Changing(Arc<Mutex<Snapshot>>);

    impl Changing {
        fn set(&self, snapshot: Snapshot) {
            *self.0.lock().unwrap() = snapshot;
        }
    }

    impl Source for Changing {
        fn snapshot(&self) -> Snapshot {
            self.0.lock().unwrap().clone()
        }

        fn recent(&self, _n: usize) -> Vec<LogEntry> {
            Vec::new()
        }
    }

    /// The counters of a joined board that has answered `queries` today and refused `blocked` of them,
    /// with the clock set.
    fn counted(blocked: u64, queries: u64) -> Snapshot {
        Snapshot {
            today: Some(Counts {
                queries,
                blocked,
                ..Counts::default()
            }),
            ..reference_snapshot()
        }
    }

    /// The panel as it draws a Drain with no names, a funnel that is empty, and `stills`; `blocked` is
    /// the day's figure the Drain is given.
    fn screen_of(blocked: Option<u32>, stills: &Stills) -> Vec<u8> {
        let mut drain = Drain::new(DRAIN_SEED);
        drain.set_blocked_today(blocked);
        Painter::new().paint(&drain, stills).to_vec()
    }

    /// The clock the reference snapshot shows.
    const TEN_TWENTY: Option<TimeOfDay> = Some(TimeOfDay {
        hour: 10,
        minute: 20,
    });

    #[test]
    fn boot_shows_joining_then_listening_then_the_count() {
        let source = Changing::default();
        source.set(Snapshot {
            clock_set: true,
            ..Snapshot::default()
        });
        let mut rig = Rig::new(Arc::new(source.clone()));
        // The Wi-Fi is not joined: the lane says so, and the percentage and the count are dashes.
        rig.turn(0);
        let joining = Stills {
            clock: TEN_TWENTY,
            words: Some(LaneWords::Joining),
            ..Stills::default()
        };
        assert_eq!(rig.panel.shown, joining);
        assert!(rig.frames.last() == screen_of(None, &joining));
        assert_eq!(rig.frames.count(), 1);

        // Joined, and no query has come: `LISTENING` and a true zero. The two figures go in writes of
        // their own.
        source.set(Snapshot {
            wifi: Some(joined()),
            today: Some(Counts::default()),
            clock_set: true,
            ..Snapshot::default()
        });
        let mut now = FRAME_MS;
        let writes = rig.drive(&mut now, 2_500);
        let listening = Stills {
            clock: TEN_TWENTY,
            count: Some(0),
            words: Some(LaneWords::Listening),
            ..Stills::default()
        };
        assert_eq!(rig.panel.shown, listening);
        assert!(rig.frames.last() == screen_of(Some(0), &listening));
        assert_eq!(writes.len(), 2, "{writes:?}");

        // Queries come, some refused: the words go and the figures are the resolver's.
        source.set(counted(5, 12));
        let writes = rig.drive(&mut now, 5_000);
        let counting = Stills {
            percent: Some(Share::Percent(42)),
            clock: TEN_TWENTY,
            count: Some(5),
            ..Stills::default()
        };
        assert_eq!(rig.panel.shown, counting);
        assert!(rig.frames.last() == screen_of(Some(5), &counting));
        assert_eq!(
            writes.len(),
            3,
            "the count, the share, the words: {writes:?}"
        );
        assert!(
            writes
                .iter()
                .all(|write| write.rows.is_none_or(|rows| rows <= 80)),
            "{writes:?}"
        );
    }

    #[test]
    fn a_count_stays_a_dash_while_the_wifi_is_not_joined() {
        let source = Changing::default();
        source.set(reference_snapshot());
        let mut rig = Rig::new(Arc::new(source.clone()));
        let mut now = 0;
        rig.drive(&mut now, 1_500);
        assert_eq!(rig.panel.shown.count, Some(4_780));

        // The link drops. Within a snapshot the count and the share are dashes and the lane says why.
        source.set(Snapshot {
            wifi: Some(WifiStatus::default()),
            ..reference_snapshot()
        });
        rig.drive(&mut now, 3_000);
        let shown = rig.panel.shown.clone();
        assert_eq!(
            (shown.count, shown.percent, shown.words),
            (None, None, Some(LaneWords::Joining))
        );

        // Names go on being offered (the resolver is the LAN's, whatever the link). Whole refusals
        // pass: the Drain's own count, which counts a name when it melts, is never shown.
        for name in ["example.org", "example.com", "ads"] {
            rig.refuse(name);
        }
        let mut done = 0;
        while now < 40_000 {
            let wait = rig.turn(now);
            now += wait.max(1);
            assert_eq!(rig.panel.shown.count, None, "at {now}");
            assert_eq!(rig.panel.wanted.count, None, "at {now}");
            assert_eq!(rig.panel.drain.count(), None, "at {now}");
            done += usize::from(rig.panel.drain.refusal_step().is_some());
        }
        assert!(done > 30, "the names were on their way: {done} frames");

        // The link is back: the count is the figure again, and the names that melted are in it.
        source.set(counted(4_783, 10_821));
        rig.drive(&mut now, 42_000);
        assert_eq!(rig.panel.shown.count, Some(4_783));
        assert_eq!(rig.panel.shown.percent, Some(Share::Percent(44)));
        assert_eq!(rig.panel.shown.words, None);
    }

    #[test]
    fn quiet_after_two_minutes_without_a_lane_name_and_gone_when_one_comes() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        // The panel's thread may be running a good while before its first frame: the quiet is counted
        // from that frame, not from the thread's start.
        let first = 40_000;
        let mut now = first;
        rig.drive(&mut now, first + 119_000);
        assert_eq!(rig.panel.shown.words, None, "two minutes have not passed");
        rig.drive(&mut now, first + 122_000);
        let quiet = |minutes| Stills {
            percent: Some(Share::Percent(44)),
            clock: TEN_TWENTY,
            count: Some(4_780),
            words: Some(LaneWords::Quiet { minutes }),
            ..Stills::default()
        };
        assert_eq!(rig.panel.shown, quiet(2));
        assert!(rig.frames.last() == screen_of(Some(4_780), &quiet(2)));
        // The words change once a minute at most, each in a write of its own.
        let before = rig.frames.count();
        let writes = rig.drive(&mut now, first + 175_000);
        assert!(writes.is_empty(), "{writes:?}");
        let writes = rig.drive(&mut now, first + 185_000);
        assert_eq!(rig.panel.shown.words, Some(LaneWords::Quiet { minutes: 3 }));
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(rig.frames.count(), before + 1);

        // A name comes. The words go in the frame that takes it: the lane is not idle, so they
        // are not drawn, though the figure for them is still on its way out.
        let mut rest = rig.panel.shown.clone();
        rest.words = None;
        rig.refuse("ads.example.org");
        let taken = now;
        let until = now + FRAME_MS;
        rig.drive(&mut now, until);
        assert!(!rig.panel.drain.idle());
        assert!(rig.frames.last() == Painter::new().paint(&rig.panel.drain, &rest));
        rig.drive(&mut now, taken + 2_000);
        assert_eq!(rig.panel.shown.words, None);

        // And the quiet is counted again from that name.
        rig.drive(&mut now, taken + 118_000);
        assert_eq!(rig.panel.shown.words, None);
        rig.drive(&mut now, taken + 123_000);
        assert_eq!(rig.panel.shown.words, Some(LaneWords::Quiet { minutes: 2 }));
        // The count is the figure again, with the refused name in it.
        assert_eq!(rig.panel.shown.count, Some(4_780));

        // Days of nothing, in one late turn: a frame, the new words, and no overflow.
        let days = now + 3 * 86_400_000;
        rig.turn(days);
        rig.turn(days + STILL_OFFSET_MS);
        assert_eq!(
            rig.panel.shown.words.map(LaneWords::text).as_deref(),
            Some("QUIET 3 D")
        );
    }

    #[test]
    fn the_percent_forms_reach_the_screen() {
        let source = Changing::default();
        source.set(reference_snapshot());
        let mut rig = Rig::new(Arc::new(source.clone()));
        let mut now = 0;
        rig.drive(&mut now, 500);
        for (blocked, queries, share, text) in [
            (0, 3, Share::Zero, "0%"),
            (1, 1_000, Share::UnderOne, "<1%"),
            (4_780, 10_818, Share::Percent(44), "44%"),
            (999, 1_000, Share::Percent(99), "99%"),
            (5, 5, Share::All, "ALL"),
        ] {
            source.set(counted(blocked, queries));
            let until = now + 2_500;
            rig.drive(&mut now, until);
            assert_eq!(rig.panel.shown.percent, Some(share));
            assert_eq!(share.text(), text);
            let stills = Stills {
                percent: Some(share),
                clock: TEN_TWENTY,
                count: u32::try_from(blocked).ok(),
                ..Stills::default()
            };
            assert!(
                rig.frames.last() == screen_of(u32::try_from(blocked).ok(), &stills),
                "{text}"
            );
        }
    }

    #[test]
    fn the_gate_refills_as_the_pause_runs_out_in_writes_of_its_own() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        // Each write's rows and which figure it carried: the countdown, the refill or the words.
        let mut writes = Vec::new();
        let mut previous = Vec::new();
        let (mut countdown_writes, mut refill_writes, mut words_writes) = (0, 0, 0);
        let mut fills = Vec::new();
        // A pause of 300 s on the control's clock, from the panel's first frame to the last second.
        while now < 299_000 {
            rig.control
                .set_left(Some(300 - u16::try_from(now / 1_000).unwrap()));
            let (blocking, fill, words) = (
                rig.panel.shown.blocking,
                rig.panel.shown.gate_fill,
                rig.panel.shown.words,
            );
            let wrote = rig.frames.count();
            let wait = rig.turn(now);
            let moved = (
                rig.panel.shown.blocking != blocking,
                rig.panel.shown.gate_fill != fill,
                rig.panel.shown.words != words,
            );
            assert!(
                usize::from(moved.0) + usize::from(moved.1) + usize::from(moved.2) <= 1,
                "two figures changed in the write at {now}"
            );
            countdown_writes += usize::from(moved.0);
            words_writes += usize::from(moved.2);
            if moved.1 {
                refill_writes += 1;
                fills.push(rig.panel.shown.gate_fill);
            }
            if rig.frames.count() > wrote {
                let frame = rig.frames.last();
                let rows = (!previous.is_empty()).then(|| {
                    changed_rows(&previous, &frame, WIDTH * 2).map_or(0, |rows| rows.len())
                });
                previous = frame;
                writes.push((rows, moved));
            }
            now += wait.max(1);
        }
        // 18 steps of 12 pixels, one to a write, each after the countdown's write of its second.
        assert_eq!(fills, (1..=18).collect::<Vec<u8>>());
        assert_eq!(refill_writes, 18);
        // The countdown: the first frame's, and one write for every second that passed after it.
        assert_eq!(countdown_writes, 1 + 298);
        let rows: Vec<usize> = writes.iter().filter_map(|(rows, _)| *rows).collect();
        let refill_rows: Vec<usize> = writes
            .iter()
            .filter(|(_, moved)| moved.1)
            .filter_map(|(rows, _)| *rows)
            .collect();
        eprintln!(
            "rows per write, a pause running out: {} writes, peak {}, the countdown's peak {}, the refill's {:?}",
            rows.len(),
            rows.iter().max().unwrap(),
            writes
                .iter()
                .filter(|(_, moved)| moved.0)
                .filter_map(|(rows, _)| *rows)
                .max()
                .unwrap(),
            refill_rows
        );
        assert!(rows.iter().all(|&rows| rows <= 80), "{rows:?}");
        assert_eq!(refill_rows.len(), 18);
        // The lane's quiet words came at two, three and four minutes, each in a write of its own.
        assert_eq!(words_writes, 3);
        // Every write was one figure's: nothing else moved.
        assert_eq!(
            writes.len(),
            countdown_writes + refill_writes + words_writes
        );
    }

    /// Where a tap works HOME's tiles and the lane's `HOME`.
    const HOME_DEVICES: (u16, u16) = (237, 130);
    const HOME_ALERTS: (u16, u16) = (394, 130);
    const HOME_GATE: (u16, u16) = (80, 246);
    const LANE_HOME: (u16, u16) = (50, 30);
    /// The first place of the wall of devices, and the third.
    const WALL_FIRST: (u16, u16) = (63, 115);
    const WALL_THIRD: (u16, u16) = (300, 115);
    /// The floor's left key, and its right one; and the four keys of the gate's choices (the first keeps).
    const FLOOR_LEFT: (u16, u16) = (100, 280);
    const FLOOR_RIGHT: (u16, u16) = (360, 280);
    const CHOICE_5_MIN: (u16, u16) = (180, 280);
    const CHOICE_30_MIN: (u16, u16) = (300, 280);

    /// What screen the panel shows, for a test to say where a tap took it.
    fn at_screen(rig: &Rig) -> (Section, Option<&Opened>) {
        (rig.panel.nav.root(), rig.panel.nav.top())
    }

    /// Taps through the screens until every kind of intent of the first phase has been carried out: the
    /// gate opened and shut, a device's ads blocked, let through, and an edit that is refused, a device's
    /// own gate opened and shut, an alert dismissed. The rig is at HOME with `house()`, one alert, and the
    /// panel's thread at `*now`.
    fn walk_through_every_intent(rig: &mut Rig, now: &mut u64) {
        let desk = hw(12);
        let until = *now + 300;
        rig.drive(now, until);
        assert_eq!(at_screen(rig), (Section::Home, None));
        // The main gate: HOME's key, the choices, a half hour; then the key again, which shuts it.
        rig.tap_at(now, HOME_GATE.0, HOME_GATE.1);
        assert_eq!(rig.panel.nav.root(), Section::Gate);
        rig.tap_at(now, CHOICE_30_MIN.0, CHOICE_30_MIN.1);
        assert_eq!(at_screen(rig), (Section::Home, None));
        assert_eq!(rig.control.calls(), ["pause 1800"]);
        assert_eq!(
            rig.panel.world.blocking,
            Some(Pause {
                left_secs: 1_800,
                total_secs: 1_800
            })
        );
        rig.tap_at(now, HOME_GATE.0, HOME_GATE.1);
        assert_eq!(rig.control.calls(), ["pause 1800", "resume"]);

        // The wall, the desk PC (ads pass), and the device's own screen.
        rig.tap_at(now, HOME_DEVICES.0, HOME_DEVICES.1);
        assert_eq!(at_screen(rig), (Section::Devices, None));
        rig.tap_at(now, WALL_FIRST.0, WALL_FIRST.1);
        assert_eq!(
            at_screen(rig),
            (Section::Devices, Some(&Opened::Device(desk)))
        );
        // Block its ads: the key, the question, its answer.
        rig.tap_at(now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert_eq!(
            at_screen(rig),
            (
                Section::Devices,
                Some(&Opened::Confirm {
                    mac: desk,
                    pass: false
                })
            )
        );
        rig.tap_at(now, FLOOR_RIGHT.0, FLOOR_RIGHT.1);
        assert_eq!(rig.devices.state().edits, ["ads 12 false"]);
        // The inventory is read again at once, and the question that was answered is not asked again.
        let until = *now + FRAME_MS;
        rig.drive(now, until);
        assert_eq!(
            at_screen(rig),
            (Section::Devices, Some(&Opened::Device(desk)))
        );
        // Its own gate: the key, five minutes; then the key again, which shuts it.
        rig.tap_at(now, FLOOR_RIGHT.0, FLOOR_RIGHT.1);
        assert_eq!(
            at_screen(rig),
            (Section::Devices, Some(&Opened::ItsGate(desk)))
        );
        rig.tap_at(now, CHOICE_5_MIN.0, CHOICE_5_MIN.1);
        assert_eq!(rig.devices.state().edits, ["ads 12 false", "gate 12 300"]);
        let until = *now + FRAME_MS;
        rig.drive(now, until);
        assert_eq!(
            at_screen(rig),
            (Section::Devices, Some(&Opened::Device(desk)))
        );
        rig.tap_at(now, FLOOR_RIGHT.0, FLOOR_RIGHT.1);
        assert_eq!(
            rig.devices.state().edits,
            ["ads 12 false", "gate 12 300", "gate 12 0"]
        );
        let until = *now + FRAME_MS;
        rig.drive(now, until);
        // Let its ads pass again: the key, the question, its answer.
        rig.tap_at(now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert_eq!(
            at_screen(rig),
            (
                Section::Devices,
                Some(&Opened::Confirm {
                    mac: desk,
                    pass: true
                })
            )
        );
        rig.tap_at(now, FLOOR_RIGHT.0, FLOOR_RIGHT.1);
        assert_eq!(rig.devices.state().edits.last().unwrap(), "ads 12 true");
        let until = *now + FRAME_MS;
        rig.drive(now, until);

        // An alert: from HOME, its screen, and its dismissal.
        rig.tap_at(now, LANE_HOME.0, LANE_HOME.1);
        assert_eq!(at_screen(rig), (Section::Home, None));
        rig.tap_at(now, HOME_ALERTS.0, HOME_ALERTS.1);
        assert_eq!(at_screen(rig), (Section::Alerts, Some(&Opened::Alert(7))));
        rig.tap_at(now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert_eq!(at_screen(rig), (Section::Home, None));
        assert_eq!(rig.alerts.acks(), [7]);

        // An edit that the inventory refuses, because the device is gone from it between the panel's last
        // look and the tap: nothing changes, and the panel says so without saying whom.
        rig.tap_at(now, HOME_DEVICES.0, HOME_DEVICES.1);
        rig.tap_at(now, WALL_THIRD.0, WALL_THIRD.1);
        let tv = hw(31);
        assert_eq!(
            at_screen(rig),
            (Section::Devices, Some(&Opened::Device(tv)))
        );
        rig.devices
            .edit(|view| view.devices.retain(|device| device.mac.0 != tv));
        rig.tap_at(now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        let edits = rig.devices.state().edits.len();
        rig.panel.touch(&tap_on(*now, FLOOR_RIGHT.0, FLOOR_RIGHT.1));
        assert_eq!(
            rig.devices.state().edits.len(),
            edits,
            "nothing was changed"
        );
    }

    /// HOME's LIVE tile, and the lane's words between `HOME` and the clock.
    const HOME_LIVE: (u16, u16) = (80, 130);
    const LANE_WORDS: (u16, u16) = (200, 30);

    /// The frame that `screens` paints for where the panel is and what its world holds.
    fn painted(panel: &Panel<Recorder>) -> Vec<u8> {
        let view = screens::view_of(&panel.nav, &panel.world).expect("a screen with a view");
        let mut painter = ScreenPainter::new();
        painter.paint(view, screens::draw);
        painter.frame().to_vec()
    }

    /// The rows of the frames that were written, each from the one before it; the first has none.
    fn rows_of(frames: &Frames) -> Vec<usize> {
        let all = frames.all();
        all.windows(2)
            .map(|pair| changed_rows(&pair[0], &pair[1], WIDTH * 2).map_or(0, |rows| rows.len()))
            .collect()
    }

    #[test]
    fn a_boot_begins_at_home_which_is_written_whole_in_its_first_frame() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        rig.turn(0);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.frames.count(), 1);
        assert!(rig.frames.last() == painted(&rig.panel));
        // It is what the world says: 44% of 10,818 refused, 10:20, 212K names are not known to the
        // snapshot of these tests, the house's six places and a lamp on none.
        let world = &rig.panel.world;
        assert_eq!(world.share, Some(Share::Percent(44)));
        assert_eq!(world.queries_today, Some(10_818));
        assert_eq!(
            world.time,
            Some(TimeOfDay {
                hour: 10,
                minute: 20
            })
        );
        assert!(world.wifi_joined);
        assert_eq!(world.refused_today, Some(4_780));
        assert_eq!(world.devices.len(), 6);
        assert_eq!(world.version, crate::VERSION);
        assert_eq!(world.uptime_days, Some(0));
        assert_eq!(world.newcomer, Newcomer::Blocked);
        assert_eq!(world.blocking, None);
        assert_eq!((world.alerts_waiting, world.top_alert.is_none()), (0, true));
        // The wall is the labelled devices in their order, then the others, then the router.
        let names: Vec<&str> = world.devices.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Desk PC",
                "Sam's phone",
                "Living room TV",
                "printer",
                "\u{b7}143",
                "\u{b7}1"
            ]
        );
    }

    #[test]
    fn a_home_with_nothing_to_move_paints_nothing_after_its_first_frame() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        let writes = rig.drive(&mut now, 20_000);
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert!(now > 20_000);
    }

    #[test]
    fn the_wall_and_the_device_are_read_only_while_their_screens_show() {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        // LIVE: no device is read at all, and the alerts are asked for once a second.
        rig.drive(&mut now, 10_000);
        assert_eq!(rig.devices.reads(), (0, 0, 0));
        let asked = rig.alerts.asked();
        assert!((10..=11).contains(&asked), "{asked}");

        // HOME: the inventory every two seconds, who asked every second.
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        let (views, askings, watched) = rig.devices.reads();
        assert_eq!(
            (views, askings, watched),
            (1, 1, 0),
            "read at once on arriving"
        );
        let from = now;
        rig.drive(&mut now, from + 10_000 - 1);
        let (views, askings, watched) = rig.devices.reads();
        assert_eq!(
            (views, askings, watched),
            (6, 11, 0),
            "five more reads in ten seconds, ten more of who asked"
        );

        // DEVICES: the same cadences; no more at once, as HOME read them.
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        assert_eq!(at_screen(&rig), (Section::Devices, None));
        let before = rig.devices.reads();
        rig.run_for(&mut now, 10_000);
        let after = rig.devices.reads();
        assert!(
            (5..=6).contains(&(after.0 - before.0)),
            "{before:?} {after:?}"
        );
        assert!(
            (10..=11).contains(&(after.1 - before.1)),
            "{before:?} {after:?}"
        );
        assert_eq!(after.2, 0);

        // DEVICE: the inventory, never who asked, and the device's names every frame.
        rig.tap_at(&mut now, WALL_FIRST.0, WALL_FIRST.1);
        assert_eq!(
            at_screen(&rig)
                .1
                .map(|screen| matches!(screen, Opened::Device(_))),
            Some(true)
        );
        let before = rig.devices.reads();
        rig.run_for(&mut now, 10_000);
        let after = rig.devices.reads();
        assert!(
            (5..=6).contains(&(after.0 - before.0)),
            "{before:?} {after:?}"
        );
        assert_eq!(after.1, before.1, "nobody is asked who asked on DEVICE");
        assert!(
            (79..=81).contains(&(after.2 - before.2)),
            "{before:?} {after:?}"
        );

        // CONFIRM and ITS GATE over it read the inventory and nothing else.
        rig.tap_at(&mut now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert!(matches!(at_screen(&rig).1, Some(Opened::Confirm { .. })));
        let before = rig.devices.reads();
        rig.run_for(&mut now, 6_000);
        let after = rig.devices.reads();
        assert!(
            (3..=4).contains(&(after.0 - before.0)),
            "{before:?} {after:?}"
        );
        assert_eq!((after.1, after.2), (before.1, before.2));

        // ALERT, GATE and SYSTEM read no device at all.
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        rig.alerts.raise(threat_alert(3));
        rig.run_for(&mut now, 1_500);
        rig.tap_at(&mut now, HOME_ALERTS.0, HOME_ALERTS.1);
        assert!(matches!(at_screen(&rig).1, Some(Opened::Alert(3))));
        let before = rig.devices.reads();
        rig.run_for(&mut now, 6_000);
        assert_eq!(rig.devices.reads(), before);
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        rig.tap_at(&mut now, HOME_GATE.0, HOME_GATE.1);
        assert_eq!(rig.panel.nav.root(), Section::Gate);
        let before = rig.devices.reads();
        rig.run_for(&mut now, 6_000);
        assert_eq!(rig.devices.reads(), before);
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        rig.tap_at(&mut now, 394, 246);
        assert_eq!(rig.panel.nav.root(), Section::System);
        let before = rig.devices.reads();
        rig.run_for(&mut now, 6_000);
        assert_eq!(rig.devices.reads(), before);
    }

    #[test]
    fn a_device_screen_watches_its_device_from_when_it_opens_until_it_closes() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let desk = Some(Ipv4Addr::new(192, 0, 2, 12));
        let mut now = 0;
        rig.drive(&mut now, 300);
        assert!(rig.devices.state().watches.is_empty());
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        assert!(
            rig.devices.state().watches.is_empty(),
            "the wall watches nobody"
        );
        rig.tap_at(&mut now, WALL_FIRST.0, WALL_FIRST.1);
        assert_eq!(rig.devices.state().watches, [desk]);
        // The names it asks come to its screen, newest first, fitted.
        rig.devices.state().watched = vec![
            LaneWatched {
                name: "ads.example".to_owned(),
                outcome: WatchedOutcome::Refused,
            },
            LaneWatched {
                name: "example.org".to_owned(),
                outcome: WatchedOutcome::Answered,
            },
        ];
        let until = now + FRAME_MS * 2;
        rig.drive(&mut now, until);
        let names: Vec<(&str, Outcome)> = rig
            .panel
            .world
            .watched
            .iter()
            .map(|name| (name.name.as_str(), name.outcome))
            .collect();
        assert_eq!(
            names,
            [
                ("ads.example", Outcome::Refused),
                ("example.org", Outcome::Answered)
            ]
        );
        // Nothing but the watch of the same device has been asked for since.
        assert_eq!(rig.devices.state().watches, [desk]);

        // A question over the device's screen stops the watch, and the screen back starts it again.
        rig.tap_at(&mut now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert!(matches!(at_screen(&rig).1, Some(Opened::Confirm { .. })));
        assert_eq!(rig.devices.state().watches, [desk, None]);
        assert!(
            rig.panel.world.watched.is_empty(),
            "what was kept goes with the watch"
        );
        rig.tap_at(&mut now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert!(matches!(at_screen(&rig).1, Some(Opened::Device(_))));
        assert_eq!(rig.devices.state().watches, [desk, None, desk]);

        // The lane's HOME closes it.
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.devices.state().watches, [desk, None, desk, None]);
    }

    /// A panel on a surface that can be broken from outside, with a device that is watched.
    struct Switch(Arc<std::sync::atomic::AtomicBool>);

    impl Surface for Switch {
        fn show(&mut self, _frame: &[u8]) -> io::Result<()> {
            if self.0.load(std::sync::atomic::Ordering::Relaxed) {
                Err(io::Error::other("the screen is gone"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn a_new_panel_lets_go_of_a_watch_that_a_panel_before_it_left_on() {
        let desk = Ipv4Addr::new(192, 0, 2, 12);
        let devices = FakeDevices::with(house());
        // The panel before this one failed with DEVICE open, and the lane feed went on keeping the names
        // of the desk.
        devices.watch_client(Some(desk));
        assert_eq!(devices.state().watching, Some(desk));
        let _panel = Panel::new(
            recorder().0,
            Deps {
                devices: Arc::new(devices.clone()),
                ..deps(Arc::new(Nothing))
            },
        );
        assert_eq!(devices.state().watching, None);
        assert_eq!(devices.state().watches, [Some(desk), None]);
    }

    #[test]
    fn a_panel_that_ends_lets_go_of_the_watch_whatever_its_end() {
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let devices = FakeDevices::with(house());
        let mut panel = Panel::new(
            Switch(broken.clone()),
            Deps {
                devices: Arc::new(devices.clone()),
                ..deps(Arc::new(Fixed(reference_snapshot())))
            },
        );
        // DEVICE of the desk opens: its names are watched.
        let tap_at = |panel: &mut Panel<Switch>, now: u64, x: u16, y: u16| {
            panel.touch(&tap(now, 60, raw_of(x, y)));
            panel.turn(now + 100, UNIX).unwrap();
        };
        panel.turn(0, UNIX).unwrap();
        tap_at(&mut panel, 300, HOME_DEVICES.0, HOME_DEVICES.1);
        tap_at(&mut panel, 600, WALL_FIRST.0, WALL_FIRST.1);
        let desk = Some(Ipv4Addr::new(192, 0, 2, 12));
        assert_eq!(devices.state().watching, desk);

        // The screen breaks: `serve` returns the failure, and the watch is let go with it.
        broken.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(
            serve(&mut panel, None::<std::io::PipeReader>, Instant::now()),
            PanelEnd::Failed(_)
        ));
        assert_eq!(devices.state().watching, None);
        assert_eq!(devices.state().watches.last(), Some(&None));
    }

    #[test]
    fn the_watch_is_cleared_when_the_idle_minute_takes_the_panel_home_and_when_the_device_goes() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let (desk, tv) = (
            Some(Ipv4Addr::new(192, 0, 2, 12)),
            Some(Ipv4Addr::new(192, 0, 2, 31)),
        );
        let mut now = 0;
        rig.drive(&mut now, 300);
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        rig.tap_at(&mut now, WALL_FIRST.0, WALL_FIRST.1);
        assert_eq!(rig.devices.state().watches, [desk]);
        // A minute without a touch: HOME, and the watch is let go in the same turn.
        let touched = now;
        rig.drive(&mut now, touched + 59_000);
        assert!(matches!(at_screen(&rig).1, Some(Opened::Device(_))));
        rig.drive(&mut now, touched + 61_000);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.devices.state().watches, [desk, None]);

        // The living room TV: it goes from the inventory while its screen shows.
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        rig.tap_at(&mut now, WALL_THIRD.0, WALL_THIRD.1);
        assert_eq!(rig.devices.state().watches, [desk, None, tv]);
        rig.devices
            .edit(|view| view.devices.retain(|device| device.mac.0 != hw(31)));
        let until = now + 2_500;
        rig.drive(&mut now, until);
        // The next read finds it gone, and the screen goes back to the wall, which shows no such place.
        assert_eq!(at_screen(&rig), (Section::Devices, None));
        assert_eq!(rig.devices.state().watches, [desk, None, tv, None]);
        assert!(rig.panel.world.device(&hw(31)).is_none());
        assert_eq!(rig.panel.world.devices.len(), 5);
    }

    #[test]
    fn a_confirm_that_is_done_or_no_longer_true_is_popped_to_the_device_under_it() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        rig.tap_at(&mut now, WALL_FIRST.0, WALL_FIRST.1);
        rig.tap_at(&mut now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        let desk = hw(12);
        assert_eq!(
            at_screen(&rig).1,
            Some(&Opened::Confirm {
                mac: desk,
                pass: false
            })
        );
        // The desktop blocked its ads in the meantime: the question has no answer to give.
        rig.devices.edit(|view| view.devices[0].exempt = false);
        let until = now + 2_500;
        rig.drive(&mut now, until);
        assert_eq!(at_screen(&rig).1, Some(&Opened::Device(desk)));
    }

    #[test]
    fn a_minute_without_a_touch_takes_the_panel_home_in_one_write_and_a_touch_starts_the_minute_again()
     {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        let touched = now;
        // A press that is no tap is a touch all the same: it starts the minute again at 50 s.
        rig.drive(&mut now, touched + 50_000);
        rig.panel.touch(&tap(1, 10, ELSEWHERE));
        let until = touched + 100_000;
        rig.drive(&mut now, until);
        assert_eq!(at_screen(&rig), (Section::Devices, None), "{now} ms");
        let writes = rig.frames.count();
        let until = touched + 112_000;
        rig.drive(&mut now, until);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        // HOME was written whole once, and nothing else was written.
        assert_eq!(rig.frames.count(), writes + 1);
        assert!(rig.frames.last() == painted(&rig.panel));
        // At HOME it stays, writing nothing, for as long as nobody touches.
        let until = touched + 400_000;
        let more = rig.drive(&mut now, until);
        assert!(more.is_empty(), "{more:?}");
    }

    #[test]
    fn a_switch_of_screens_is_one_full_write_of_the_screen_and_leaving_live_and_coming_back_is_too()
    {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        rig.refuse("ads.example.org");
        let mut now = 0;
        rig.drive(&mut now, 1_000);
        let live = rig.frames.count();
        // LIVE to HOME: one write, which is the whole of HOME.
        rig.tap_at(&mut now, LANE_HOME.0, LANE_HOME.1);
        assert_eq!(rig.frames.count(), live + 1);
        assert!(rig.frames.last() == painted(&rig.panel));
        let rows = rows_of(&rig.frames);
        assert!(*rows.last().unwrap() > 200, "{rows:?}");
        // The Drain ran on behind it: names still go through it and the figures follow the snapshot.
        rig.refuse("tracker.example");
        let until = now + 3_000;
        rig.drive(&mut now, until);
        // HOME to LIVE: the Drain as it is now, written whole, and it is what a Drain painted from this
        // state gives.
        rig.tap_at(&mut now, HOME_LIVE.0, HOME_LIVE.1);
        assert_eq!(at_screen(&rig), (Section::Live, None));
        let shown = rig.panel.shown.clone();
        assert_eq!(shown, rig.panel.wanted);
        assert!(rig.frames.last() == Painter::new().paint(&rig.panel.drain, &shown));
        // And from there it goes on in motion writes of its own, each within the budget.
        let until = now + 6_000;
        let writes = rig.drive(&mut now, until);
        assert!(writes.len() > 10);
        assert!(
            writes
                .iter()
                .all(|write| write.rows.is_none_or(|rows| rows <= 80)),
            "{writes:?}"
        );
    }

    #[test]
    fn the_gates_toggle_is_one_full_write_and_its_countdown_after_it_is_one_band_a_second() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 1_000);
        let before = rig.frames.count();
        // Someone opens the gate for half an hour from the desktop.
        rig.control.set_left(Some(1_800));
        let until = now + 500;
        rig.drive(&mut now, until);
        assert_eq!(rig.frames.count(), before + 1);
        assert!(rig.frames.last() == painted(&rig.panel));
        assert_eq!(
            rig.panel.world.blocking,
            Some(Pause {
                left_secs: 1_800,
                total_secs: 1_800
            })
        );
        // The lane and the gate's tile changed together: over a hundred rows, written at once.
        assert!(*rows_of(&rig.frames).last().unwrap() > 80);
        // Every second after it, the figure of the tile: one write of its own.
        let mark = rig.frames.count();
        for left in (1_790..1_800).rev() {
            rig.control.set_left(Some(left));
            let until = now + 1_000;
            rig.drive(&mut now, until);
        }
        let rows: Vec<usize> = rows_of(&rig.frames).split_off(mark - 1);
        assert_eq!(rows.len(), 10, "{rows:?}");
        assert!(
            rows.iter().all(|&rows| (1..=80).contains(&rows)),
            "{rows:?}"
        );
        // The pause ends by itself: the toggle back is one full write again.
        let before = rig.frames.count();
        rig.control.set_left(None);
        let until = now + 500;
        rig.drive(&mut now, until);
        assert_eq!(rig.frames.count(), before + 1);
        assert_eq!(rig.panel.world.blocking, None);
        assert!(rig.frames.last() == painted(&rig.panel));
    }

    #[test]
    fn the_gates_total_is_what_it_was_opened_for_and_a_pause_from_elsewhere_is_taken_for_its_length()
     {
        let mut rig = Rig::new(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        // Opened on the panel for half an hour: 900 s left is half of it.
        rig.panel.touch(&tap_on(now, KEY.0, KEY.1));
        let until = now + 300;
        rig.drive(&mut now, until);
        rig.panel.touch(&tap_on(now, ASK_30_MIN.0, ASK_30_MIN.1));
        assert_eq!(rig.control.calls(), ["pause 1800"]);
        for (left, fill) in [(1_800, 0), (900, 9), (180, 17), (1, 18)] {
            rig.control.set_left(Some(left));
            let until = now + 300;
            rig.drive(&mut now, until);
            assert_eq!(rig.panel.wanted.gate_fill, fill, "{left} s left");
            assert_eq!(rig.panel.gate_total, Some(1_800));
        }
        // It ends, and what is remembered goes with it.
        rig.control.set_left(None);
        let until = now + 300;
        rig.drive(&mut now, until);
        assert_eq!(rig.panel.gate_total, None);
        // A pause the API made, with 1,000 s left, is taken for half an hour; as it runs down the total
        // holds, so the gate does not jump.
        rig.control.set_left(Some(1_000));
        let until = now + 300;
        rig.drive(&mut now, until);
        assert_eq!(
            (rig.panel.gate_total, rig.panel.wanted.gate_fill),
            (Some(1_800), 8)
        );
        rig.control.set_left(Some(250));
        let until = now + 300;
        rig.drive(&mut now, until);
        assert_eq!(
            (rig.panel.gate_total, rig.panel.wanted.gate_fill),
            (Some(1_800), 16)
        );
        rig.control.set_left(None);
        let until = now + 300;
        rig.drive(&mut now, until);
        // A short one, which came from elsewhere, is five minutes.
        rig.control.set_left(Some(250));
        let until = now + 300;
        rig.drive(&mut now, until);
        assert_eq!(
            (rig.panel.gate_total, rig.panel.wanted.gate_fill),
            (Some(300), 3)
        );
    }

    #[test]
    fn homes_newest_is_when_the_alert_last_happened_not_when_it_first_did() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        assert_eq!(rig.panel.world.alert_newest, None);
        // The threat first came at 10:03 and came again at 10:19; the clock says 10:20.
        rig.alerts.raise(threat_alert(7));
        let until = now + 1_300;
        rig.drive(&mut now, until);
        assert_eq!(
            rig.panel.world.alert_newest,
            Some(TimeOfDay {
                hour: 10,
                minute: 19
            })
        );
        // An alert that last happened before today is no time of day, however fresh its first sighting.
        rig.alerts.api_ack(7);
        rig.alerts.raise(PanelAlert {
            first_unix: UNIX - 90_000,
            last_unix: UNIX - 86_400,
            ..threat_alert(9)
        });
        let until = now + 1_300;
        rig.drive(&mut now, until);
        assert_eq!(rig.panel.world.alert_newest, None);
        assert_eq!(rig.panel.world.alerts_waiting, 1);
    }

    #[test]
    fn an_alert_lights_home_and_its_tile_and_the_lanes_words_open_it_and_dismiss_acks_it() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        // With none waiting the tile is inert.
        rig.tap_at(&mut now, HOME_ALERTS.0, HOME_ALERTS.1);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.panel.world.alerts_waiting, 0);

        rig.alerts.raise(threat_alert(7));
        rig.alerts.raise(PanelAlert {
            severity: Severity::Info,
            ..threat_alert(8)
        });
        let until = now + 1_300;
        let writes = rig.drive(&mut now, until);
        // Both wait; the high one is the one to open; the tile and the lane say so, with no screen of its own.
        assert_eq!(rig.panel.world.alerts_waiting, 2);
        assert_eq!(
            rig.panel.world.top_alert.as_ref().map(|alert| alert.id),
            Some(7)
        );
        assert!(rig.panel.world.alert_newest.is_some());
        assert_eq!(at_screen(&rig), (Section::Home, None));
        // The tile that turned white and the lane's words are written in bands, none of them over 80 rows.
        assert!(!writes.is_empty());
        assert!(
            writes
                .iter()
                .all(|write| write.rows.is_none_or(|rows| rows <= 80)),
            "{writes:?}"
        );
        assert!(rig.frames.last() == painted(&rig.panel));

        // The lane's words open it too.
        rig.tap_at(&mut now, LANE_WORDS.0, LANE_WORDS.1);
        assert_eq!(at_screen(&rig), (Section::Alerts, Some(&Opened::Alert(7))));
        let row = rig.panel.world.top_alert.clone().unwrap();
        assert_eq!(row.title, "THREAT BLOCKED");
        assert_eq!(row.lines[0], "secure-login-verify.example");
        // Sam's phone is on the wall that HOME read: its name, not its address.
        assert_eq!(row.lines[1], "Sam's phone asked 3 times by 10:19");
        assert!(rig.frames.last() == painted(&rig.panel));
        // Dismissed: acked in the store, back at HOME, and the other waits.
        rig.tap_at(&mut now, FLOOR_LEFT.0, FLOOR_LEFT.1);
        assert_eq!(rig.alerts.acks(), [7]);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.panel.world.alerts_waiting, 1);
        assert_eq!(
            rig.panel.world.top_alert.as_ref().map(|alert| alert.id),
            Some(8)
        );
    }

    #[test]
    fn the_alert_screen_goes_home_when_its_alert_is_no_longer_waiting_or_is_not_the_top() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        rig.alerts.raise(threat_alert(7));
        let mut now = 0;
        rig.drive(&mut now, 1_300);
        rig.tap_at(&mut now, HOME_ALERTS.0, HOME_ALERTS.1);
        assert_eq!(at_screen(&rig), (Section::Alerts, Some(&Opened::Alert(7))));
        // The desktop acked it: within a second the screen is gone.
        rig.alerts.api_ack(7);
        let until = now + 1_300;
        rig.drive(&mut now, until);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.panel.world.alerts_waiting, 0);
        assert!(rig.frames.last() == painted(&rig.panel));
        assert!(rig.alerts.acks().is_empty(), "the panel acked nothing");

        // A newer high alert comes while one is open: the world holds only the top, and the screen
        // that was about the other goes HOME, where the tile says there is one waiting.
        rig.alerts.raise(threat_alert(9));
        let until = now + 1_300;
        rig.drive(&mut now, until);
        rig.tap_at(&mut now, HOME_ALERTS.0, HOME_ALERTS.1);
        assert_eq!(at_screen(&rig), (Section::Alerts, Some(&Opened::Alert(9))));
        rig.alerts.raise(threat_alert(10));
        let until = now + 1_300;
        rig.drive(&mut now, until);
        assert_eq!(at_screen(&rig), (Section::Home, None));
        assert_eq!(rig.panel.world.alerts_waiting, 2);
    }

    #[test]
    fn open_gates_count_down_every_second_between_two_reads_of_the_inventory() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        rig.devices
            .edit(|view| view.devices[2].gate_left_secs = Some(60));
        let mut now = 0;
        rig.drive(&mut now, 300);
        rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
        let tv = |rig: &Rig| rig.panel.world.device(&hw(31)).unwrap().state;
        assert_eq!(tv(&rig), DeviceState::Open { left_secs: 60 });
        let (reads, from) = (rig.devices.reads().0, now);
        // The inventory's own gate runs down in whole seconds, as the real one does; the panel reads it
        // every two seconds and counts down by itself in between.
        let mut seen = Vec::new();
        while now < from + 9_000 {
            let left = 60 - u16::try_from((now - from) / 1_000).unwrap();
            rig.devices
                .edit(|view| view.devices[2].gate_left_secs = Some(left));
            rig.turn(now);
            if let DeviceState::Open { left_secs } = tv(&rig)
                && seen.last() != Some(&left_secs)
            {
                seen.push(left_secs);
            }
            now += FRAME_MS;
        }
        // Every second a step, though the inventory was read only four or five times in nine seconds, and
        // never a step back up.
        assert!(seen.len() >= 9, "{seen:?}");
        assert!(seen.windows(2).all(|pair| pair[1] < pair[0]), "{seen:?}");
        assert!(rig.devices.reads().0 - reads <= 5);
    }

    #[test]
    fn home_and_the_wall_write_at_most_eighty_rows_at_a_time_over_two_and_a_half_minutes_of_traffic()
     {
        for wall in [false, true] {
            let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
            // The idle minute is another test's: here the panel stays where it is.
            rig.panel.idle_after_ms = u64::MAX;
            let mut now = 0;
            rig.drive(&mut now, 300);
            if wall {
                rig.tap_at(&mut now, HOME_DEVICES.0, HOME_DEVICES.1);
            }
            // Lamps, a device's gate counting down, the clock's minute turning, the lane's beat, and a
            // gate that the desktop opens for a minute and shuts.
            rig.devices
                .edit(|view| view.devices[1].gate_left_secs = Some(300));
            let begin = now;
            let (mut toggles, mut turns) = (0, Vec::new());
            while now < begin + 150_000 {
                let second = (now - begin) / 1_000;
                rig.devices.state().asking = [12, 23, 31, 57, 143]
                    .into_iter()
                    .filter(|octet| (second + u64::from(*octet)) % 3 == 0)
                    .map(|octet| Ipv4Addr::new(192, 0, 2, octet))
                    .collect();
                let left = (60..120)
                    .contains(&second)
                    .then(|| u16::try_from(120 - second).unwrap());
                toggles += usize::from(rig.control.paused_left_secs().is_some() != left.is_some());
                rig.control.set_left(left);
                if (now - begin) % 1_000 < FRAME_MS {
                    rig.lane.observe("ads.example", true, second, |_| false);
                }
                let before = rig.frames.count();
                let wait = rig
                    .panel
                    .turn(now, UNIX + 20 + (now - begin) / 1_000)
                    .unwrap();
                turns.push((now - begin, before, rig.frames.count()));
                now += wait.max(1);
            }
            // Every write, from the frame before it: the rows from the first that differs to the last.
            let all = rig.frames.all();
            let mut rows: Vec<(u64, usize)> = Vec::new();
            for &(at, from, to) in &turns {
                for index in from.max(1)..to {
                    let changed = changed_rows(&all[index - 1], &all[index], WIDTH * 2);
                    rows.push((at, changed.map_or(0, |rows| rows.len())));
                }
            }
            // The gate's two toggles are two full writes; every other write fits.
            let big: Vec<&(u64, usize)> = rows.iter().filter(|(_, rows)| *rows > 80).collect();
            let name = if wall { "DEVICES" } else { "HOME" };
            eprintln!(
                "{name}: {} writes, peak {}, {} over 80 ({toggles} toggles): {big:?}",
                rows.len(),
                rows.iter().map(|(_, rows)| *rows).max().unwrap(),
                big.len()
            );
            assert_eq!(toggles, 2);
            if wall {
                // The wall shows nothing of the gate: its toggle writes nothing.
                assert!(big.is_empty(), "{name}: {big:?}");
            } else {
                // HOME's lane words and its gate tile change together, once opening and once shutting.
                assert_eq!(big.len(), 2, "{name}: {big:?}");
                assert!(big.iter().all(|(at, _)| {
                    [60_000, 120_000]
                        .iter()
                        .any(|toggle| at.abs_diff(*toggle) < 200)
                }));
            }
            assert!(rows.len() > 100, "{name}: {} writes", rows.len());
        }
    }

    #[test]
    fn the_lamp_of_the_live_tile_is_white_for_three_frames_after_a_name_goes_by() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        assert!(!rig.panel.world.beat);
        rig.refuse("ads.example.org");
        let mut beats = Vec::new();
        for _ in 0..6 {
            rig.turn(now);
            beats.push(rig.panel.world.beat);
            now += FRAME_MS;
        }
        assert_eq!(beats, [true, true, true, false, false, false]);
    }

    #[test]
    fn the_wall_has_the_devices_that_ask_lit_and_the_lamps_follow_who_asked_every_second() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        let mut now = 0;
        rig.drive(&mut now, 300);
        assert!(!rig.panel.world.any_asking());
        rig.devices.state().asking =
            vec![Ipv4Addr::new(192, 0, 2, 23), Ipv4Addr::new(192, 0, 2, 57)];
        let until = now + 1_300;
        rig.drive(&mut now, until);
        let lamps: Vec<bool> = rig
            .panel
            .world
            .devices
            .iter()
            .map(|row| row.asking)
            .collect();
        assert_eq!(lamps, [false, true, false, true, false, false]);
        rig.devices.state().asking.clear();
        let until = now + 1_300;
        rig.drive(&mut now, until);
        assert!(!rig.panel.world.any_asking());
    }

    /// A logger that keeps what is logged, for the one test that installs it.
    struct Capture(Mutex<Vec<String>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", record.level(), record.args()));
        }

        fn flush(&self) {}
    }

    #[test]
    fn taps_reach_the_fakes_for_every_kind_of_intent() {
        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        rig.alerts.raise(threat_alert(7));
        let mut now = 0;
        walk_through_every_intent(&mut rig, &mut now);
    }

    #[test]
    fn the_panel_never_logs_a_name() {
        crate::testns::run_inside("panel::tests::ns_the_panel_never_logs_a_name");
    }

    #[test]
    #[ignore = "run by the_panel_never_logs_a_name, in a process of its own"]
    fn ns_the_panel_never_logs_a_name() {
        if !crate::testns::inside() {
            return;
        }
        // This process runs this one test, so the logger is its own to set.
        static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));
        log::set_logger(&CAPTURE).unwrap();
        log::set_max_level(log::LevelFilter::Trace);

        let mut rig = Rig::home(Arc::new(Fixed(reference_snapshot())));
        // Refused names, plain names, a name too long for the lane, and a pause.
        // The names stay different after fitting to the lane, so that all of them really go through it:
        // `fit_site` drops host labels whole, so a long host in front of `example.org` would be
        // `example.org` like the plain one.
        let long = format!("{}.example", "x".repeat(200));
        let names = [
            ("ads.example.org", 15),
            ("example.org", 20),
            (long.as_str(), 15),
            ("tracker.example", 15),
        ];
        let fitted: std::collections::HashSet<String> = names
            .iter()
            .map(|&(name, cells)| qb_panel::format::fit_site(name, cells))
            .collect();
        assert_eq!(
            fitted.len(),
            names.len(),
            "the names are different on the lane"
        );
        rig.refuse("ads.example.org");
        rig.lane.observe("example.org", false, 5, |_| false);
        rig.refuse(&long);
        // Devices with labels, hostnames and addresses, one that asks, one whose names are watched, and
        // an alert about a name and a device.
        rig.alerts.raise(threat_alert(7));
        rig.devices.state().asking = vec![Ipv4Addr::new(192, 0, 2, 23)];
        rig.devices.state().watched = vec![LaneWatched {
            name: "tracker.example".to_owned(),
            outcome: WatchedOutcome::Refused,
        }];
        let mut now = 0;
        rig.drive(&mut now, 1_000);
        rig.refuse("tracker.example");
        walk_through_every_intent(&mut rig, &mut now);
        let until = now + 3_000;
        rig.drive(&mut now, until);
        // A gate that someone opens, and that ends by itself.
        rig.control.set_left(Some(1));
        let until = now + 1_000;
        rig.drive(&mut now, until);
        rig.control.set_left(None);
        let until = now + 1_000;
        rig.drive(&mut now, until);

        let lines = CAPTURE.0.lock().unwrap().clone();
        for line in [
            "INFO panel: gate opened for 1800 s",
            "INFO panel: gate shut",
            "INFO panel: device ads blocked",
            "INFO panel: device gate opened for 300 s",
            "INFO panel: device gate shut",
            "INFO panel: device ads pass set",
            "INFO panel: alert dismissed",
            "WARN panel: device ads edit refused: unknown",
            "INFO panel: gate: the pause ended",
        ] {
            assert!(lines.iter().any(|seen| seen == line), "{line}: {lines:?}");
        }
        for line in &lines {
            for secret in [
                "example.org",
                ".example",
                "tracker",
                "xxxx",
                "192.0.2.",
                "02:00:00",
                "2:0:0",
                "Desk PC",
                "Sam",
                "Living",
                "printer",
                "secure-login",
                "doubleclick",
            ] {
                assert!(!line.contains(secret), "{line}");
            }
        }
    }

    /// The bytes of `events` as a device sends them.
    fn wire(events: &[RawEvent]) -> Vec<u8> {
        events.iter().flat_map(|&event| bytes_of(event)).collect()
    }

    #[test]
    fn the_events_of_a_device_work_the_key_until_the_device_goes() {
        let (mut panel, frames, control) = panel();
        let (reader, mut writer) = std::io::pipe().unwrap();
        // The key, then a choice of the gate's, which the first tap unfolded.
        writer
            .write_all(&wire(&tap_on(1_000, KEY.0, KEY.1)))
            .unwrap();
        writer
            .write_all(&wire(&tap_on(2_000, ASK_5_MIN.0, ASK_5_MIN.1)))
            .unwrap();
        drop(writer);
        assert_eq!(
            serve(&mut panel, Some(reader), Instant::now()),
            PanelEnd::Failed("reading the touch device: the touch device gave nothing more".into())
        );
        assert_eq!(control.calls(), ["pause 300"]);
        assert!(frames.last() == frame_of(Blocking::Paused { left_secs: 300 }));
    }

    #[test]
    fn a_screen_that_breaks_ends_the_loop_with_no_touch_device_too() {
        let mut panel = Panel::new(Broken, deps(Arc::new(Nothing)));
        assert_eq!(
            serve(&mut panel, None::<std::io::PipeReader>, Instant::now()),
            PanelEnd::Failed("writing the screen: the screen is gone".into())
        );
    }

    #[test]
    fn a_calibration_boot_shows_the_crosses_and_goes_on_with_what_the_taps_measured() {
        let measured = Calibration {
            swap_xy: false,
            left: 3890,
            right: 210,
            top: 300,
            bottom: 3800,
        };
        let (mut panel, frames, control) = panel();
        panel.deps.calibrate = true;
        let (reader, mut writer) = std::io::pipe().unwrap();
        let mut device = Some(reader);
        thread::scope(|scope| {
            scope.spawn(|| {
                for (n, (x, y)) in calibrate::TARGETS.into_iter().enumerate() {
                    // The owner taps a cross once it is on the screen.
                    while frames.count() <= n {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let tap = press(1_000, raw_at(&measured, x, y), 5);
                    writer.write_all(&wire(&tap)).unwrap();
                }
            });
            assert_eq!(measure_touch(&mut panel, device.as_mut()), Ok(()));
        });
        assert_eq!(
            frames.all(),
            (0..4).map(calibrate::target_frame).collect::<Vec<_>>()
        );
        assert_close(&panel.deps.calibration, &measured);
        // The panel reads the touch screen as it was measured from now on.
        let on_switch = raw_at(&measured, 40, 290);
        panel.touch(&tap(
            2_000,
            60,
            (i32::from(on_switch.raw_x), i32::from(on_switch.raw_y)),
        ));
        assert!(panel.nav.gate_ask(), "the key unfolds the gate's choices");
        assert!(control.calls().is_empty());
    }

    #[test]
    fn the_first_tap_after_a_calibration_boot_works_though_the_kernel_leaves_out_an_axis() {
        let measured = Calibration {
            swap_xy: false,
            left: 3890,
            right: 210,
            top: 300,
            bottom: 3800,
        };
        let (mut panel, frames, control) = panel();
        panel.deps.calibrate = true;
        let (reader, mut writer) = std::io::pipe().unwrap();
        let mut device = Some(reader);
        let mut sent = Reading { raw_x: 0, raw_y: 0 };
        thread::scope(|scope| {
            let owner = scope.spawn(|| {
                for (n, (x, y)) in calibrate::TARGETS.into_iter().enumerate() {
                    while frames.count() <= n {
                        thread::sleep(Duration::from_millis(5));
                    }
                    let tap = kernel_press(1_000, &mut sent, raw_at(&measured, x, y), 5);
                    writer.write_all(&wire(&tap)).unwrap();
                }
            });
            assert_eq!(measure_touch(&mut panel, device.as_mut()), Ok(()));
            owner.join().unwrap();
        });
        assert_close(&panel.deps.calibration, &measured);
        // The switch is at the screen's left edge, as the last cross is: the kernel does not send x again.
        let on_switch = raw_at(&measured, 40, 290);
        assert_eq!(on_switch.raw_x, sent.raw_x);
        writer
            .write_all(&wire(&kernel_press(2_000, &mut sent, on_switch, 3)))
            .unwrap();
        drop(writer);
        assert_eq!(
            serve(&mut panel, device.take(), Instant::now()),
            PanelEnd::Failed("reading the touch device: the touch device gave nothing more".into())
        );
        assert!(panel.nav.gate_ask(), "the key unfolds the gate's choices");
        assert!(control.calls().is_empty());
    }

    #[test]
    fn a_boot_that_does_not_ask_for_a_calibration_shows_no_crosses() {
        let (mut panel, frames, _) = panel();
        let (mut reader, _writer) = std::io::pipe().unwrap();
        assert_eq!(measure_touch(&mut panel, Some(&mut reader)), Ok(()));
        assert_eq!(frames.count(), 0);
        assert_eq!(panel.deps.calibration, Calibration::NOMINAL);
    }

    #[test]
    fn a_calibration_boot_without_a_touch_device_has_nothing_to_tap_with() {
        let (mut panel, frames, _) = panel();
        panel.deps.calibrate = true;
        let outcome = measure_touch(&mut panel, None::<&mut std::io::PipeReader>);
        assert_eq!(outcome, Ok(()));
        assert_eq!(frames.count(), 0);
        assert_eq!(panel.deps.calibration, Calibration::NOMINAL);
    }

    #[test]
    fn a_touch_device_that_goes_during_a_calibration_ends_the_panel() {
        let (mut panel, frames, _) = panel();
        panel.deps.calibrate = true;
        let (mut reader, writer) = std::io::pipe().unwrap();
        drop(writer);
        assert_eq!(
            measure_touch(&mut panel, Some(&mut reader)),
            Err(PanelEnd::Failed(
                "calibrating the touch screen: the touch device gave nothing more".into()
            ))
        );
        assert_eq!(frames.all(), [calibrate::target_frame(0)]);
    }

    /// A `/sys/class/input` holding the touch screen as `event2`, in a directory of its own, so that it can be moved into place whole.
    fn touch_input() -> TempDir {
        let sys = tempfile::tempdir().unwrap();
        fs::create_dir_all(sys.path().join("event2/device")).unwrap();
        fs::write(
            sys.path().join("event2/device/name"),
            format!("{}\n", touch::TOUCH_NAME),
        )
        .unwrap();
        sys
    }

    #[test]
    fn a_touch_device_that_never_comes_is_waited_for_as_long_as_asked_and_no_longer() {
        let sys = touch_input();
        let dev = Path::new("/dev/input");
        assert_eq!(
            await_touch(sys.path(), dev, Duration::ZERO),
            Some(dev.join("event2"))
        );
        let empty = tempfile::tempdir().unwrap();
        let began = Instant::now();
        assert_eq!(
            await_touch(empty.path(), dev, Duration::from_millis(200)),
            None
        );
        assert!(began.elapsed() >= Duration::from_millis(200));
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_touch_device_that_appears_while_its_driver_comes_up_is_waited_for() {
        let sys = tempfile::tempdir().unwrap();
        let appearing = touch_input();
        let dev = Path::new("/dev/input");
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(150));
                // Whole, so that it is never seen half made.
                fs::rename(appearing.path().join("event2"), sys.path().join("event2")).unwrap();
            });
            assert_eq!(
                await_touch(sys.path(), dev, Duration::from_secs(30)),
                Some(dev.join("event2"))
            );
        });
    }

    /// The panel's memory (spec 6.7): what its painters, the glass's kept frame and the world hold, summed
    /// after everything Phase 1 draws has been drawn through the panel's own painters, so that the glyph
    /// caches hold what a day of use puts in them. The ceiling is 5 MB more resident than the Drain
    /// alone; the sum here counts the Drain's painter as well, which is stricter.
    #[test]
    fn the_panels_buffers_and_caches_stay_within_the_memory_budget() {
        use qb_panel::drain::{Fixture, drain_fixture};
        use qb_panel::os::Change;
        use qb_panel::os::fixture::{Still, live_scene, mac_of, world as still_world};
        use qb_panel::os::world::{DeviceRow, MAX_DEVICES, WatchedName as Watched, display_name};

        // The spec's figures, in bytes: 1.2 MB a painter's frames, 2 MB of glyph caches in all, 64 KB of world.
        const FRAMES: usize = 614_400 + 307_200 + 307_200;
        const GLYPHS: usize = 2 * 1024 * 1024;
        const WORLD: usize = 64 * 1024;
        const CEILING: usize = 5 * 1000 * 1000;

        let (mut panel, _frames, _control) = panel();

        // Everything P1 draws: the Drain's stills, LIVE's two, every screen of the OS.
        for fixture in Fixture::ALL {
            let (drain, stills) = drain_fixture(fixture);
            panel.painter.paint(&drain, &stills);
        }
        for still in Still::ALL {
            if let Some((drain, stills)) = live_scene(still) {
                panel.painter.paint(&drain, &stills);
                continue;
            }
            let world = still_world(still);
            if let Some(view) = screens::view_of(&still.nav(&world), &world) {
                panel.screens.paint(view, screens::draw);
            }
        }
        // A crowded house with the longest names and the widest words, every page of the wall, DEVICE with
        // its two watched names, ALERT with its fullest lines.
        let mut crowd = still_world(Still::Devices);
        let widest = "W".repeat(qb_panel::os::world::DEVICE_NAME_CELLS);
        for octet in 0..MAX_DEVICES as u8 {
            let (name, named) = if octet % 2 == 0 {
                (widest.clone(), true)
            } else {
                display_name(None, None, octet)
            };
            let _ = crowd.push_device(DeviceRow {
                mac: mac_of(100 + octet),
                name,
                named,
                octet,
                state: qb_panel::os::world::DeviceState::Blocked,
                fixed: false,
                asking: octet % 5 == 0,
            });
        }
        crowd.watched = vec![
            Watched::new("a.very-long-and-busy-site.example.com", Outcome::Refused),
            Watched::new("www.example.org", Outcome::Passed),
        ];
        let mut fullest = still_world(Still::Alert);
        let long = "M".repeat(40);
        fullest.top_alert = Some(qb_panel::os::world::AlertRow::new(
            9,
            true,
            &"W".repeat(24),
            &[&long, &long, &long],
        ));
        for page in 0..3 {
            let mut nav = Nav::at(Section::Devices);
            nav.apply(Change::DevicesPage(page));
            if let Some(view) = screens::view_of(&nav, &crowd) {
                panel.screens.paint(view, screens::draw);
            }
        }
        let mut nav = Nav::at(Section::Devices);
        nav.open(Opened::Device(crowd.devices[0].mac));
        if let Some(view) = screens::view_of(&nav, &crowd) {
            panel.screens.paint(view, screens::draw);
        }
        let mut nav = Nav::at(Section::Alerts);
        nav.open(Opened::Alert(9));
        if let Some(view) = screens::view_of(&nav, &fullest) {
            panel.screens.paint(view, screens::draw);
        }

        let ((drain_frames, drain_glyphs), (screen_frames, screen_glyphs)) =
            (panel.painter.bytes(), panel.screens.bytes());
        let glass = panel.glass.capacity();
        let world = World::bound_bytes().max(crowd.bytes()).max(fullest.bytes());
        let total = drain_frames + drain_glyphs + screen_frames + screen_glyphs + glass + world;
        println!(
            "panel memory: Drain painter {drain_frames} B of frames + {drain_glyphs} B of glyphs, \
             ScreenPainter {screen_frames} B + {screen_glyphs} B, glass {glass} B, \
             World bound {world} B: {total} B in all ({:.2} MB)",
            total as f64 / 1e6
        );

        // The frames are the spec's 1.2 MB each (RGBA, RGB565, and the one it is compared with).
        assert!(
            drain_frames <= FRAMES,
            "Drain painter frames {drain_frames} B"
        );
        assert!(
            screen_frames <= FRAMES,
            "ScreenPainter frames {screen_frames} B"
        );
        // Both glyph caches together stay inside the spec's 2 MB.
        assert!(
            drain_glyphs + screen_glyphs <= GLYPHS,
            "glyph caches {} B",
            drain_glyphs + screen_glyphs
        );
        // The glass keeps one RGB565 frame; the world is under its 64 KB.
        assert_eq!(glass, WIDTH * HEIGHT * 2);
        assert!(world <= WORLD, "world {world} B");
        assert!(total <= CEILING, "{total} B is over the 5 MB ceiling");
    }
}
