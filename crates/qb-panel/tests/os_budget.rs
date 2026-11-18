//! The OS screens' budget: the rows each write of a screen changes while its figures move.
//!
//! ALERTS at rest (the log, round 9's concept A) is replayed too: nothing moves but the clock and the log's cursor,
//! 4 frames on and 4 off at 8 a second, whose write is the 18 rows it stands in (spec §12.4); and an alert
//! arriving over HOME and being dismissed, whose two screen changes are counted apart.
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
use qb_panel::os::{Arrivals, Intent, Nav, Screen, Section, World, arrive, refresh, tap};
use qb_panel::ui::ScreenPainter;
use qb_panel::{HEIGHT, Share};

mod os_common;
use os_common::traffic::Sim;

/// The most rows a write may change.
const WRITE_ROWS_MAX: usize = 80;
/// The most rows the log's cursor may change: 10 by 18 px (spec §12.4).
const CURSOR_ROWS_MAX: usize = 18;
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
    /// ALERTS: the log with three entries, at rest: the clock and the cursor move.
    Alerts,
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
    /// The rows of each write of a frame where only the log's cursor changed (not the clock).
    cursor_writes: Vec<usize>,
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
    let mut world = world(if show == Show::Alerts {
        Still::Alerts
    } else {
        Still::Home
    });
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
        Show::Alerts => Nav::at(Section::Alerts),
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
        cursor_writes: vec![],
    };
    let mut asking = [0_u32; HOUSE.len()];
    let mut beat = 0_u32;
    let mut was_lit = (false, vec![false; HOUSE.len()]);
    for tick in 0..SECONDS * FPS {
        let minute_before = world.time;
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
        // The log's cursor: on for 4 frames, off for 4.
        world.cursor_on = (tick / 4) % 2 == 0;
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
            if show == Show::Alerts && world.time == minute_before {
                measured
                    .cursor_writes
                    .extend(bands.iter().map(|band| band.len()));
            }
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

/// ALERTS at rest for 150 s: the clock, once a minute, and the log's cursor, twice a second, are all that move. The
/// cursor's write is the 18 rows it stands in; the clock's is a few small bands; none is near 80.
#[test]
fn budget_alerts_at_rest_the_cursor_is_18_rows_and_every_write_fits_80() {
    for lamps in [Lamps::EveryFrame, Lamps::EverySecond] {
        let measured = run(Show::Alerts, lamps);
        check(&format!("Alerts ({lamps:?})"), &measured);
        // 150 s at 8 frames a second: the cursor flips every 4 frames, 75 times (the first paint is the screen's).
        assert!(
            measured.cursor_writes.len() >= 70,
            "the cursor blinked: {} writes",
            measured.cursor_writes.len()
        );
        let (cursor_mean, cursor_peak) = summary(&measured.cursor_writes);
        println!(
            "Alerts cursor alone: {} writes, avg {cursor_mean:.1} peak {cursor_peak} rows",
            measured.cursor_writes.len()
        );
        assert_eq!(
            cursor_peak, CURSOR_ROWS_MAX,
            "the cursor is 10 by 18: a write of 18 rows"
        );
        // Nothing else changed in those frames: they are all the cursor, a single band of 18 rows.
        assert!(
            measured
                .cursor_writes
                .iter()
                .all(|&rows| rows == CURSOR_ROWS_MAX)
        );
        // Both 10:22 and 10:23 came in 150 s: the clock's writes are the lane's and the typed line's, in bands.
        assert!(
            measured.writes.iter().any(|&rows| rows != CURSOR_ROWS_MAX),
            "the minute changed"
        );
    }
}

/// An alert arriving over HOME opens its slip, and dismissing it goes back to the log: each is a screen change,
/// written whole once; what happens on the screens between is within the budget.
#[test]
fn an_alert_arriving_and_dismissed_is_two_screen_changes_and_each_screen_stays_within_the_budget() {
    let mut painter: ScreenPainter<View> = ScreenPainter::new();
    let mut nav = Nav::new();
    let mut waiting = world(Still::Home);
    painter.paint_bands(
        screens::view_of(&nav, &waiting).expect("a view"),
        screens::draw,
    );
    // The panel is quiet at HOME for a while; then four alerts are waiting, as `Still::HomeAlert` has them.
    let alerting = world(Still::HomeAlert);
    let mut arrivals = Arrivals::new();
    assert!(!arrive(&mut nav, &mut arrivals, &waiting), "none waiting");
    assert!(arrive(&mut nav, &mut arrivals, &alerting), "one arrived");
    waiting = alerting;
    let mut screen_changes = vec![];
    // The arrival is a screen change: written whole, as the loop writes a switch.
    painter.invalidate();
    let bands = painter.paint_bands(
        screens::view_of(&nav, &waiting).expect("the slip"),
        screens::draw,
    );
    assert_eq!(bands, vec![0..HEIGHT]);
    screen_changes.push(bands[0].len());
    // On the slip for 60 s the clock may tick over: every write within the screen is small.
    let mut worst = 0;
    for second in 1..=60_u32 {
        let mut now = waiting.clone();
        // 10:21 until the 40th second, then 10:22.
        now.time = Some(qb_panel::TimeOfDay {
            hour: 10,
            minute: if second > 39 { 22 } else { 21 },
        });
        for band in painter.paint_bands(
            screens::view_of(&nav, &now).expect("the slip"),
            screens::draw,
        ) {
            worst = worst.max(band.len());
        }
        waiting = now;
    }
    println!("slip at rest: the largest write is {worst} rows");
    assert!(worst <= WRITE_ROWS_MAX, "{worst} rows");
    // DISMISS acks it and goes back to the log, which no longer holds it: the second screen change.
    let intent = tap(&mut nav, &waiting, 100, 280);
    assert_eq!(intent, Some(Intent::Ack(4)));
    waiting.alerts.remove(0);
    waiting.alerts_waiting = 3;
    assert!(
        !refresh(&mut nav, &waiting),
        "the log is where the panel is"
    );
    painter.invalidate();
    let bands = painter.paint_bands(
        screens::view_of(&nav, &waiting).expect("the log"),
        screens::draw,
    );
    assert_eq!(bands, vec![0..HEIGHT]);
    screen_changes.push(bands[0].len());
    assert_eq!(
        screen_changes,
        [HEIGHT, HEIGHT],
        "two screen changes, counted apart"
    );
    // The log at rest after it: the cursor flips are single bands of 18 rows.
    let mut cursor = vec![];
    for tick in 0..80_usize {
        waiting.cursor_on = (tick / 4) % 2 == 0;
        for band in painter.paint_bands(
            screens::view_of(&nav, &waiting).expect("the log"),
            screens::draw,
        ) {
            cursor.push(band.len());
        }
    }
    assert!(
        cursor.len() >= 18 && cursor.iter().all(|&rows| rows == CURSOR_ROWS_MAX),
        "{cursor:?}"
    );
}

/// SYSTEM and its readings at rest for 150 s: nothing moves but the clock, once a minute (spec §12.4 and ruling R9:
/// no sweep, no timer that animates), and the log's cursor, which these screens do not draw. Every write within the
/// screen fits the budget; a reading the Pi takes every five seconds that has not changed is no write at all.
#[test]
fn budget_system_and_its_readings_at_rest_only_the_clock_moves() {
    use qb_panel::os::world::Rejoin;
    for still in [
        Still::System,
        Still::Wifi,
        Still::Version,
        Still::Uptime,
        Still::NightHours,
        Still::Lock,
        Still::About,
    ] {
        let mut world = world(still);
        let nav = still.nav(&world);
        let mut painter: ScreenPainter<View> = ScreenPainter::new();
        // The first paint is a screen change: not measured.
        painter.paint_bands(
            screens::view_of(&nav, &world).expect("a view"),
            screens::draw,
        );
        let mut writes = vec![];
        for tick in 0..SECONDS * FPS {
            // The cursor and the lamps flip as they would; none of these screens draws them.
            world.cursor_on = (tick / 4) % 2 == 0;
            world.beat = tick % 5 == 0;
            // The clock: 10:21, and 10:22 after 40 s, 10:23 after 100 s.
            let seconds = tick / FPS;
            world.time = Some(qb_panel::TimeOfDay {
                hour: 10,
                minute: 21 + u8::from(seconds >= 40) + u8::from(seconds >= 100),
            });
            let view = screens::view_of(&nav, &world).expect("a view");
            writes.extend(
                painter
                    .paint_bands(view, screens::draw)
                    .iter()
                    .map(std::ops::Range::len),
            );
        }
        println!(
            "{still:?} at rest: {} writes, largest {} rows",
            writes.len(),
            writes.iter().max().copied().unwrap_or(0)
        );
        assert_eq!(
            writes.len(),
            2,
            "{still:?}: the clock, twice, and nothing else"
        );
        assert!(
            writes.iter().all(|&rows| rows <= WRITE_ROWS_MAX),
            "{still:?}: {writes:?}"
        );
    }
    // Wi-Fi joining is one of the allowed one-off writes: WI-FI goes dark and says JOINING, and back; each is a
    // write of the rows that differ (the dashes, the word and the first line), never a sweep in between.
    let mut world = world(Still::Wifi);
    let nav = Still::Wifi.nav(&world);
    let mut painter: ScreenPainter<View> = ScreenPainter::new();
    painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );
    world.rejoin = Rejoin::Joining;
    world.wifi_joined = false;
    let down = painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );
    assert!(!down.is_empty());
    for _ in 0..40 {
        assert!(
            painter
                .paint_bands(
                    screens::view_of(&nav, &world).expect("a view"),
                    screens::draw
                )
                .is_empty(),
            "JOINING stands still until the link is back"
        );
    }
    world.rejoin = Rejoin::Wait(40);
    world.wifi_joined = true;
    let back = painter.paint_bands(
        screens::view_of(&nav, &world).expect("a view"),
        screens::draw,
    );
    assert!(!back.is_empty());
    println!(
        "WI-FI joining: {} rows down in {} bands, {} rows back in {} bands",
        down.iter().map(std::ops::Range::len).sum::<usize>(),
        down.len(),
        back.iter().map(std::ops::Range::len).sum::<usize>(),
        back.len()
    );
}

// ---------------------------------------------------------------------------------------------
// The states of the whole system (spec §5.4, plan Task 15): the trial's countdown, the receiving fill, the night clock,
// the checklist and the PIN pad. A change from one state to another is a screen change and is written whole once; what
// moves inside a state is measured here, band by band as the panel's write is.

/// The rows of each write a world's new view gives after `painter` showed the one before: each band that changed.
fn writes_of(painter: &mut ScreenPainter<View>, nav: &Nav, world: &World) -> Vec<usize> {
    painter
        .paint_bands(screens::view_of(nav, world).expect("a view"), screens::draw)
        .iter()
        .map(std::ops::Range::len)
        .collect()
}

/// A painter that shows `nav` and `world`: the screen change that put it there is not measured.
fn showing(nav: &Nav, world: &World) -> ScreenPainter<View> {
    let mut painter = ScreenPainter::new();
    painter.paint_bands(screens::view_of(nav, world).expect("a view"), screens::draw);
    painter
}

/// An update on its way from slot A to slot B in this phase.
fn updating(phase: qb_panel::os::world::UpdatePhase) -> World {
    use qb_panel::os::world::{Slot, UpdateRead};
    World {
        update: Some(UpdateRead {
            phase,
            before: Slot::A,
            after: Slot::B,
        }),
        version: "0.4.2".to_string(),
        ..world(Still::Home)
    }
}

fn update_nav() -> Nav {
    let mut nav = Nav::new();
    nav.set_whole(Some(qb_panel::os::Whole::Update));
    nav
}

/// The trial counts down 120 s once a second, then names what it waits for: every write is the figure's rows.
#[test]
fn budget_update_trial_countdown_is_a_write_of_the_figures_rows_a_second() {
    use qb_panel::os::world::{UpdatePhase, Waiting};
    let nav = update_nav();
    let trial = |left_secs, waiting_for| UpdatePhase::Trial {
        left_secs,
        waiting_for,
    };
    let mut painter = showing(&nav, &updating(trial(120, None)));
    let mut writes = vec![];
    for left in (0..120).rev() {
        writes.extend(writes_of(&mut painter, &nav, &updating(trial(left, None))));
    }
    // Past the minimum time the trial still waits for Wi-Fi, then the clock, then the upstream.
    for waiting in [Waiting::Wifi, Waiting::Clock, Waiting::Upstream] {
        writes.extend(writes_of(
            &mut painter,
            &nav,
            &updating(trial(0, Some(waiting))),
        ));
    }
    let (mean, peak) = summary(&writes);
    println!(
        "UPDATE trial countdown: {} writes over 120 s and three words, avg {mean:.1} peak {peak} rows",
        writes.len()
    );
    assert!(writes.len() >= 120, "a write a second");
    assert!(peak <= WRITE_ROWS_MAX, "{peak} rows");
}

/// The receiving fill rises as the build comes in: by every percent, by jumps of five, and from nothing to the whole
/// in one step (a world made once a second may find a lot received). The fill and the share are one write.
#[test]
fn budget_update_receiving_fill_is_a_write_of_at_most_the_box_and_every_write_fits_80_rows() {
    use qb_panel::os::world::UpdatePhase;
    const LENGTH: u64 = 5_000_000;
    let nav = update_nav();
    let receiving = |received| {
        updating(UpdatePhase::Receiving {
            received,
            length: LENGTH,
        })
    };
    for (what, step) in [
        ("every percent", LENGTH / 100),
        ("by five percent", LENGTH / 20),
        ("a third at a time", LENGTH / 3),
        ("the whole at once", LENGTH),
    ] {
        let mut painter = showing(&nav, &receiving(0));
        let mut writes = vec![];
        let mut received = 0;
        while received < LENGTH {
            received = (received + step).min(LENGTH);
            writes.extend(writes_of(&mut painter, &nav, &receiving(received)));
        }
        let (mean, peak) = summary(&writes);
        println!(
            "UPDATE receiving {what}: {} writes, avg {mean:.1} peak {peak} rows",
            writes.len()
        );
        assert!(!writes.is_empty(), "{what}");
        assert!(peak <= WRITE_ROWS_MAX, "{what}: {peak} rows");
    }
}

/// Receiving, on trial, confirmed and rolled back are four arrangements of one screen: each change is a whole-state
/// change, written whole once, and a screen that has not changed writes nothing.
#[test]
fn an_update_changing_stage_is_one_whole_write_and_a_still_update_writes_nothing() {
    use qb_panel::os::world::UpdatePhase;
    let nav = update_nav();
    let mut painter = showing(
        &nav,
        &updating(UpdatePhase::Receiving {
            received: 100,
            length: 100,
        }),
    );
    for phase in [
        UpdatePhase::Trial {
            left_secs: 120,
            waiting_for: None,
        },
        UpdatePhase::Confirmed,
    ] {
        // The panel loop writes a state change whole, and says so with `invalidate`.
        painter.invalidate();
        let bands = painter.paint_bands(
            screens::view_of(&nav, &updating(phase)).expect("a view"),
            screens::draw,
        );
        assert_eq!(bands, one(0..HEIGHT), "{phase:?}");
        assert!(
            writes_of(&mut painter, &nav, &updating(phase)).is_empty(),
            "{phase:?}: nothing moves at rest"
        );
    }
}

/// The night clock's minute changes at 96 px: the hour's rows, 134 to 205 by the font, are all that change, and the
/// carries (09:59 to 10:00, 23:59 to 00:00) change three or four digits in the same rows. Measured, as the plan asks.
#[test]
fn budget_night_clock_minute_change_is_the_hours_rows_and_fits_80() {
    use qb_panel::TimeOfDay;
    let mut nav = Nav::new();
    nav.set_whole(Some(qb_panel::os::Whole::Night));
    let at = |hour, minute, waiting| World {
        time: Some(TimeOfDay { hour, minute }),
        alerts_waiting: waiting,
        ..World::default()
    };
    let mut all = vec![];
    for waiting in [0, 4] {
        let mut painter = showing(&nav, &at(22, 59, waiting));
        // Every minute of the night, which takes every digit through every glyph (23:00 to 07:00: 480 changes).
        let (mut hour, mut minute) = (22_u8, 59_u8);
        for _ in 0..(8 * 60 + 60) {
            minute += 1;
            if minute == 60 {
                minute = 0;
                hour = (hour + 1) % 24;
            }
            all.extend(writes_of(&mut painter, &nav, &at(hour, minute, waiting)));
        }
    }
    let (mean, peak) = summary(&all);
    println!(
        "NIGHT clock: {} minute changes, avg {mean:.1} peak {peak} rows of {HEIGHT}",
        all.len()
    );
    assert!(all.len() >= 1000, "every minute of the night, twice over");
    assert!(
        peak <= WRITE_ROWS_MAX,
        "the minute change is {peak} rows, over {WRITE_ROWS_MAX}: say so in the commit"
    );
    // An alert arriving turns the words white and says how many: the words' rows only.
    let mut painter = showing(&nav, &at(2, 14, 0));
    let writes = writes_of(&mut painter, &nav, &at(2, 14, 1));
    assert!(
        !writes.is_empty() && writes.iter().all(|&rows| rows <= 20),
        "{writes:?}"
    );
}

/// STARTING ticks a line at a time: each tick is the rows of two lines at most, and a reading that does not change writes nothing.
#[test]
fn budget_starting_each_tick_is_one_lines_rows() {
    use qb_panel::TimeOfDay;
    let mut nav = Nav::new();
    nav.set_whole(Some(qb_panel::os::Whole::Starting));
    let mut world = World::default();
    let mut painter = showing(&nav, &world);
    let mut writes = vec![];
    world.wifi_joined = true;
    writes.extend(writes_of(&mut painter, &nav, &world));
    world.time = Some(TimeOfDay {
        hour: 10,
        minute: 21,
    });
    writes.extend(writes_of(&mut painter, &nav, &world));
    world.list_names = Some(212_000);
    writes.extend(writes_of(&mut painter, &nav, &world));
    world.dns_answering = true;
    writes.extend(writes_of(&mut painter, &nav, &world));
    assert!(writes_of(&mut painter, &nav, &world).is_empty());
    let (mean, peak) = summary(&writes);
    println!(
        "STARTING: {} ticks, avg {mean:.1} peak {peak} rows",
        writes.len()
    );
    // Each tick changes two lines (the one done, the one now under way): two bands of a line's rows, three at the ends.
    assert!(writes.len() >= 4, "{writes:?}");
    assert!(peak <= WRITE_ROWS_MAX, "{peak} rows");
}

/// The PIN pad: a digit typed is the box's square, a wrong try the question and the boxes, a wait a write of the
/// question's rows a second.
#[test]
fn budget_pin_typing_wrong_and_the_wait_are_small_writes() {
    use qb_panel::os::{Verified, verified};
    let mut world = world(Still::Pin);
    let mut nav = Still::Pin.nav(&world);
    let mut painter = showing(&nav, &world);
    let mut writes = vec![];
    let key = |i: usize| {
        let key = qb_panel::os::screens::whole::pad_rect(i);
        (key.x + 45, key.y + 25)
    };
    // The third and fourth digits (the first two are typed), the lock says wrong, and a digit takes the word back.
    for i in [2, 3] {
        let (x, y) = key(i);
        tap(&mut nav, &world, x, y);
        writes.extend(writes_of(&mut painter, &nav, &world));
    }
    verified(&mut nav, Verified::Wrong);
    writes.extend(writes_of(&mut painter, &nav, &world));
    let (x, y) = key(4);
    tap(&mut nav, &world, x, y);
    writes.extend(writes_of(&mut painter, &nav, &world));
    // Three wrong in a row: the wait, 30 s counted down by the second.
    verified(&mut nav, Verified::Wait(30));
    for left in (0..=30).rev() {
        world.pin_wait_secs = left;
        writes.extend(writes_of(&mut painter, &nav, &world));
        refresh(&mut nav, &world);
    }
    let (mean, peak) = summary(&writes);
    println!(
        "PIN pad: {} writes, avg {mean:.1} peak {peak} rows",
        writes.len()
    );
    assert!(writes.len() > 30);
    assert!(peak <= WRITE_ROWS_MAX, "{peak} rows");
}
