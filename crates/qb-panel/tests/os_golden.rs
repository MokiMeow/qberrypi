//! The OS screens' golden tests: each still as this crate paints it, compared with the committed
//! reference in `tests/reference/os-<name>.png`, and with the round 8 mockup's still of the same
//! screen when `QBP_MOCKUP_DIR` names the mockup's folder.
//!
//! The comparison is `os_common`'s (the one the Drain's goldens use): the mean over the frame, the
//! share of outlier pixels and the worst tile. The references are this crate's own output, looked at
//! beside the mockup's stills before they were committed; `the_lane_matches_the_mockup` and
//! `the_screens_match_the_mockup` compare
//! regions with those stills directly.
//!
//! To write the references again after a change of look, run the tests with `QBP_BLESS=1`: each golden
//! test then writes its reference from the rendering and passes. Look at the PNGs before committing them.
//!
//! The stills are the bare lane, on HOME and on DEVICES (the rest of the frame is black), and the screens of
//! Phase 1 as they are drawn: HOME in its three states, GATE, ABOUT and ALERT, and LIVE, which the Drain's
//! own painter draws, with and without the gate ask over its field.

use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use qb_panel::os::fixture::{Still, live_scene, world};
use qb_panel::os::screens::lane::{self, LaneView};
use qb_panel::os::screens::{self, View};
use qb_panel::os::{Section, tap};
use qb_panel::ui::ScreenPainter;
use qb_panel::{HEIGHT, Painter, WIDTH};

mod os_common;
use os_common::{
    Rect, blessing, close, diff, mockup_dir, mockup_frame, read_png, region_diff, write_png,
};

/// The lane's rows: everything above the first tile.
const LANE_ROWS: usize = 72;

/// A colour as the frame stores it, without the alpha.
type Rgb = [u8; 3];

/// A bare lane still: its reference's name, the root it is under, the still whose world it reads, and
/// the mockup's still it is compared with.
struct LaneCase {
    /// The reference is `os-<name>.png`.
    name: &'static str,
    /// The root the lane names.
    section: Section,
    /// The still the world comes from.
    still: Still,
    /// The mockup's still, without `@2x.png`.
    mockup: &'static str,
}

/// The bare lane on HOME, and on DEVICES.
const LANE_CASES: [LaneCase; 2] = [
    LaneCase {
        name: "lane-home",
        section: Section::Home,
        still: Still::Home,
        mockup: "r8-wall-home",
    },
    LaneCase {
        name: "lane-devices",
        section: Section::Devices,
        still: Still::Devices,
        // Ruling R4: DEVICES is compared with the ADS PASS still.
        mockup: "r8-wall-devices-adspass",
    },
];

/// The reference PNG of a still.
fn reference_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/reference")
        .join(format!("os-{name}.png"))
}

/// The bare lane as the screen painter paints it from scratch, as RGBA.
fn render(case: &LaneCase) -> Vec<u8> {
    let mut painter = ScreenPainter::new();
    let view = LaneView::of(case.section, &world(case.still));
    painter.paint(view, lane::draw);
    painter.rgba().to_vec()
}

/// Saves a rendering that missed its reference among the build's temporary files and says where.
fn save_actual(name: &str, rgba: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("qb-panel-os-{name}.png"));
    write_png(&path, rgba)?;
    Ok(path)
}

/// Checks a rendering against the reference of the same name, saving it if it is not close. With
/// `QBP_BLESS` set, it writes the reference from the rendering instead.
fn check_frame(name: &str, ours: &[u8]) {
    let path = reference_path(name);
    if blessing() {
        write_png(&path, ours).expect("the reference is written");
        return;
    }
    let d = diff(ours, &read_png(&path));
    if !close(&d) {
        match save_actual(name, ours) {
            Ok(path) => eprintln!("the rendering is in {}", path.display()),
            Err(error) => eprintln!("the rendering could not be saved: {error}"),
        }
    }
    assert!(
        close(&d),
        "{name}: mean_abs {:.3}, outliers {:.5}, worst_tile {:.2}",
        d.mean_abs,
        d.outliers,
        d.worst_tile
    );
}

/// Checks a bare lane still.
fn check(case: &LaneCase) {
    check_frame(case.name, &render(case));
}

#[test]
fn os_lane_home_renders_as_the_reference() {
    check(&LANE_CASES[0]);
}

#[test]
fn os_lane_devices_renders_as_the_reference() {
    check(&LANE_CASES[1]);
}

// ---------------------------------------------------------------------------------------------
// The screens of Phase 1.

/// A screen still: its reference's name, the still whose world and place it is, and the mockup's
/// still it is compared with.
struct ScreenCase {
    /// The reference is `os-<name>.png`.
    name: &'static str,
    /// The still: its world, and where the panel is.
    still: Still,
    /// The mockup's still, without `@2x.png`.
    mockup: &'static str,
    /// Where this crate's screen is meant to differ from the mockup's, and so is left out of the
    /// comparison: rulings and the spec's deviations.
    excluded: &'static [Rect],
}

/// The ten screens: HOME at rest, with alerts, with the gate open; GATE; ABOUT; ALERT; DEVICES, DEVICE, CONFIRM and ITS GATE.
const SCREEN_CASES: [ScreenCase; 10] = [
    ScreenCase {
        name: "home",
        still: Still::Home,
        mockup: "r8-wall-home",
        excluded: &[],
    },
    ScreenCase {
        name: "home-alert",
        still: Still::HomeAlert,
        mockup: "r8-wall-home-alert",
        excluded: &[],
    },
    ScreenCase {
        name: "home-open",
        still: Still::HomeOpen,
        mockup: "r8-wall-home-open",
        excluded: &[],
    },
    ScreenCase {
        name: "gate",
        still: Still::Gate,
        mockup: "r8-wall-gate",
        excluded: &[],
    },
    ScreenCase {
        name: "about",
        still: Still::About,
        mockup: "r8-wall-about",
        excluded: &[],
    },
    ScreenCase {
        name: "alert",
        still: Still::Alert,
        mockup: "r8-wall-alert",
        // Ruling R2: P1's ALERT has no `SEE DEVICE`, so the right half of the floor stays black.
        excluded: &[(244, 236, 236, 84)],
    },
    ScreenCase {
        name: "devices",
        still: Still::Devices,
        // Ruling R4: the wall is compared with the ADS PASS still, not the AD-FREE one.
        mockup: "r8-wall-devices-adspass",
        excluded: &[],
    },
    ScreenCase {
        name: "device",
        still: Still::Device,
        mockup: "r8-wall-device",
        // Ruling R4: the state key says `ON THIS DEVICE` over `ADS BLOCKED`, where the still has `THIS DEVICE IS`
        // over `BLOCKED`.
        excluded: &[(14, 246, 216, 52)],
    },
    ScreenCase {
        name: "confirm",
        still: Still::Confirm,
        mockup: "r8-wall-confirm",
        // Ruling R4: the slab's second line says `Ads pass now.` where the still has `Now ad-free.`, and the
        // left key `KEEP ADS PASS` where the still has `KEEP AD-FREE`.
        excluded: &[(12, 200, 456, 26), (14, 246, 216, 52)],
    },
    ScreenCase {
        name: "pause",
        still: Still::Pause,
        mockup: "r8-wall-pause",
        excluded: &[],
    },
];

/// The view of a screen still, as the nav and the world say.
fn view_of_case(case: &ScreenCase) -> View {
    let world = world(case.still);
    screens::view_of(&case.still.nav(&world), &world).expect("a screen the painter draws")
}

/// A screen as the screen painter paints it from scratch, as RGBA.
fn render_screen(case: &ScreenCase) -> Vec<u8> {
    let mut painter = ScreenPainter::new();
    painter.paint(view_of_case(case), screens::draw);
    painter.rgba().to_vec()
}

fn screen_case(name: &str) -> &'static ScreenCase {
    SCREEN_CASES
        .iter()
        .find(|case| case.name == name)
        .expect("a screen case of this name")
}

#[test]
fn os_home_renders_as_the_reference() {
    check_frame("home", &render_screen(screen_case("home")));
}

#[test]
fn os_home_alert_renders_as_the_reference() {
    check_frame("home-alert", &render_screen(screen_case("home-alert")));
}

#[test]
fn os_home_open_renders_as_the_reference() {
    check_frame("home-open", &render_screen(screen_case("home-open")));
}

#[test]
fn os_gate_renders_as_the_reference() {
    check_frame("gate", &render_screen(screen_case("gate")));
}

#[test]
fn os_about_renders_as_the_reference() {
    check_frame("about", &render_screen(screen_case("about")));
}

#[test]
fn os_alert_renders_as_the_reference() {
    check_frame("alert", &render_screen(screen_case("alert")));
}

#[test]
fn os_devices_renders_as_the_reference() {
    check_frame("devices", &render_screen(screen_case("devices")));
}

#[test]
fn os_device_renders_as_the_reference() {
    check_frame("device", &render_screen(screen_case("device")));
}

#[test]
fn os_confirm_renders_as_the_reference() {
    check_frame("confirm", &render_screen(screen_case("confirm")));
}

#[test]
fn os_pause_renders_as_the_reference() {
    check_frame("pause", &render_screen(screen_case("pause")));
}

/// The house and twenty more devices, `·101` to `·120`: thirty in all, three pages of the wall.
fn crowded_world() -> qb_panel::os::World {
    let mut crowd = world(Still::Devices);
    for octet in 101..=120_u8 {
        let (name, named) = qb_panel::os::world::display_name(None, None, octet);
        crowd.push_device(qb_panel::os::world::DeviceRow {
            mac: qb_panel::os::fixture::mac_of(octet),
            name,
            named,
            octet,
            state: if octet % 3 == 0 {
                qb_panel::os::world::DeviceState::Blocked
            } else {
                qb_panel::os::world::DeviceState::AdsPass
            },
            fixed: false,
            asking: octet == 105,
        });
    }
    crowd
}

/// A page of the wall of thirty devices, from scratch.
fn render_page(page: usize) -> Vec<u8> {
    let crowd = crowded_world();
    let mut nav = qb_panel::os::Nav::at(Section::Devices);
    nav.apply(qb_panel::os::Change::DevicesPage(page));
    let view = screens::view_of(&nav, &crowd).expect("the wall has a view");
    let mut painter = ScreenPainter::new();
    painter.paint(view, screens::draw);
    painter.rgba().to_vec()
}

/// The wall past eleven devices: the third page, with its `NEW ONES` after the eighth and `MORE · 22` for
/// the page turner. The mockup has no such still, so only the reference is compared.
#[test]
fn os_devices_page_renders_as_the_reference() {
    check_frame("devices-page", &render_page(2));
    // The first page is full: eleven devices and the turner.
    let first = render_page(0);
    let last = render_page(2);
    assert_ne!(first, last);
}

/// The frame with the rectangle painted one colour.
fn filled(frame: &[u8], (left, top, width, height): Rect, colour: Rgb) -> Vec<u8> {
    let mut out = frame.to_vec();
    for y in top..top + height {
        for x in left..left + width {
            let at = (y * WIDTH + x) * 4;
            out[at..at + 3].copy_from_slice(&colour);
        }
    }
    out
}

/// The frame with every pixel of exactly the colour `from` in the colour `to`.
fn recoloured(frame: &[u8], from: Rgb, to: Rgb) -> Vec<u8> {
    let mut out = frame.to_vec();
    for pixel in out.as_chunks_mut::<4>().0 {
        if pixel[..3] == from {
            pixel[..3].copy_from_slice(&to);
        }
    }
    out
}

#[test]
fn the_metric_rejects_altered_lane_frames() {
    const G2: Rgb = [0x8e, 0x8e, 0x8e];
    const WHITE: Rgb = [0xff, 0xff, 0xff];
    const BLACK: Rgb = [0, 0, 0];
    for case in &LANE_CASES {
        let ours = render(case);
        let reference = read_png(reference_path(case.name));
        let cases: [(&str, Vec<u8>); 5] = [
            (
                "the lit rail missing",
                filled(&ours, (0, 64, 480, 4), BLACK),
            ),
            (
                "the clock missing",
                filled(&ours, (360, 16, 120, 40), BLACK),
            ),
            ("HOME missing", filled(&ours, (0, 16, 96, 40), BLACK)),
            ("the clock in white", recoloured(&ours, G2, WHITE)),
            ("the ground at 3, 3, 3", recoloured(&ours, BLACK, [3, 3, 3])),
        ];
        for (what, frame) in &cases {
            assert!(
                !close(&diff(frame, &reference)) || ours == *frame,
                "{}: {what} passed the golden limits",
                case.name
            );
        }
    }
}

#[test]
fn the_lane_stills_have_nothing_but_the_lane() {
    for case in &LANE_CASES {
        let ours = render(case);
        let below = &ours[LANE_ROWS * WIDTH * 4..];
        assert!(
            below
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 0, 255]),
            "{}: the rest of the frame is black",
            case.name
        );
        assert_eq!(ours.len(), WIDTH * HEIGHT * 4);
    }
}

// ---------------------------------------------------------------------------------------------
// The comparison with the round 8 mockup.
//
// The lane is rails, a lit rail and letters. Rails are rects the browser fills on whole pixels, so
// they are the mockup's exactly. The letters are compared as the Drain's text is: the browser and this
// crate rasterise glyphs differently at their edges, so no letter is the same pixel for pixel, but a
// letter in the wrong place or the wrong letter is far over the limits. The numbers are printed.

/// The limits for a region of text: a wider piece of the lane, whose ink is a small share of its
/// pixels. The Drain's, since the lane's letters are the same letters.
const TEXT_MEAN_MAX: f64 = 4.0;
/// See `TEXT_MEAN_MAX`.
const TEXT_OUTLIERS_MAX: f64 = 0.035;
/// See `TEXT_MEAN_MAX`.
const TEXT_TILE_MAX: f64 = 70.0;

/// The limits for the box of one letter, which is mostly ink.
const BOX_MEAN_MAX: f64 = 10.0;
/// See `BOX_MEAN_MAX`.
const BOX_OUTLIERS_MAX: f64 = 0.10;

/// The rows a lane letter stands in, with room round it: its cap height stands on y 45.
const LETTER_ROWS: (usize, usize) = (22, 32);

/// What a region of the lane may differ from the mockup's by.
#[derive(Clone, Copy)]
struct Limits {
    /// The most the mean of the differences may be.
    mean: f64,
    /// The most the share of outlier pixels may be.
    outliers: f64,
    /// The most the worst tile may be; `None` for a box that is less than a tile.
    tile: Option<f64>,
}

/// The limits of a wide region of letters.
const TEXT: Limits = Limits {
    mean: TEXT_MEAN_MAX,
    outliers: TEXT_OUTLIERS_MAX,
    tile: Some(TEXT_TILE_MAX),
};

/// The limits of the box of one letter.
const LETTER: Limits = Limits {
    mean: BOX_MEAN_MAX,
    outliers: BOX_OUTLIERS_MAX,
    tile: None,
};

/// The limits of text of 30 px and more (a tile's figure, a slab's title, a reading's figure): the
/// browser and this crate differ by the same fringe at every edge, and a larger glyph has more edge
/// to the area, so the share and the mean are higher than for the lane's 24 px.
const BIG: Limits = Limits {
    mean: 8.0,
    outliers: 0.065,
    tile: Some(TEXT_TILE_MAX),
};

/// The limits of black words on a white tile: twice the contrast of the lane's light words on black,
/// so the same fringe at the edges is twice the difference.
const INVERSE: Limits = Limits {
    mean: 6.0,
    outliers: 0.05,
    tile: Some(TEXT_TILE_MAX),
};

/// The limits of a whole screen: mostly flat fills that are the mockup's exactly, and some text.
const WHOLE: Limits = Limits {
    mean: 2.5,
    outliers: 0.02,
    tile: Some(TEXT_TILE_MAX),
};

/// Every pixel the mockup's: the rails.
const EXACT: Limits = Limits {
    mean: 0.0,
    outliers: 0.0,
    tile: Some(0.0),
};

/// Whether every pixel of the rectangle is the ground.
fn is_blank(frame: &[u8], (left, top, width, height): Rect) -> bool {
    (top..top + height).all(|y| {
        (left..left + width).all(|x| {
            let at = (y * WIDTH + x) * 4;
            frame[at..at + 3] == [0, 0, 0]
        })
    })
}

/// Compares a region and prints the numbers; a message if it is out of the limits.
fn compare(
    name: &str,
    what: &str,
    (ours, theirs): (&[u8], &[u8]),
    rects: &[Rect],
    limits: Limits,
) -> Option<String> {
    let d = region_diff(ours, theirs, rects);
    eprintln!(
        "{name:13} {what:18} mean_abs {:.3} outliers {:.5} worst_tile {:.2}",
        d.mean_abs, d.outliers, d.worst_tile
    );
    let out = d.mean_abs > limits.mean
        || d.outliers > limits.outliers
        || limits.tile.is_some_and(|tile| d.worst_tile > tile);
    out.then(|| {
        format!(
            "{name}: {what}: mean_abs {:.3}, outliers {:.5}, worst_tile {:.2}",
            d.mean_abs, d.outliers, d.worst_tile
        )
    })
}

// It needs the mockup stills: set QBP_MOCKUP_DIR to their directory.
#[test]
fn the_lane_matches_the_mockup() {
    let Some(dir) = mockup_dir() else {
        return;
    };
    let mut bad = vec![];
    for case in &LANE_CASES {
        let ours = render(case);
        let theirs = mockup_frame(&dir.join(format!("{}@2x.png", case.mockup)));
        let frames = (ours.as_slice(), theirs.as_slice());
        // The whole lane region, for the numbers.
        bad.extend(compare(
            case.name,
            "lane (y 0 to 72)",
            frames,
            &[(0, 0, WIDTH, LANE_ROWS)],
            TEXT,
        ));
        // The rails and the lit rail: every pixel is the mockup's.
        let rails = [
            (0, 0, WIDTH, LETTER_ROWS.0 - 6),
            (0, 56, WIDTH, LANE_ROWS - 56),
        ];
        bad.extend(compare(case.name, "rails", frames, &rails, EXACT));
        // The letters, as a region and cell by cell.
        bad.extend(compare(
            case.name,
            "letters",
            frames,
            &[(0, 16, WIDTH, 40)],
            TEXT,
        ));
        for cell in 0..WIDTH / 24 {
            let rect = (cell * 24, LETTER_ROWS.0, 24, LETTER_ROWS.1);
            // A cell with no letter on either side is blank in both: nothing to compare.
            if is_blank(&ours, rect) && is_blank(&theirs, rect) {
                continue;
            }
            bad.extend(compare(
                case.name,
                &format!("cell {cell}"),
                frames,
                &[rect],
                LETTER,
            ));
        }
    }
    assert!(bad.is_empty(), "out of bounds:\n{}", bad.join("\n"));
}

/// `QBP_DUMP_DIR` names a directory to write the frames in, beside the mockup's, so a difference can be looked at.
#[test]
fn the_lane_frames_are_dumped_when_asked() {
    let Some(dir) = env::var_os("QBP_DUMP_DIR") else {
        return;
    };
    for case in &LANE_CASES {
        let path = Path::new(&dir).join(format!("os-{}.png", case.name));
        write_png(&path, &render(case))
            .unwrap_or_else(|e| panic!("{} is written: {e}", path.display()));
    }
}

/// The screens too.
#[test]
fn the_screen_frames_are_dumped_when_asked() {
    let Some(dir) = env::var_os("QBP_DUMP_DIR") else {
        return;
    };
    for case in &SCREEN_CASES {
        let path = Path::new(&dir).join(format!("os-{}.png", case.name));
        write_png(&path, &render_screen(case))
            .unwrap_or_else(|e| panic!("{} is written: {e}", path.display()));
    }
}

// ---------------------------------------------------------------------------------------------
// The screens against the mockup, by region.
//
// A screen is rectangles and text. A rectangle the browser fills on whole pixels is the mockup's
// exactly, so the frames of the tiles, slabs and keys are compared at no difference at all. Text
// is compared as the lane's is, a region at a time, with the Drain's text limits: the browser and
// this crate rasterise glyphs differently at their edges. The lamps and the mini wall's dots are
// circles, which the two antialias alike but not identically: the single-letter limits.

/// A region of a screen, what it is, and what it may differ from the mockup by.
struct Region {
    /// Its name, for the numbers.
    what: String,
    /// The rectangles it is made of.
    rects: Vec<Rect>,
    /// How much it may differ.
    limits: Limits,
}

/// The frame of a rectangle, `t` pixels thick, inside it: four strips.
fn ring((x, y, w, h): Rect, t: usize) -> Vec<Rect> {
    vec![
        (x, y, w, t),
        (x, y + h - t, w, t),
        (x, y + t, t, h - 2 * t),
        (x + w - t, y + t, t, h - 2 * t),
    ]
}

/// The rectangle shrunk by `t` on every side.
fn inside((x, y, w, h): Rect, t: usize) -> Rect {
    (x + t, y + t, w - 2 * t, h - 2 * t)
}

/// The frame less one rectangle: four bands round it, for a comparison that leaves a place out.
fn leaving_out((ex, ey, ew, eh): Rect) -> Vec<Rect> {
    vec![
        (0, 0, WIDTH, ey),
        (0, ey + eh, WIDTH, HEIGHT - ey - eh),
        (0, ey, ex, eh),
        (ex + ew, ey, WIDTH - ex - ew, eh),
    ]
}

/// The frame less some rectangles, as a few bands of rows: between the rectangles' top and bottom edges
/// each band is the width of the frame less the columns the rectangles in it cover.
fn leaving_out_all(excluded: &[Rect]) -> Vec<Rect> {
    let mut edges = vec![0, HEIGHT];
    for &(_, y, _, h) in excluded {
        edges.extend([y, y + h]);
    }
    edges.sort_unstable();
    edges.dedup();
    let mut kept = vec![];
    for rows in edges.windows(2) {
        let (top, bottom) = (rows[0], rows[1]);
        let mut covered: Vec<(usize, usize)> = excluded
            .iter()
            .filter(|&&(_, y, _, h)| y <= top && bottom <= y + h)
            .map(|&(x, _, w, _)| (x, x + w))
            .collect();
        covered.sort_unstable();
        let mut at = 0;
        for (from, to) in covered {
            if from > at {
                kept.push((at, top, from - at, bottom - top));
            }
            at = at.max(to);
        }
        if at < WIDTH {
            kept.push((at, top, WIDTH - at, bottom - top));
        }
    }
    kept
}

/// The lane's regions, as the lane's own test compares them: the rails exactly, and its letters.
fn lane_regions() -> Vec<Region> {
    vec![
        Region {
            what: "lane rails".into(),
            rects: vec![
                (0, 0, WIDTH, LETTER_ROWS.0 - 6),
                (0, 56, WIDTH, LANE_ROWS - 56),
            ],
            limits: EXACT,
        },
        Region {
            what: "lane letters".into(),
            rects: vec![(0, 16, WIDTH, 40)],
            limits: TEXT,
        },
    ]
}

/// HOME's tiles: each one's frame exactly, its words, its lamp and the mini wall.
fn home_regions(alerts_wait: bool) -> Vec<Region> {
    let mut regions = lane_regions();
    for i in 0..6 {
        let tile = (8 + (i % 3) * 157, 80 + (i / 3) * 116, 150, 108);
        regions.push(Region {
            what: format!("tile {i} frame"),
            rects: ring(tile, 6),
            limits: EXACT,
        });
        // The label and the line (16 px), then the figure (40 px), inside the frame and clear of the lamp.
        regions.push(Region {
            what: format!("tile {i} label and line"),
            rects: vec![
                (tile.0 + 6, tile.1 + 12, 120, 20),
                (tile.0 + 6, tile.1 + 82, 140, 18),
            ],
            // The ALERTS tile is white while alerts wait: black words on it.
            limits: if alerts_wait && i == 2 { INVERSE } else { TEXT },
        });
        regions.push(Region {
            what: format!("tile {i} figure"),
            rects: vec![(tile.0 + 6, tile.1 + 38, 140, 40)],
            limits: BIG,
        });
        regions.push(Region {
            what: format!("tile {i} lamp"),
            rects: vec![(tile.0 + 134 - 8, tile.1 + 21 - 8, 16, 16)],
            limits: LETTER,
        });
    }
    // DEVICES' mini wall: 76 by 23 at (tile + 66, tile + 45).
    regions.push(Region {
        what: "mini wall".into(),
        rects: vec![(165 + 66, 80 + 45, 76, 23)],
        limits: LETTER,
    });
    regions
}

/// The four keys of the gate's floor: each one's frame exactly, and its words.
fn four_keys_regions() -> Vec<Region> {
    let mut regions = vec![];
    for i in 0..4 {
        let key = (8 + 118 * i, 240, 110, 64);
        regions.push(Region {
            what: format!("key {i} frame"),
            rects: ring(key, 4),
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("key {i} words"),
            rects: vec![inside(key, 4)],
            limits: TEXT,
        });
    }
    regions
}

/// A slab: its frame (4 px when it is hollow, a ring of the same when solid) exactly, and its words.
fn slab_regions() -> Vec<Region> {
    let slab = (8, 80, 464, 152);
    vec![
        Region {
            what: "slab frame".into(),
            rects: ring(slab, 6),
            limits: EXACT,
        },
        Region {
            what: "slab title".into(),
            rects: vec![(12, 84, 456, 52)],
            limits: BIG,
        },
        Region {
            what: "slab lines".into(),
            rects: vec![(12, 136, 456, 90)],
            limits: TEXT,
        },
    ]
}

/// The floor's left key: its frame exactly, and its words.
fn left_key_regions() -> Vec<Region> {
    let key = (8, 240, 228, 64);
    vec![
        Region {
            what: "left key frame".into(),
            rects: ring(key, 6),
            limits: EXACT,
        },
        Region {
            what: "left key words".into(),
            rects: vec![inside(key, 6)],
            limits: TEXT,
        },
    ]
}

/// The floor's right key: its frame exactly, and its words (black on white when it is solid).
fn right_key_regions(words: Limits) -> Vec<Region> {
    let key = (244, 240, 228, 64);
    vec![
        Region {
            what: "right key frame".into(),
            rects: ring(key, 6),
            limits: EXACT,
        },
        Region {
            what: "right key words".into(),
            rects: vec![inside(key, 6)],
            limits: words,
        },
    ]
}

/// The wall of DEVICES: each place's frame exactly (a place with nothing in it is black), its words, its
/// lamp or its dot.
fn wall_regions() -> Vec<Region> {
    // Which places hold a name that is words, and which an octet in 24 px, in the house's order.
    let octets = [6, 7];
    let mut regions = lane_regions();
    for i in 0..12 {
        let place = (8 + (i % 4) * 118, 80 + (i / 4) * 77, 110, 70);
        regions.push(Region {
            what: format!("place {i} frame"),
            rects: ring(place, 6),
            limits: EXACT,
        });
        match i {
            // `NEW ONES` and the newcomer's word, 16 px in G3.
            10 => regions.push(Region {
                what: "new ones".into(),
                rects: vec![(place.0 + 6, place.1 + 8, 100, 40)],
                limits: TEXT,
            }),
            // The empty place's dot.
            11 => regions.push(Region {
                what: "empty place dot".into(),
                rects: vec![(place.0 + 47, place.1 + 27, 16, 16)],
                limits: LETTER,
            }),
            _ => {
                let name = if octets.contains(&i) {
                    (place.0 + 4, place.1 + 10, 70, 26)
                } else {
                    (place.0 + 6, place.1 + 8, 84, 38)
                };
                regions.push(Region {
                    what: format!("place {i} name"),
                    rects: vec![name],
                    limits: if octets.contains(&i) { BIG } else { TEXT },
                });
                regions.push(Region {
                    what: format!("place {i} state word"),
                    rects: vec![(place.0 + 6, place.1 + 48, 100, 18)],
                    limits: TEXT,
                });
                // The mockup's lamp is r 6 for the first quarter second of an ask (the printer's is
                // in the still); the panel's is r 5 whenever it asks, and r 2.5 otherwise.
                regions.push(Region {
                    what: format!("place {i} lamp"),
                    rects: vec![(place.0 + 89, place.1 + 5, 16, 16)],
                    limits: if i == 4 { BIGGER_LAMP } else { LETTER },
                });
            }
        }
    }
    regions
}

/// The limits of a lamp whose radius differs by one pixel from the mockup's: a ring of about 34 pixels of 256.
const BIGGER_LAMP: Limits = Limits {
    mean: 45.0,
    outliers: 0.25,
    tile: None,
};

/// DEVICE: its name and octet, the two rows of names, and the floor's two keys; the state key's words are
/// excluded by the case (ruling R4), so only its frame is compared.
fn device_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "name".into(),
        rects: vec![(0, 84, 400, 46)],
        limits: BIG,
    });
    regions.push(Region {
        what: "octet".into(),
        rects: vec![(400, 92, 76, 30)],
        limits: TEXT,
    });
    for k in 0..2 {
        let y = 132 + 52 * k;
        regions.push(Region {
            what: format!("row {k} square"),
            rects: vec![(8, y + 16, 16, 16)],
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("row {k} name"),
            rects: vec![(30, y + 10, 340, 34)],
            limits: BIG,
        });
        regions.push(Region {
            what: format!("row {k} outcome"),
            rects: vec![(380, y + 14, 96, 22)],
            limits: TEXT,
        });
    }
    regions.extend(left_key_regions());
    regions.extend(right_key_regions(TEXT));
    regions
}

/// CONFIRM: the slab's frame, title and first line (its second line is excluded by the case, ruling R4), and
/// the floor's keys.
fn confirm_regions() -> Vec<Region> {
    let slab = (8, 80, 464, 152);
    let mut regions = lane_regions();
    regions.push(Region {
        what: "slab frame".into(),
        rects: ring(slab, 6),
        limits: EXACT,
    });
    regions.push(Region {
        what: "slab title".into(),
        rects: vec![(12, 84, 456, 52)],
        limits: BIG,
    });
    regions.push(Region {
        what: "slab first line".into(),
        rects: vec![(12, 168, 456, 32)],
        limits: TEXT,
    });
    regions.extend(left_key_regions());
    regions.extend(right_key_regions(INVERSE));
    regions
}

/// The regions a screen is compared by.
fn screen_regions(case: &ScreenCase) -> Vec<Region> {
    match case.name {
        "devices" => wall_regions(),
        "device" => device_regions(),
        "confirm" => confirm_regions(),
        "pause" => {
            let mut regions = lane_regions();
            regions.extend(slab_regions());
            regions.extend(four_keys_regions());
            regions
        }
        "home" | "home-open" => home_regions(false),
        "home-alert" => home_regions(true),
        "gate" => {
            let mut regions = lane_regions();
            regions.extend(slab_regions());
            regions.extend(four_keys_regions());
            regions
        }
        "about" => {
            let mut regions = lane_regions();
            regions.push(Region {
                what: "label and lines".into(),
                rects: vec![(0, 90, WIDTH, 20), (0, 186, WIDTH, 48)],
                limits: TEXT,
            });
            regions.push(Region {
                what: "figure".into(),
                rects: vec![(0, 112, WIDTH, 72)],
                limits: BIG,
            });
            regions.extend(left_key_regions());
            regions
        }
        "alert" => {
            let mut regions = lane_regions();
            regions.extend(slab_regions());
            regions.extend(left_key_regions());
            regions
        }
        other => panic!("no regions for {other}"),
    }
}

// It needs the mockup stills: set QBP_MOCKUP_DIR to their directory.
#[test]
fn the_screens_match_the_mockup() {
    let Some(dir) = mockup_dir() else {
        return;
    };
    let mut bad = vec![];
    for case in &SCREEN_CASES {
        let ours = render_screen(case);
        let theirs = mockup_frame(&dir.join(format!("{}@2x.png", case.mockup)));
        let frames = (ours.as_slice(), theirs.as_slice());
        // The whole frame, less what a ruling or the spec changes.
        let everything = leaving_out_all(case.excluded);
        bad.extend(compare(
            case.name,
            "whole frame",
            frames,
            &everything,
            WHOLE,
        ));
        for region in screen_regions(case) {
            // A region inside an excluded place is not compared.
            if region
                .rects
                .iter()
                .any(|&(x, y, w, h)| case.excluded.iter().any(|&e| overlaps((x, y, w, h), e)))
            {
                continue;
            }
            bad.extend(compare(
                case.name,
                &region.what,
                frames,
                &region.rects,
                region.limits,
            ));
        }
    }
    assert!(bad.is_empty(), "out of bounds:\n{}", bad.join("\n"));
}

/// Whether two rectangles share a pixel.
fn overlaps((ax, ay, aw, ah): Rect, (bx, by, bw, bh): Rect) -> bool {
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

#[test]
fn the_metric_rejects_altered_screen_frames() {
    const G2: Rgb = [0x8e, 0x8e, 0x8e];
    const WHITE: Rgb = [0xff, 0xff, 0xff];
    const BLACK: Rgb = [0, 0, 0];
    for case in &SCREEN_CASES {
        let ours = render_screen(case);
        let reference = read_png(reference_path(case.name));
        let cases: [(&str, Vec<u8>); 5] = [
            (
                "the clock missing",
                filled(&ours, (360, 16, 120, 40), BLACK),
            ),
            (
                "the slab or the first row of tiles missing",
                filled(&ours, (0, 80, 480, 156), BLACK),
            ),
            ("the floor missing", filled(&ours, (0, 236, 480, 84), BLACK)),
            ("the clock in white", recoloured(&ours, G2, WHITE)),
            ("the ground at 3, 3, 3", recoloured(&ours, BLACK, [3, 3, 3])),
        ];
        for (what, frame) in &cases {
            assert!(
                !close(&diff(frame, &reference)) || ours == *frame,
                "{}: {what} passed the golden limits",
                case.name
            );
        }
    }
}

#[test]
fn a_press_on_the_tile_gets_the_screen_of_the_still() {
    // The stills are where a tap leads: HOME's gate key is GATE, its SYSTEM tile ABOUT, its ALERTS tile ALERT.
    let go = |from: Still, x: u16, y: u16| {
        let world = world(from);
        let mut nav = from.nav(&world);
        assert_eq!(tap(&mut nav, &world, x, y), None);
        screens::view_of(&nav, &world)
    };
    let of = |still: Still| {
        let world = world(still);
        screens::view_of(&still.nav(&world), &world)
    };
    assert_eq!(go(Still::Home, 80, 250), of(Still::Gate));
    assert_eq!(go(Still::Home, 400, 250), of(Still::About));
    assert_eq!(go(Still::HomeAlert, 400, 130), of(Still::Alert));
    // And from the lane's words.
    assert_eq!(go(Still::HomeAlert, 200, 40), of(Still::Alert));
    assert_eq!(go(Still::Home, 400, 40), of(Still::About));
    // A press that is not a door changes nothing.
    assert_eq!(go(Still::Home, 400, 130), of(Still::Home));
}

#[test]
fn a_press_through_the_device_screens_gets_each_still_in_turn() {
    // DEVICES' tile → DEVICE → its state key → CONFIRM; DEVICE → its gate key → ITS GATE; the lane's HOME and
    // word on the way. Each step's view is the still's (DEVICE's own world has the names it was watching).
    let wall = world(Still::Devices);
    let mut nav = Still::Devices.nav(&wall);
    // The living room TV is the first tile of the second... third place of the first row.
    assert_eq!(tap(&mut nav, &wall, 300, 110), None);
    let watching = world(Still::Device);
    assert_eq!(
        screens::view_of(&nav, &watching),
        screens::view_of(&Still::Device.nav(&watching), &watching)
    );
    assert_eq!(nav.watched_mac(), Still::Device.device());
    // Its gate key: ITS GATE, which watches nothing.
    let mut over_gate = nav.clone();
    assert_eq!(tap(&mut over_gate, &watching, 400, 280), None);
    let pause = world(Still::Pause);
    assert_eq!(
        screens::view_of(&over_gate, &pause),
        screens::view_of(&Still::Pause.nav(&pause), &pause)
    );
    assert_eq!(over_gate.watched_mac(), None);
    // `KEEP SHUT` is back to DEVICE, which watches again.
    assert_eq!(tap(&mut over_gate, &pause, 60, 280), None);
    assert_eq!(over_gate, nav);
    // Sam's phone: its tile, then the state key, is the confirm of the still.
    let mut nav = Still::Devices.nav(&wall);
    assert_eq!(tap(&mut nav, &wall, 180, 110), None);
    assert_eq!(tap(&mut nav, &wall, 100, 280), None);
    let confirm = world(Still::Confirm);
    assert_eq!(
        screens::view_of(&nav, &confirm),
        screens::view_of(&Still::Confirm.nav(&confirm), &confirm)
    );
    // `BLOCK ADS` asks for it and is back at DEVICE.
    let intent = tap(&mut nav, &confirm, 400, 280);
    assert_eq!(
        intent,
        Some(qb_panel::os::Intent::DeviceAds {
            mac: qb_panel::os::fixture::mac_of(23),
            pass: false
        })
    );
    assert_eq!(
        nav.top(),
        Some(&qb_panel::os::Screen::Device(
            qb_panel::os::fixture::mac_of(23)
        ))
    );
    // The lane's word goes back to the wall, HOME to HOME.
    assert_eq!(tap(&mut nav, &confirm, 200, 40), None);
    assert_eq!((nav.root(), nav.stack().len()), (Section::Devices, 0));
}

// ---------------------------------------------------------------------------------------------
// LIVE: the Drain, which has its own painter, with `HOME` in the lane's first four cells, and the gate ask.
//
// The Drain is compared with the round 8 mockup's LIVE stills by region, as its own golden compares it with
// round 6's: flat shapes (the rails, the gate, the key) pixel for pixel, text with the text limits, the
// funnel's dots with the limits of round shapes. The mockup's funnel and traffic are its own run's; the
// fixture's are the Drain's `Normal` scene, the same moment of the same model, so the funnel's runs may be
// shaded differently and are compared as a whole by how many dots are lit, not which shade.

/// A LIVE still: its reference's name, the still, and the mockup's still it is compared with.
struct LiveCase {
    /// The reference is `os-<name>.png`.
    name: &'static str,
    /// The still: `Still::Live` or `Still::LiveGate`.
    still: Still,
    /// The mockup's still, without `@2x.png`.
    mockup: &'static str,
}

/// LIVE, and LIVE with the gate ask.
const LIVE_CASES: [LiveCase; 2] = [
    LiveCase {
        name: "live",
        still: Still::Live,
        mockup: "r8-wall-live",
    },
    LiveCase {
        name: "live-gate",
        still: Still::LiveGate,
        mockup: "r8-wall-live-gate",
    },
];

/// LIVE as the Drain's painter paints it from scratch, as RGBA.
fn render_live(case: &LiveCase) -> Vec<u8> {
    let (drain, stills) = live_scene(case.still).expect("a LIVE still has a Drain");
    let mut painter = Painter::new();
    painter.paint(&drain, &stills);
    painter.rgba().to_vec()
}

fn live_case(name: &str) -> &'static LiveCase {
    LIVE_CASES
        .iter()
        .find(|case| case.name == name)
        .expect("a LIVE case of this name")
}

#[test]
fn os_live_renders_as_the_reference() {
    check_frame("live", &render_live(live_case("live")));
}

#[test]
fn os_live_gate_renders_as_the_reference() {
    check_frame("live-gate", &render_live(live_case("live-gate")));
}

/// Where the gate ask's field is, and the Drain's own field: the funnel and the count.
const FIELD: Rect = (120, 80, 360, 240);

/// The regions of LIVE that the Drain's golden compares with round 6's, and the ones round 8 adds.
fn live_regions(ask: bool) -> Vec<Region> {
    let mut regions = vec![
        // The rails on both sides of the lane, but for the cells `HOME` has: every pixel the mockup's.
        Region {
            what: "rails and gate column".into(),
            rects: vec![(0, 0, WIDTH, 8), (0, 64, WIDTH, 4), (96, 0, 24, HEIGHT)],
            limits: EXACT,
        },
        // `HOME`, the lane's first four cells, as the other screens' lanes set it.
        Region {
            what: "HOME".into(),
            rects: vec![(0, 16, 96, 40)],
            limits: TEXT,
        },
        // The left column: the share, its label, the clock, and the key's frame and words.
        Region {
            what: "left column text".into(),
            rects: vec![(0, 72, 96, 148)],
            limits: BIG,
        },
        Region {
            what: "key".into(),
            rects: vec![(8, 224, 80, 80)],
            limits: TEXT,
        },
        Region {
            what: "key frame".into(),
            rects: ring((8, 224, 80, 80), 4),
            limits: EXACT,
        },
        // The lane's tiles and their melt, right of the gate.
        Region {
            what: "lane right of the gate".into(),
            rects: vec![(120, 8, 360, 56)],
            limits: TEXT,
        },
    ];
    for cell in 0..4 {
        regions.push(Region {
            what: format!("HOME cell {cell}"),
            rects: vec![(cell * 24, LETTER_ROWS.0, 24, LETTER_ROWS.1)],
            limits: LETTER,
        });
    }
    if ask {
        let keys: Vec<Rect> = (0..3).map(|i| (128 + 117 * i, 224, 110, 80)).collect();
        regions.push(Region {
            what: "ask title".into(),
            rects: vec![(136, 90, 340, 44)],
            limits: BIG,
        });
        regions.push(Region {
            what: "ask lines".into(),
            rects: vec![(136, 142, 340, 70)],
            limits: TEXT,
        });
        for (i, key) in keys.iter().enumerate() {
            regions.push(Region {
                what: format!("ask key {i} frame"),
                rects: ring(*key, 4),
                limits: EXACT,
            });
            regions.push(Region {
                what: format!("ask key {i} words"),
                rects: vec![inside(*key, 4)],
                limits: TEXT,
            });
        }
        // Everything of the field that is not the ask's own text and keys is black: the funnel and the count are covered.
        regions.push(Region {
            what: "field margins".into(),
            rects: vec![(120, 80, 360, 10), (120, 214, 360, 10), (120, 80, 12, 240)],
            limits: EXACT,
        });
    } else {
        // The funnel: round shapes, as many lit as the mockup's, and the count.
        regions.push(Region {
            what: "count".into(),
            rects: vec![(128, 240, 352, 80)],
            limits: BIG,
        });
    }
    regions
}

// It needs the mockup stills: set QBP_MOCKUP_DIR to their directory.
#[test]
fn live_matches_the_mockup() {
    let Some(dir) = mockup_dir() else {
        return;
    };
    let mut bad = vec![];
    for case in &LIVE_CASES {
        let ours = render_live(case);
        let theirs = mockup_frame(&dir.join(format!("{}@2x.png", case.mockup)));
        let frames = (ours.as_slice(), theirs.as_slice());
        // The whole frame, less the funnel, whose runs are the mockup's own traffic's: the ask covers it.
        let everything = if case.still == Still::LiveGate {
            vec![(0, 0, WIDTH, HEIGHT)]
        } else {
            leaving_out((120, 80, 360, 144))
        };
        bad.extend(compare(
            case.name,
            "whole frame",
            frames,
            &everything,
            WHOLE,
        ));
        for region in live_regions(case.still == Still::LiveGate) {
            bad.extend(compare(
                case.name,
                &region.what,
                frames,
                &region.rects,
                region.limits,
            ));
        }
        if case.still == Still::Live {
            // The funnel's dots: every pixel of the bowl and the spout is a dot's, or black, in both.
            bad.extend(compare(
                case.name,
                "funnel",
                frames,
                &[(120, 80, 360, 144)],
                Limits {
                    mean: 12.0,
                    outliers: 0.25,
                    tile: None,
                },
            ));
        }
    }
    assert!(bad.is_empty(), "out of bounds:\n{}", bad.join("\n"));
}

/// The LIVE frames too.
#[test]
fn the_live_frames_are_dumped_when_asked() {
    let Some(dir) = env::var_os("QBP_DUMP_DIR") else {
        return;
    };
    for case in &LIVE_CASES {
        let path = Path::new(&dir).join(format!("os-{}.png", case.name));
        write_png(&path, &render_live(case))
            .unwrap_or_else(|e| panic!("{} is written: {e}", path.display()));
    }
}

#[test]
fn the_metric_rejects_altered_live_frames() {
    const G2: Rgb = [0x8e, 0x8e, 0x8e];
    const WHITE: Rgb = [0xff, 0xff, 0xff];
    const BLACK: Rgb = [0, 0, 0];
    for case in &LIVE_CASES {
        let ours = render_live(case);
        let reference = read_png(reference_path(case.name));
        let cases: [(&str, Vec<u8>); 5] = [
            ("HOME missing", filled(&ours, (0, 16, 96, 40), BLACK)),
            ("the key missing", filled(&ours, (8, 224, 80, 80), BLACK)),
            ("the gate missing", filled(&ours, (96, 0, 24, 304), BLACK)),
            ("HOME in white", recoloured(&ours, G2, WHITE)),
            ("the ground at 3, 3, 3", recoloured(&ours, BLACK, [3, 3, 3])),
        ];
        for (what, frame) in &cases {
            assert!(
                !close(&diff(frame, &reference)) || ours == *frame,
                "{}: {what} passed the golden limits",
                case.name
            );
        }
    }
    // The ask's keys are the one thing that tells the two stills apart below the lane.
    let ours = render_live(live_case("live-gate"));
    let reference = read_png(reference_path("live-gate"));
    assert!(!close(&diff(&filled(&ours, FIELD, BLACK), &reference)));
}

#[test]
fn the_ask_is_the_only_difference_between_the_two_live_stills() {
    let (live, ask) = (
        render_live(live_case("live")),
        render_live(live_case("live-gate")),
    );
    // Everything left of the field and above it is the same Drain.
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let in_field = x >= FIELD.0 && y >= FIELD.1;
            let at = (y * WIDTH + x) * 4;
            assert!(
                in_field || live[at..at + 4] == ask[at..at + 4],
                "({x}, {y})"
            );
        }
    }
    assert!(live != ask);
}
