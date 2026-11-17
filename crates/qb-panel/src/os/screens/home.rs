//! HOME (spec §3.2, the mockup's `wall` home in `homes.js`): the lane, and six tiles of 150 by 108, three
//! across and two down: LIVE, DEVICES, ALERTS; the gate key, NAMES, SYSTEM. Each is a label, a lamp, one
//! 40 px figure and a line, and each is a door.
//!
//! `HomeView::of` makes the view of exactly what is on the glass, words and colours decided, so a lamp, a
//! count or a place of the mini wall that changes is a change of the view, and `draw` paints it.

use crate::format::{self, DASH};
use crate::geometry::{Rect, Rgb};
use crate::os::nav::Section;
use crate::os::screens::devices::{self, MINI_PLACES, Place};
use crate::os::screens::lane::{self, LaneView};
use crate::os::world::World;
use crate::state::TimeOfDay;
use crate::ui::canvas::{Canvas, Face};
use crate::ui::look::{
    BLUE, BLUE_DEEP, BLUE_LT, BLUE_MD, BLUE_NIGHT, G1, G2, G3, G4, GROUND, HOME_COLUMNS,
    HOME_HIT_X0, HOME_HIT_Y0, HOME_TILE_H, HOME_TILE_PITCH_X, HOME_TILE_PITCH_Y, HOME_TILE_W,
    HOME_TILE_X0, HOME_TILE_Y0, HOME_TILES, WHITE,
};

/// The tiles, by their place on HOME.
pub const LIVE: usize = 0;
/// The tiles, by their place on HOME.
pub const DEVICES: usize = 1;
/// The tiles, by their place on HOME.
pub const ALERTS: usize = 2;
/// The tiles, by their place on HOME: the gate, which is a key.
pub const GATE: usize = 3;
/// The tiles, by their place on HOME.
pub const NAMES: usize = 4;
/// The tiles, by their place on HOME.
pub const SYSTEM: usize = 5;

/// How many places stand in a row of the mini wall (`devices::MINI_PLACES` of them, six across and two down).
const MINI_COLUMNS: usize = 6;
/// The distance between two places of the mini wall.
const MINI_PITCH: u16 = 13;
/// A place of the mini wall is this wide and this high.
const MINI_SIZE: u16 = 10;
/// Where the mini wall starts in its tile.
const MINI_AT: (u16, u16) = (66, 45);

/// The longest figure (in letters) that is set at the full 40 px: six letters of 24 px stand in the tile.
const FIGURE_BIG_LETTERS: usize = 6;
/// The longest figure when it is set smaller, 30 px.
const FIGURE_SMALL_LETTERS: usize = 7;

/// How a tile looks: its ground, and the ink of its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// A plain tile: deep blue, white figure, light blue words.
    Plain,
    /// A tile that wants a look (alerts wait): white, black words.
    Attn,
    /// A tile with nothing to say: a hollow grey frame and grey words.
    Calm,
    /// The gate key while the gate is shut: solid blue.
    KeyShut,
    /// The gate key while the gate is open: a white frame round black.
    KeyOpen,
}

impl Look {
    /// The colours of a look: the figure's ink and the words' (`ink`, `dim`).
    const fn inks(self) -> (Rgb, Rgb) {
        match self {
            Self::Plain => (WHITE, BLUE_LT),
            Self::Attn => (GROUND, GROUND),
            Self::Calm => (G3, G3),
            Self::KeyShut => (WHITE, BLUE_NIGHT),
            Self::KeyOpen => (WHITE, G1),
        }
    }

    /// What the tile's frame is drawn on inside: the ground the mini wall's hollow places show.
    const fn ground(self) -> Rgb {
        match self {
            Self::Plain => BLUE_DEEP,
            Self::Attn => WHITE,
            Self::KeyShut => BLUE,
            Self::Calm | Self::KeyOpen => GROUND,
        }
    }
}

/// One tile as it is on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileView {
    /// The ground and the inks.
    pub look: Look,
    /// The small label at the top.
    pub label: &'static str,
    /// The lamp at the top right and its colour, if the tile has one.
    pub lamp: Option<Rgb>,
    /// The figure, 40 px bold (30 px when it is long).
    pub figure: String,
    /// The figure's size.
    pub figure_px: u8,
    /// The figure's ink: the look's, or `G3` while the figure is not known.
    pub figure_ink: Rgb,
    /// The line under the figure.
    pub line: String,
    /// The mini wall of devices, on DEVICES' tile.
    pub mini: Option<[Place; MINI_PLACES]>,
}

/// What HOME puts on the glass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeView {
    /// The lane, with the system in two words.
    pub lane: LaneView,
    /// The six tiles, in `LIVE`, `DEVICES`, `ALERTS`, `GATE`, `NAMES`, `SYSTEM` order.
    pub tiles: [TileView; HOME_TILES],
}

/// A figure as its text, size and ink: `—` in `G3` when it is not known; a long one is smaller.
fn figure(text: Option<String>, ink: Rgb) -> (String, u8, Rgb) {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return (DASH.to_string(), 40, G3);
    };
    if text.chars().count() <= FIGURE_BIG_LETTERS {
        (text, 40, ink)
    } else {
        (format::fit_text(&text, FIGURE_SMALL_LETTERS), 30, ink)
    }
}

/// A tile from its parts; the figure is `None` while it is not known.
fn tile(
    look: Look,
    label: &'static str,
    lamp: Option<Rgb>,
    figure_text: Option<String>,
    line: String,
) -> TileView {
    let (figure, figure_px, figure_ink) = figure(figure_text, look.inks().0);
    TileView {
        look,
        label,
        lamp,
        figure,
        figure_px,
        figure_ink,
        line,
        mini: None,
    }
}

/// `NEWEST hh:mm`, or the words for an alert that came before today.
fn newest(time: Option<TimeOfDay>) -> String {
    match time {
        Some(time) => format!("NEWEST {}", format::clock(Some(time))),
        None => "BEFORE TODAY".to_string(),
    }
}

/// `UP n DAYS`, with the dash for a count that is not known.
fn up_days(days: Option<u32>) -> String {
    match days {
        Some(1) => "UP 1 DAY".to_string(),
        Some(days) => format!("UP {days} DAYS"),
        None => format!("UP {DASH}"),
    }
}

impl HomeView {
    /// What HOME shows for a world.
    pub fn of(world: &World) -> Self {
        let waiting = world.alerts_waiting;
        let lamp = |lit: bool| Some(if lit { WHITE } else { BLUE });
        let live = tile(
            Look::Plain,
            "LIVE",
            lamp(world.beat),
            world.refused_today.map(format::count),
            format!(
                "{} REFUSED",
                world.share.map_or(DASH.to_string(), |share| share.text())
            ),
        );
        let mut devices = tile(
            Look::Plain,
            "DEVICES",
            lamp(world.any_asking()),
            Some(format::count(
                u32::try_from(world.devices.len()).unwrap_or(u32::MAX),
            )),
            format!("{} BLOCKED", world.blocked_count()),
        );
        // The same rows as the wall of DEVICES, one place each (`devices::Place::of`): its first twelve.
        devices.mini = Some(devices::mini_places(&world.devices));
        let alerts = if waiting > 0 {
            // A white tile has no lamp.
            tile(
                Look::Attn,
                "ALERTS",
                None,
                Some(format::count(waiting)),
                newest(world.alert_newest),
            )
        } else {
            tile(
                Look::Calm,
                "ALERTS",
                lamp(false),
                Some("0".to_string()),
                "ALL CLEAR".to_string(),
            )
        };
        let gate = match world.blocking {
            Some(pause) => tile(
                Look::KeyOpen,
                "OPEN",
                None,
                Some(format::mss(pause.left_secs)),
                "ADS PASS".to_string(),
            ),
            None => tile(
                Look::KeyShut,
                "GATE",
                None,
                Some("SHUT".to_string()),
                "ADS REFUSED".to_string(),
            ),
        };
        let names = tile(
            Look::Plain,
            "NAMES",
            lamp(false),
            world.list_names.map(format::short_count),
            "NAMES LISTED".to_string(),
        );
        let system = tile(
            Look::Plain,
            "SYSTEM",
            lamp(false),
            Some(world.version.clone()),
            up_days(world.uptime_days),
        );
        Self {
            lane: LaneView::of(Section::Home, world),
            tiles: [live, devices, alerts, gate, names, system],
        }
    }
}

/// The place of tile `i` (`HT`): 150 by 108 at `x 8 + 157·col, y 80 + 116·row`.
pub fn tile_rect(i: usize) -> Rect {
    let (col, row) = (i % HOME_COLUMNS, i / HOME_COLUMNS);
    Rect::new(
        HOME_TILE_X0 + u16::try_from(col).unwrap_or(0) * HOME_TILE_PITCH_X,
        HOME_TILE_Y0 + u16::try_from(row).unwrap_or(0) * HOME_TILE_PITCH_Y,
        HOME_TILE_W,
        HOME_TILE_H,
    )
}

/// The tile a press at `(x, y)` is on (`wallHit`): rows from y 76, the second from 192; columns from
/// x 4, the second from 161 and the third from 318; the last row and column go on to the screen's edges.
/// `None` above y 76.
pub fn wall_hit(x: u16, y: u16) -> Option<usize> {
    if y < HOME_HIT_Y0 {
        return None;
    }
    let row = usize::from((y - HOME_HIT_Y0) / HOME_TILE_PITCH_Y).min(1);
    let col = usize::from(x.saturating_sub(HOME_HIT_X0) / HOME_TILE_PITCH_X).min(HOME_COLUMNS - 1);
    Some(row * HOME_COLUMNS + col)
}

/// Draws one tile (`wallTile`).
fn draw_tile(canvas: &mut Canvas, r: Rect, tile: &TileView) {
    let (_, dim) = tile.look.inks();
    let ground = tile.look.ground();
    match tile.look {
        Look::KeyOpen => canvas.hollow(r, WHITE, 4, GROUND),
        Look::Calm => canvas.hollow(r, G4, 2, GROUND),
        _ => canvas.rect(r, ground),
    }
    let (x, y) = (f32::from(r.x), f32::from(r.y));
    canvas.text(
        tile.label,
        x + 10.0,
        y + 27.0,
        Face::bold(16.0, dim).track(2.0),
    );
    if let Some(lamp) = tile.lamp {
        canvas.dot(x + f32::from(r.w) - 16.0, y + 21.0, 5.0, lamp);
    }
    canvas.text(
        &tile.figure,
        x + 8.0,
        y + 72.0,
        Face::bold(f32::from(tile.figure_px), tile.figure_ink),
    );
    if let Some(places) = &tile.mini {
        draw_mini_wall(canvas, r.x + MINI_AT.0, r.y + MINI_AT.1, places, ground);
    }
    canvas.text(
        &tile.line,
        x + 10.0,
        y + 96.0,
        Face::bold(16.0, dim).track(1.0),
    );
}

/// Draws the devices in miniature (`miniWall`): twelve places, a device a 10 by 10 block on a 13 px pitch.
fn draw_mini_wall(canvas: &mut Canvas, x: u16, y: u16, places: &[Place; MINI_PLACES], ground: Rgb) {
    for (i, place) in places.iter().enumerate() {
        let col = u16::try_from(i % MINI_COLUMNS).unwrap_or(0);
        let row = u16::try_from(i / MINI_COLUMNS).unwrap_or(0);
        let (sx, sy) = (x + col * MINI_PITCH, y + row * MINI_PITCH);
        let place_rect = Rect::new(sx, sy, MINI_SIZE, MINI_SIZE);
        match place {
            Place::Empty => canvas.dot(f32::from(sx) + 5.0, f32::from(sy) + 5.0, 1.5, BLUE_MD),
            Place::Asking => canvas.rect(place_rect, WHITE),
            Place::Pass => canvas.hollow(place_rect, G2, 2, ground),
            Place::Blocked => canvas.rect(place_rect, BLUE),
        }
    }
}

/// Draws HOME: the lane and the six tiles.
pub fn draw(canvas: &mut Canvas, view: &HomeView) {
    lane::draw(canvas, &view.lane);
    for (i, tile) in view.tiles.iter().enumerate() {
        draw_tile(canvas, tile_rect(i), tile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::fixture::{Still, world};
    use crate::os::world::{DeviceRow, DeviceState, Pause, display_name};
    use crate::ui::painter::ScreenPainter;

    fn view(still: Still) -> HomeView {
        HomeView::of(&world(still))
    }

    #[test]
    fn the_tiles_are_where_the_numbers_put_them() {
        let places: Vec<(u16, u16)> = (0..HOME_TILES)
            .map(|i| (tile_rect(i).x, tile_rect(i).y))
            .collect();
        assert_eq!(
            places,
            [
                (8, 80),
                (165, 80),
                (322, 80),
                (8, 196),
                (165, 196),
                (322, 196)
            ]
        );
        assert!((0..HOME_TILES).all(|i| (tile_rect(i).w, tile_rect(i).h) == (150, 108)));
    }

    #[test]
    fn the_home_at_rest_says_what_the_mockup_says() {
        let home = view(Still::Home);
        let said: Vec<(&str, &str, &str)> = home
            .tiles
            .iter()
            .map(|t| (t.label, t.figure.as_str(), t.line.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("LIVE", "4,780", "44% REFUSED"),
                ("DEVICES", "10", "2 BLOCKED"),
                ("ALERTS", "0", "ALL CLEAR"),
                ("GATE", "SHUT", "ADS REFUSED"),
                ("NAMES", "212K", "NAMES LISTED"),
                ("SYSTEM", "0.4.1", "UP 12 DAYS"),
            ]
        );
        let looks: Vec<Look> = home.tiles.iter().map(|t| t.look).collect();
        assert_eq!(
            looks,
            [
                Look::Plain,
                Look::Plain,
                Look::Calm,
                Look::KeyShut,
                Look::Plain,
                Look::Plain
            ]
        );
        // Lamps: LIVE blue (no name just went by), DEVICES white (two ask), ALERTS blue (clear), the key none.
        let lamps: Vec<Option<Rgb>> = home.tiles.iter().map(|t| t.lamp).collect();
        assert_eq!(
            lamps,
            [
                Some(BLUE),
                Some(WHITE),
                Some(BLUE),
                None,
                Some(BLUE),
                Some(BLUE)
            ]
        );
        assert!(home.tiles.iter().all(|t| t.figure_px == 40));
    }

    #[test]
    fn alerts_make_the_tile_white_with_the_time_and_no_lamp() {
        let home = view(Still::HomeAlert);
        let alerts = &home.tiles[ALERTS];
        assert_eq!(alerts.look, Look::Attn);
        assert_eq!(
            (alerts.figure.as_str(), alerts.line.as_str(), alerts.lamp),
            ("4", "NEWEST 10:19", None)
        );
        // An alert that came before today has no time to give.
        let mut older = world(Still::HomeAlert);
        older.alert_newest = None;
        assert_eq!(HomeView::of(&older).tiles[ALERTS].line, "BEFORE TODAY");
    }

    #[test]
    fn an_open_gate_is_a_hollow_key_counting_down_to_an_hour() {
        let home = view(Still::HomeOpen);
        let gate = &home.tiles[GATE];
        assert_eq!(gate.look, Look::KeyOpen);
        assert_eq!(
            (gate.label, gate.figure.as_str(), gate.line.as_str()),
            ("OPEN", "4:37", "ADS PASS")
        );
        let mut hour = world(Still::HomeOpen);
        hour.blocking = Some(Pause {
            left_secs: 3_600,
            total_secs: 3_600,
        });
        assert_eq!(HomeView::of(&hour).tiles[GATE].figure, "60:00");
        // The lane says it too.
        assert_eq!(home.lane.words, "GATE OPEN");
    }

    #[test]
    fn what_is_not_known_is_a_dash_in_g3() {
        let home = HomeView::of(&World::default());
        for i in [LIVE, NAMES] {
            let t = &home.tiles[i];
            assert_eq!((t.figure.as_str(), t.figure_ink), (DASH, G3), "{}", t.label);
        }
        assert_eq!(home.tiles[LIVE].line, format!("{DASH} REFUSED"));
        assert_eq!(home.tiles[SYSTEM].figure, DASH, "no version yet");
        assert_eq!(home.tiles[SYSTEM].line, format!("UP {DASH}"));
        // A known figure keeps the look's ink.
        let known = view(Still::Home);
        assert_eq!(known.tiles[LIVE].figure_ink, WHITE);
        assert_eq!(
            known.tiles[ALERTS].figure_ink, G3,
            "the calm tile's own grey"
        );
        assert_eq!(up_days(Some(1)), "UP 1 DAY");
        assert_eq!(up_days(Some(0)), "UP 0 DAYS");
    }

    #[test]
    fn a_long_figure_is_set_smaller_and_cut() {
        let mut long = world(Still::Home);
        long.version = "0.10.2-rc1".to_string();
        let t = &HomeView::of(&long).tiles[SYSTEM];
        assert_eq!((t.figure_px, t.figure.chars().count()), (30, 7));
        assert!(t.figure.ends_with('~'));
        long.version = "0.10.22".to_string();
        assert_eq!(HomeView::of(&long).tiles[SYSTEM].figure_px, 30);
        long.version = "0.4.12".to_string();
        assert_eq!(HomeView::of(&long).tiles[SYSTEM].figure_px, 40);
        // The biggest count is six letters.
        long.refused_today = Some(99_999);
        let live = &HomeView::of(&long).tiles[LIVE];
        assert_eq!((live.figure.as_str(), live.figure_px), ("99,999", 40));
    }

    #[test]
    fn the_mini_wall_is_the_first_twelve_devices_as_the_wall_shows_them() {
        let home = view(Still::Home);
        // Desk PC, Sam's phone (asks), the TV and the tablet (blocked), the printer (asks), the doorbell, two
        // unnamed (ads pass), the router and the Pi (blue, as the mockup draws `always`), two empty places.
        assert_eq!(
            home.tiles[DEVICES].mini,
            Some([
                Place::Pass,
                Place::Asking,
                Place::Blocked,
                Place::Blocked,
                Place::Asking,
                Place::Pass,
                Place::Pass,
                Place::Pass,
                Place::Blocked,
                Place::Blocked,
                Place::Empty,
                Place::Empty
            ])
        );
        assert!(
            home.tiles
                .iter()
                .enumerate()
                .all(|(i, t)| (i == DEVICES) == t.mini.is_some())
        );
        // A device whose own gate is open is a hollow place; more than twelve show twelve.
        let mut busy = world(Still::Home);
        busy.devices[2].state = DeviceState::Open { left_secs: 60 };
        for octet in 100..140 {
            let (name, named) = display_name(None, None, octet);
            busy.push_device(DeviceRow {
                mac: [2, 0, 0, 0, 0, octet],
                name,
                named,
                octet,
                state: DeviceState::Blocked,
                fixed: false,
                asking: false,
            });
        }
        let mini = HomeView::of(&busy).tiles[DEVICES]
            .mini
            .expect("a mini wall");
        assert_eq!(mini[2], Place::Pass);
        assert!(mini[10] == Place::Blocked && mini[11] == Place::Blocked);
        assert_eq!(
            HomeView::of(&busy).tiles[DEVICES].figure,
            "33",
            "the figure is the wall's count (33 at most)"
        );
    }

    #[test]
    fn the_wall_hit_is_the_mockups() {
        // `wallHit`: y >= 76 ? min(1, floor((y - 76) / 116)) * 3 + min(2, floor((x - 4) / 157)) : -1.
        assert_eq!(wall_hit(0, 75), None);
        assert_eq!(wall_hit(0, 76), Some(0));
        assert_eq!(wall_hit(160, 76), Some(0));
        assert_eq!(wall_hit(161, 76), Some(1));
        assert_eq!(wall_hit(317, 191), Some(1));
        assert_eq!(wall_hit(318, 191), Some(2));
        assert_eq!(wall_hit(479, 191), Some(2));
        assert_eq!(wall_hit(4, 192), Some(3));
        assert_eq!(wall_hit(479, 319), Some(5));
        assert_eq!(wall_hit(u16::MAX, u16::MAX), Some(5));
        // x < 4 is the first column (the mockup's index is -1 there, which on the second row is the third tile).
        assert_eq!(wall_hit(0, 300), Some(3));
    }

    #[test]
    fn a_tile_is_drawn_on_its_ground_with_its_corner_and_the_hollow_ones_with_a_frame() {
        let mut canvas = Canvas::new();
        draw(&mut canvas, &view(Still::HomeOpen));
        let px = |x, y| canvas.pixel(x, y);
        // Plain: deep blue; calm: a 2 px G4 frame round black; the open key: a 4 px white frame round black.
        assert_eq!(px(9, 81), Some(BLUE_DEEP));
        assert_eq!(px(322, 80), Some(G4));
        assert_eq!(px(323, 81), Some(G4));
        assert_eq!(px(324, 82), Some(GROUND));
        assert_eq!(px(8, 196), Some(WHITE));
        assert_eq!(px(11, 199), Some(WHITE));
        assert_eq!(px(12, 200), Some(GROUND));
        // The gaps between tiles are black.
        assert_eq!(px(158, 100), Some(GROUND));
        assert_eq!(px(100, 190), Some(GROUND));
        let mut shut = Canvas::new();
        draw(&mut shut, &view(Still::Home));
        assert_eq!(shut.pixel(8, 196), Some(BLUE));
        let mut alert = Canvas::new();
        draw(&mut alert, &view(Still::HomeAlert));
        assert_eq!(alert.pixel(322, 80), Some(WHITE));
    }

    #[test]
    fn a_lamp_that_flips_is_a_write_of_a_few_rows() {
        let mut painter = ScreenPainter::new();
        let mut w = world(Still::Home);
        assert_eq!(
            painter.paint(HomeView::of(&w), draw),
            Some(0..crate::HEIGHT)
        );
        assert_eq!(painter.paint(HomeView::of(&w), draw), None);
        // LIVE's lamp: r 5 at (142, 101).
        w.beat = true;
        let rows = painter
            .paint(HomeView::of(&w), draw)
            .expect("the lamp changed");
        assert!(rows.len() <= 11, "{rows:?}");
        // The gate counts down: only the key's figure.
        w.beat = false;
        painter.paint(HomeView::of(&w), draw);
        w.blocking = Some(Pause {
            left_secs: 299,
            total_secs: 300,
        });
        painter.paint(HomeView::of(&w), draw);
        w.blocking = Some(Pause {
            left_secs: 298,
            total_secs: 300,
        });
        let rows = painter
            .paint(HomeView::of(&w), draw)
            .expect("a second went");
        assert!(rows.len() <= 80, "{rows:?}");
        // The clock's minute.
        w.time = Some(TimeOfDay {
            hour: 10,
            minute: 22,
        });
        let rows = painter
            .paint(HomeView::of(&w), draw)
            .expect("a minute went");
        assert!(rows.len() <= 40, "{rows:?}");
        // An asking lamp on a mini place.
        w.devices[0].asking = true;
        let rows = painter
            .paint(HomeView::of(&w), draw)
            .expect("a device asks");
        assert!(rows.len() <= 80, "{rows:?}");
    }
}
