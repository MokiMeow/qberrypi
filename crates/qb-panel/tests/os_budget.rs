//! The OS screens' budget: the rows each write of a screen changes while its figures move.
//!
//! The screen's driver sends one span for each write, so `changed_rows` measures a write, and a write
//! inside a screen must fit a quarter of the screen's rows, 80 (spec §5.1, plan "Budget"). The one-off
//! full writes (a screen change, a page turned) are counted apart.
//!
//! HOME and DEVICES, and DEVICE with its names, are replayed over the mockup's traffic for 150 s at 8
//! frames a second, as the panel loop would show them: the lamps (LIVE's for 3 frames after a name, a
//! device's for 8 after it asked), the clock, the day's figures once a second, the main gate counting
//! down on HOME and one device's own gate counting down on the wall and on its screen. Each frame is
//! painted as it would be, and the writes are the bands of changed rows (`paint_bands`), which are
//! measured beside the one span of `paint`: two lamps in two rows of the wall, or the clock and a lamp,
//! changing in one frame are one span of well over 80 rows and two small bands.
//!
//! A run is made with the lamps read every frame, as the mockup's are (the harshest case), and once a
//! second, as the panel loop reads the clients (spec §5.2).

use qb_panel::os::fixture::{HOUSE, Still, mac_of, world};
use qb_panel::os::screens::{self, View};
use qb_panel::os::world::{DeviceState, Outcome, Pause, WatchedName};
use qb_panel::os::{Nav, Screen, Section, World};
use qb_panel::ui::ScreenPainter;
use qb_panel::{HEIGHT, Share};

mod os_common;
use os_common::traffic::Sim;

/// The most rows a write may change.
const WRITE_ROWS_MAX: usize = 80;
/// Frames in a second.
const FPS: usize = 8;
/// How long the traffic runs, in seconds.
const SECONDS: usize = 150;
/// For how many frames a device's lamp stays lit after it asked: the mockup's `d.ask = 8`.
const ASK_FRAMES: u32 = 8;
/// For how many frames LIVE's lamp stays lit after a name: the mockup's `m.beat = 3`.
const BEAT_FRAMES: u32 = 3;
/// How long the main gate and the device's own gate are open at the start, in seconds.
const GATE_SECS: u16 = 299;
/// The house's living room TV, the device whose own gate counts down and whose screen is watched.
const TV: usize = 2;

/// What a run shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Show {
    /// HOME with the main gate shut.
    Home,
    /// HOME with the main gate open and counting down.
    HomeGateOpen,
    /// The wall of DEVICES.
    Devices,
    /// DEVICE: the TV's screen with the names it asks.
    Device,
}

/// How often the panel reads which devices are asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lamps {
    /// Every frame, as the mockup's lamps change.
    EveryFrame,
    /// Once a second, as the panel loop reads the clients.
    EverySecond,
}

/// What a run measured.
struct Measured {
    /// The rows of each write: each band of each frame that changed.
    writes: Vec<usize>,
    /// The rows of the one span `paint` gives for each frame that changed.
    spans: Vec<usize>,
    /// How many frames changed.
    painted: usize,
    /// How many times a lamp changed (a device's, or LIVE's).
    lamp_changes: usize,
    /// The most bands (writes) one frame had.
    most_bands: usize,
    /// How many seconds of the countdown showed.
    countdown_seconds: usize,
}

/// The mean and the peak of some row counts.
fn summary(rows: &[usize]) -> (f64, usize) {
    let mean = rows.iter().sum::<usize>() as f64 / rows.len().max(1) as f64;
    (mean, rows.iter().copied().max().unwrap_or(0))
}

/// The day's figures, once a second, as the panel reads them from the resolver.
fn snapshot(world: &mut World, sim: &Sim) {
    world.time = Some(sim.time());
    world.queries_today = Some(sim.queries);
    world.refused_today = Some(sim.blocked);
    world.share = Share::of(u64::from(sim.blocked), u64::from(sim.queries));
}

/// Replays the traffic over a screen, painting each frame two ways, and measures what a write costs.
fn run(show: Show, lamps: Lamps) -> Measured {
    let mut sim = Sim::new(7, false);
    let mut world = world(Still::Home);
    let tv = mac_of(HOUSE[TV].octet);
    // The TV's own gate is open, counting down, as the Pi's second counts it.
    world.devices[TV].state = DeviceState::Open {
        left_secs: GATE_SECS,
    };
    if show == Show::HomeGateOpen {
        world.blocking = Some(Pause {
            left_secs: GATE_SECS,
            total_secs: 300,
        });
    }
    let nav = match show {
        Show::Home | Show::HomeGateOpen => Nav::new(),
        Show::Devices => Nav::at(Section::Devices),
        Show::Device => {
            let mut nav = Nav::at(Section::Devices);
            nav.open(Screen::Device(tv));
            nav
        }
    };
    let mut painter: ScreenPainter<View> = ScreenPainter::new();
    // The first paint is a screen change: it is not measured.
    painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );

    let mut measured = Measured {
        writes: vec![],
        spans: vec![],
        painted: 0,
        lamp_changes: 0,
        most_bands: 0,
        countdown_seconds: 0,
    };
    let mut asking = [0_u32; HOUSE.len()];
    let mut beat = 0_u32;
    let mut was_lit = (false, vec![false; HOUSE.len()]);
    for tick in 0..SECONDS * FPS {
        for query in sim.step(1.0 / FPS as f64) {
            if let Some(at) = HOUSE.iter().position(|house| house.octet == query.client) {
                asking[at] = ASK_FRAMES;
            }
            beat = BEAT_FRAMES;
            if show == Show::Device && query.client == HOUSE[TV].octet {
                // The TV's gate is open: its ads pass.
                let outcome = if query.ad {
                    Outcome::Passed
                } else {
                    Outcome::Answered
                };
                world
                    .watched
                    .insert(0, WatchedName::new(&query.name, outcome));
                world.watched.truncate(2);
            }
        }
        // The second's figures: the day's counts, the clock, the gates counting down.
        if tick % FPS == 0 {
            snapshot(&mut world, &sim);
            if tick > 0 {
                let left = GATE_SECS - u16::try_from(tick / FPS).unwrap_or(0);
                world.devices[TV].state = DeviceState::Open { left_secs: left };
                if show == Show::HomeGateOpen {
                    world.blocking = Some(Pause {
                        left_secs: left,
                        total_secs: 300,
                    });
                }
                measured.countdown_seconds += 1;
            }
        }
        world.beat = beat > 0;
        if lamps == Lamps::EveryFrame || tick % FPS == 0 {
            for (row, left) in world.devices.iter_mut().zip(&asking) {
                row.asking = *left > 0;
            }
        }
        // A lamp that changed is what the screen's writes are to show.
        let lit: Vec<bool> = world.devices.iter().map(|row| row.asking).collect();
        measured.lamp_changes += usize::from(was_lit.0 != world.beat)
            + lit
                .iter()
                .zip(&was_lit.1)
                .filter(|(now, was)| now != was)
                .count();
        was_lit = (world.beat, lit);
        asking
            .iter_mut()
            .for_each(|left| *left = left.saturating_sub(1));
        beat = beat.saturating_sub(1);

        let view = screens::view_of(&nav, &world).expect("a view");
        let bands = painter.paint_bands(view, screens::draw);
        if let (Some(first), Some(last)) = (bands.first(), bands.last()) {
            measured.painted += 1;
            // The one span `paint` would give for the frame: from the first band's first row to the last's end.
            measured.spans.push(last.end - first.start);
            measured.most_bands = measured.most_bands.max(bands.len());
            measured.writes.extend(bands.iter().map(|band| band.len()));
        }
    }
    measured
}

/// Reports a run and checks it against the budget.
fn check(name: &str, measured: &Measured) {
    let (write_mean, write_peak) = summary(&measured.writes);
    let (span_mean, span_peak) = summary(&measured.spans);
    let over = measured
        .spans
        .iter()
        .filter(|&&rows| rows > WRITE_ROWS_MAX)
        .count();
    println!(
        "{name}: {} frames changed of {}; per write avg {write_mean:.1} peak {write_peak} rows of {HEIGHT} \
         over {} writes, at most {} in a frame; as one span per frame avg {span_mean:.1} peak {span_peak}, \
         {over} frames over {WRITE_ROWS_MAX}; {} lamp changes",
        measured.painted,
        SECONDS * FPS,
        measured.writes.len(),
        measured.most_bands,
        measured.lamp_changes
    );
    assert!(
        write_peak <= WRITE_ROWS_MAX,
        "{name}: a write changed {write_peak} rows, over {WRITE_ROWS_MAX}"
    );
}

#[test]
fn budget_home_every_write_fits_80_rows() {
    for lamps in [Lamps::EveryFrame, Lamps::EverySecond] {
        for show in [Show::Home, Show::HomeGateOpen] {
            let measured = run(show, lamps);
            assert!(measured.lamp_changes > 20, "the traffic lit the lamps");
            check(&format!("{show:?} ({lamps:?})"), &measured);
        }
    }
}

#[test]
fn budget_devices_every_write_fits_80_rows() {
    for lamps in [Lamps::EveryFrame, Lamps::EverySecond] {
        let measured = run(Show::Devices, lamps);
        assert!(measured.lamp_changes > 20, "the traffic lit the lamps");
        assert_eq!(measured.countdown_seconds, SECONDS - 1, "the gate counted");
        check(&format!("Devices ({lamps:?})"), &measured);
    }
}

#[test]
fn budget_device_every_write_fits_80_rows() {
    for lamps in [Lamps::EveryFrame, Lamps::EverySecond] {
        let measured = run(Show::Device, lamps);
        check(&format!("Device ({lamps:?})"), &measured);
    }
}

/// Switching screens and turning the wall's page are the writes that are allowed to be large: the first
/// is the whole frame written once, the second whatever differs between two pages. Both are counted apart.
#[test]
fn a_switch_is_one_full_write_and_a_page_turn_one_write_of_what_differs() {
    let world = world(Still::Home);
    let mut painter: ScreenPainter<View> = ScreenPainter::new();
    let home = screens::view_of(&Nav::new(), &world).expect("HOME has a view");
    assert_eq!(painter.paint_bands(home, screens::draw), one(0..HEIGHT));
    // The panel loop writes a switch whole, and says so with `invalidate`.
    painter.invalidate();
    let wall = screens::view_of(&Nav::at(Section::Devices), &world).expect("the wall has a view");
    assert_eq!(painter.paint_bands(wall, screens::draw), one(0..HEIGHT));
}

/// A device asking in each of the three rows of the wall at once is three small writes, not the span between the first lamp and the last.
#[test]
fn lamps_in_three_rows_of_the_wall_are_three_small_writes() {
    let mut world = world(Still::Devices);
    world.devices.iter_mut().for_each(|row| row.asking = false);
    let mut painter: ScreenPainter<View> = ScreenPainter::new();
    let nav = Nav::at(Section::Devices);
    painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );
    world.devices[0].asking = true;
    world.devices[5].asking = true;
    world.devices[8].asking = true;
    let bands = painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );
    assert_eq!(bands.len(), 3, "{bands:?}");
    assert!(bands.iter().all(|band| band.len() <= 14), "{bands:?}");
}

/// A list of one band of rows.
fn one(rows: std::ops::Range<usize>) -> Vec<std::ops::Range<usize>> {
    vec![rows]
}
