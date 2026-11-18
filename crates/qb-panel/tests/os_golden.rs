//! The OS screens' golden tests: each still as this crate paints it, compared with the committed
//! reference in `tests/reference/os-<name>.png`, and with the mockup's still of the same screen: round 8's
//! (`QBP_MOCKUP_DIR`, `r8-wall-<name>`) for the screens it drew, and round 9's (`QBP_MOCKUP9_DIR`,
//! `r9-<A|C>-<name>`) for the screens drawn as its concepts, the watch log and its slip from concept A and
//! the manifold from concept C (spec §12). Each case names its source.
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
//! Phases 1 and 2 as they are drawn: HOME in its three states, GATE, ABOUT, the log (`alerts`,
//! `alerts-none`) and its slip (`alert`), NAMES (`names`, `yours`, `name`, `name-locked`, `name-threat`),
//! and LIVE, which the Drain's own painter draws, with and without the gate ask over its field.
//!
//! Where this crate's screens differ from the round 9 drawing on purpose (spec §12), the differing region is
//! left out of the comparison, and listed on the case (`excluded`):
//!
//! - **§12.1, no device on a name:** NAME's left node reads `THE HOUSE` where the still has the device that
//!   asks most (`Living room TV`, `Sam's phone`): the node's inside is left out; its frame is compared.
//! - **§12.7, a threat's NAME:** the line under the mimic reads `On a threat list. Threats are never
//!   allowed.` where the still says `phishing`.
//! - **§12.3, §12.8:** the idle log's two lines and YOUR LIST's day words come from the world; the fixture
//!   gives them the still's words (`MON 21:40`, `TUE 00:00`, `MON`, `SUN`), so nothing differs.
//! - **SYSTEM (§12.3, §3.8):** under ROUTER stands the band `5 GHZ` where the still has the router's `2 MS`; the
//!   link's fifth dash is lit (`ceil(round((-54 + 100) / 5) / 2)` is 5) where the still lights four; the box's
//!   line says `LAST UPDATE CONFIRMED` with no date where the still adds `3 NOV`; the spare slot says `KEPT`
//!   where the still says `0.4.0`.
//! - **WI-FI (§12.3, §11.4):** the ninth dash is lit (`round((-54 + 100) / 5)` is 9) where the still lights eight; the
//!   second line says `upstream answers in 9 ms` where the still says `the router answers in 2 ms`.
//! - **VERSION (§3.8, §11.1):** the two lines say what runs and how the last update ended where round 8 has `up to date
//!   \u{b7} checked 09:00` and its note on the trial, and the right key `CHECK NOW` is gone.
//! - **LOCK (§3.8):** the two lines name the strong actions and where the PIN lives where round 8 has the old list.
//! - **UPDATE (§12.3):** the box of the slot that does not run has no version where the still writes both (`SLOT B ·
//!   0.4.2` while it is received, `SLOT A · 0.4.1` once the new build runs): the label of that box is left out.
//!   STARTING, NIGHT and the PIN pad (`r9-A-boot`, `r9-A-night`, `r9-A-pin`) are compared whole.

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
    Rect, blessing, close, diff, mockup_dir, mockup_frame, mockup9_dir, read_png, region_diff,
    write_png,
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
// The screens of Phases 1 and 2.

/// The mockup still a golden is compared with, and which folder it is in: round 8's (`QBP_MOCKUP_DIR`) or round
/// 9's (`QBP_MOCKUP9_DIR`, for the screens spec §12 draws as one of its concepts). The name is the still's,
/// without `@2x.png`.
#[derive(Clone, Copy)]
enum Mockup {
    /// `r8-wall-<screen>`: the wall home's screens.
    R8(&'static str),
    /// `r9-<A|C>-<screen>`: the screen as concept A (the log and its slip) or C (the manifold) draws it.
    R9(&'static str),
}

impl Mockup {
    /// The still's file name stem.
    const fn stem(self) -> &'static str {
        match self {
            Self::R8(stem) | Self::R9(stem) => stem,
        }
    }

    /// The still's path in its folder.
    fn path(self, dir: &Path) -> PathBuf {
        dir.join(format!("{}@2x.png", self.stem()))
    }
}

/// A screen still: its reference's name, the still whose world and place it is, and the mockup's
/// still it is compared with.
struct ScreenCase {
    /// The reference is `os-<name>.png`.
    name: &'static str,
    /// The still: its world, and where the panel is.
    still: Still,
    /// The mockup's still, and its folder.
    mockup: Mockup,
    /// Where this crate's screen is meant to differ from the mockup's, and so is left out of the
    /// comparison: rulings and the spec's deviations.
    excluded: &'static [Rect],
}

/// The inside of NAME's left node: `THE HOUSE` here, the device that asks most in the round 9 still (spec §12.1). The
/// node's frame, 2 px, is compared.
const NODE_INSIDE: Rect = (12, 150, 142, 44);

/// The line under a threat's mimic: `On a threat list. Threats are never allowed.` here, `On the phishing list. …` in the
/// round 9 still (spec §12.7).
const THREAT_LINE: Rect = (8, 212, 472, 20);

/// Under ROUTER: `5 GHZ` here (spec §12.3), the router's `2 MS` in the round 9 still.
const SYSTEM_BAND: Rect = (8, 188, 78, 20);

/// SYSTEM's link: the fifth dash is lit here, dark in the still (5 of 5 against 4 of 5).
const SYSTEM_FIFTH_DASH: Rect = (164, 149, 12, 6);

/// The box's line: the still adds ` 3 NOV` after `CONFIRMED` (spec §12.3: no date).
const SYSTEM_UPDATE_DATE: Rect = (398, 178, 70, 16);

/// The spare slot's figure: `KEPT` here, `0.4.0` in the still (spec §12.3: the Pi does not know the spare's version).
const SYSTEM_SPARE: Rect = (334, 138, 124, 26);

/// WI-FI's ninth dash: lit here, dark in the still (9 of 10 by the formula, against 8).
const WIFI_NINTH_DASH: Rect = (308, 119, 16, 10);

/// WI-FI's second line: `upstream answers in 9 ms` here, `the router answers in 2 ms` there (spec §11.4).
const WIFI_UPSTREAM: Rect = (8, 207, 300, 22);

/// The two lines of a plain reading, where the words are this crate's (VERSION, LOCK).
const READING_LINES: Rect = (0, 186, 480, 48);

/// The floor's right key, which VERSION does not have any more (`CHECK NOW`, spec §11.1).
const RIGHT_KEY: Rect = (244, 240, 228, 64);

/// UPDATE's right box's label while it is received or rolled back: `SLOT B` here, `SLOT B · 0.4.2` in the still (the
/// version is written only for the build that runs, spec §12.3).
const UPDATE_RIGHT_LABEL: Rect = (282, 90, 160, 20);

/// UPDATE's left box's label once the new build runs: `SLOT A` here, `SLOT A · 0.4.1` in the still.
const UPDATE_LEFT_LABEL: Rect = (30, 90, 160, 20);

/// The thirty screens: (the twenty-three:) HOME at rest, with alerts, with the gate open; GATE; ABOUT; ALERTS, none waiting, and
/// ALERT; DEVICES, DEVICE, CONFIRM and ITS GATE; NAMES, YOUR LIST and NAME in its four states; SYSTEM, WI-FI,
/// VERSION, UP, NIGHT HOURS and LOCK; STARTING, NIGHT and the PIN pad; UPDATE received, on trial, confirmed and rolled
/// back.
const SCREEN_CASES: [ScreenCase; 30] = [
    ScreenCase {
        name: "home",
        still: Still::Home,
        mockup: Mockup::R8("r8-wall-home"),
        excluded: &[],
    },
    ScreenCase {
        name: "home-alert",
        still: Still::HomeAlert,
        mockup: Mockup::R8("r8-wall-home-alert"),
        excluded: &[],
    },
    ScreenCase {
        name: "home-open",
        still: Still::HomeOpen,
        mockup: Mockup::R8("r8-wall-home-open"),
        excluded: &[],
    },
    ScreenCase {
        name: "gate",
        still: Still::Gate,
        mockup: Mockup::R8("r8-wall-gate"),
        excluded: &[],
    },
    ScreenCase {
        name: "about",
        still: Still::About,
        mockup: Mockup::R8("r8-wall-about"),
        excluded: &[],
    },
    ScreenCase {
        name: "system",
        still: Still::System,
        mockup: Mockup::R9("r9-C-system"),
        // Spec §12.3: the band under ROUTER, the fifth dash, no date on the box's line, `KEPT` for the spare.
        excluded: &[
            SYSTEM_BAND,
            SYSTEM_FIFTH_DASH,
            SYSTEM_UPDATE_DATE,
            SYSTEM_SPARE,
        ],
    },
    ScreenCase {
        name: "wifi",
        still: Still::Wifi,
        mockup: Mockup::R9("r9-C-detail"),
        // The ninth dash by the formula, and the upstream's answer time (spec §11.4).
        excluded: &[WIFI_NINTH_DASH, WIFI_UPSTREAM],
    },
    ScreenCase {
        name: "version",
        still: Still::Version,
        mockup: Mockup::R8("r8-wall-version"),
        // Spec §3.8: what runs and how the last update ended; no `CHECK NOW` (§11.1).
        excluded: &[READING_LINES, RIGHT_KEY],
    },
    ScreenCase {
        name: "uptime",
        still: Still::Uptime,
        mockup: Mockup::R8("r8-wall-uptime"),
        excluded: &[],
    },
    ScreenCase {
        name: "nighthours",
        still: Still::NightHours,
        mockup: Mockup::R8("r8-wall-nighthours"),
        excluded: &[],
    },
    ScreenCase {
        name: "lock",
        still: Still::Lock,
        mockup: Mockup::R8("r8-wall-lock"),
        // Spec §3.8, §7.2: the strong actions and where the PIN lives.
        excluded: &[READING_LINES],
    },
    ScreenCase {
        name: "alerts",
        still: Still::Alerts,
        mockup: Mockup::R9("r9-A-alerts"),
        excluded: &[],
    },
    ScreenCase {
        name: "alerts-none",
        still: Still::AlertsNone,
        mockup: Mockup::R9("r9-A-alerts-none"),
        excluded: &[],
    },
    ScreenCase {
        name: "alert",
        still: Still::Alert,
        mockup: Mockup::R9("r9-A-alert"),
        excluded: &[],
    },
    ScreenCase {
        name: "names",
        still: Still::Names,
        mockup: Mockup::R9("r9-C-names"),
        excluded: &[],
    },
    ScreenCase {
        name: "yours",
        still: Still::Yours,
        mockup: Mockup::R9("r9-C-yours"),
        excluded: &[],
    },
    ScreenCase {
        name: "name",
        still: Still::Name,
        mockup: Mockup::R9("r9-C-name"),
        // Spec §12.1: the left node says `THE HOUSE`, where the still has the device that asks most.
        excluded: &[NODE_INSIDE],
    },
    ScreenCase {
        name: "name-locked",
        still: Still::NameLocked,
        mockup: Mockup::R9("r9-C-name-locked"),
        excluded: &[NODE_INSIDE],
    },
    ScreenCase {
        name: "name-threat",
        still: Still::NameThreat,
        mockup: Mockup::R9("r9-C-name-threat"),
        // Spec §12.1 again, and §12.7: the line says `On a threat list.` where the still says `On the phishing list.`
        excluded: &[NODE_INSIDE, THREAT_LINE],
    },
    ScreenCase {
        name: "boot",
        still: Still::Boot,
        mockup: Mockup::R9("r9-A-boot"),
        excluded: &[],
    },
    ScreenCase {
        name: "night",
        still: Still::Night,
        mockup: Mockup::R9("r9-A-night"),
        excluded: &[],
    },
    ScreenCase {
        name: "pin",
        still: Still::Pin,
        mockup: Mockup::R9("r9-A-pin"),
        excluded: &[],
    },
    ScreenCase {
        name: "update-receiving",
        still: Still::UpdateReceiving,
        mockup: Mockup::R9("r9-C-update-receiving"),
        // Spec §12.3: a version only beside the slot that runs.
        excluded: &[UPDATE_RIGHT_LABEL],
    },
    ScreenCase {
        name: "update-trial",
        still: Still::UpdateTrial,
        mockup: Mockup::R9("r9-C-update-trial"),
        excluded: &[UPDATE_LEFT_LABEL],
    },
    ScreenCase {
        name: "update-confirmed",
        still: Still::UpdateConfirmed,
        mockup: Mockup::R9("r9-C-update-confirmed"),
        excluded: &[UPDATE_LEFT_LABEL],
    },
    ScreenCase {
        name: "update-rolledback",
        still: Still::UpdateRolledBack,
        mockup: Mockup::R9("r9-C-update-rolledback"),
        excluded: &[UPDATE_RIGHT_LABEL],
    },
    ScreenCase {
        name: "devices",
        still: Still::Devices,
        // Ruling R4: the wall is compared with the ADS PASS still, not the AD-FREE one.
        mockup: Mockup::R8("r8-wall-devices-adspass"),
        excluded: &[],
    },
    ScreenCase {
        name: "device",
        still: Still::Device,
        mockup: Mockup::R8("r8-wall-device"),
        // Ruling R4: the state key says `ON THIS DEVICE` over `ADS BLOCKED`, where the still has `THIS DEVICE IS`
        // over `BLOCKED`.
        excluded: &[(14, 246, 216, 52)],
    },
    ScreenCase {
        name: "confirm",
        still: Still::Confirm,
        mockup: Mockup::R8("r8-wall-confirm"),
        // Ruling R4: the slab's second line says `Ads pass now.` where the still has `Now ad-free.`, and the
        // left key `KEEP ADS PASS` where the still has `KEEP AD-FREE`.
        excluded: &[(12, 200, 456, 26), (14, 246, 216, 52)],
    },
    ScreenCase {
        name: "pause",
        still: Still::Pause,
        mockup: Mockup::R8("r8-wall-pause"),
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
fn os_system_renders_as_the_reference() {
    check_frame("system", &render_screen(screen_case("system")));
}

#[test]
fn os_wifi_renders_as_the_reference() {
    check_frame("wifi", &render_screen(screen_case("wifi")));
}

#[test]
fn os_version_renders_as_the_reference() {
    check_frame("version", &render_screen(screen_case("version")));
}

#[test]
fn os_uptime_renders_as_the_reference() {
    check_frame("uptime", &render_screen(screen_case("uptime")));
}

#[test]
fn os_nighthours_renders_as_the_reference() {
    check_frame("nighthours", &render_screen(screen_case("nighthours")));
}

#[test]
fn os_lock_renders_as_the_reference() {
    check_frame("lock", &render_screen(screen_case("lock")));
}

#[test]
fn os_boot_renders_as_the_reference() {
    check_frame("boot", &render_screen(screen_case("boot")));
}

#[test]
fn os_night_renders_as_the_reference() {
    check_frame("night", &render_screen(screen_case("night")));
}

#[test]
fn os_pin_renders_as_the_reference() {
    check_frame("pin", &render_screen(screen_case("pin")));
}

#[test]
fn os_update_receiving_renders_as_the_reference() {
    check_frame(
        "update-receiving",
        &render_screen(screen_case("update-receiving")),
    );
}

#[test]
fn os_update_trial_renders_as_the_reference() {
    check_frame("update-trial", &render_screen(screen_case("update-trial")));
}

#[test]
fn os_update_confirmed_renders_as_the_reference() {
    check_frame(
        "update-confirmed",
        &render_screen(screen_case("update-confirmed")),
    );
}

#[test]
fn os_update_rolledback_renders_as_the_reference() {
    check_frame(
        "update-rolledback",
        &render_screen(screen_case("update-rolledback")),
    );
}

#[test]
fn os_alert_renders_as_the_reference() {
    check_frame("alert", &render_screen(screen_case("alert")));
}

#[test]
fn os_alerts_renders_as_the_reference() {
    check_frame("alerts", &render_screen(screen_case("alerts")));
}

#[test]
fn os_alerts_none_renders_as_the_reference() {
    check_frame("alerts-none", &render_screen(screen_case("alerts-none")));
}

#[test]
fn os_names_renders_as_the_reference() {
    check_frame("names", &render_screen(screen_case("names")));
}

#[test]
fn os_yours_renders_as_the_reference() {
    check_frame("yours", &render_screen(screen_case("yours")));
}

#[test]
fn os_name_renders_as_the_reference() {
    check_frame("name", &render_screen(screen_case("name")));
}

#[test]
fn os_name_locked_renders_as_the_reference() {
    check_frame("name-locked", &render_screen(screen_case("name-locked")));
}

#[test]
fn os_name_threat_renders_as_the_reference() {
    check_frame("name-threat", &render_screen(screen_case("name-threat")));
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

/// The limits of a short bold word in a small box, 16 px white on black, mostly ink: BIG's, as for a larger word.
const SHORT_BOLD: Limits = BIG;

/// The limits of a rectangle whose edges fall between pixels (a pipe 13 px thick centred on a pixel corner): the
/// browser and this crate cover an edge pixel by the same share, to a level of rounding.
const FRACTIONAL: Limits = Limits {
    mean: 0.1,
    outliers: 0.0,
    tile: Some(0.5),
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

// ---------------------------------------------------------------------------------------------
// The round 9 screens: concept A's paper (the log, its slip) and concept C's manifold (NAMES, NAME).
//
// Flat rectangles on whole pixels are the mockup's exactly; so are the fractional ones the manifold's pipes and
// the log's cursor are (this crate covers a pixel by its share, as the browser does). Text is compared
// as the lane's is, white on black with the lane's limits (`TEXT`), black on white with `INVERSE`. The
// sprocket holes, the torn edges, the valves and the lock marks are circles and triangles, which the two
// antialias alike but not identically: the single-letter limits.

/// Tractor-feed paper (`paper`): the perforation's dashes, from the first at row 88, exactly, and the sprocket holes.
/// The log's torn top edge is `torn_top`.
fn paper_regions(torn_top: bool) -> Vec<Region> {
    let mut regions = vec![
        Region {
            what: "perforation".into(),
            rects: vec![(22, 88, 2, 232), (456, 88, 2, 232)],
            limits: EXACT,
        },
        Region {
            what: "sprocket holes".into(),
            rects: vec![(4, 80, 16, 240), (460, 80, 16, 240)],
            limits: LETTER,
        },
    ];
    if torn_top {
        regions.push(Region {
            what: "torn top edge".into(),
            rects: vec![(26, 76, 430, 8)],
            limits: TEXT,
        });
    }
    regions
}

/// The floor in A's dress (`keys9(.., 'A')`): the perforation above it exactly, the stubs' frames (without the
/// notches' rows) and the solid right key exactly, their words, and the notches. `right` is the right key: solid
/// white with a word, or nothing.
fn a_floor_regions(right_key: bool) -> Vec<Region> {
    let mut regions = vec![
        Region {
            what: "perforation above the keys".into(),
            rects: vec![(8, 235, 464, 2)],
            limits: EXACT,
        },
        Region {
            what: "left stub frame".into(),
            rects: vec![
                (14, 240, 208, 4),
                (14, 300, 208, 4),
                (8, 240, 4, 24),
                (8, 281, 4, 23),
                (232, 240, 4, 24),
                (232, 281, 4, 23),
            ],
            limits: EXACT,
        },
        Region {
            what: "left stub words".into(),
            rects: vec![(14, 246, 216, 52)],
            limits: TEXT,
        },
        Region {
            what: "notches".into(),
            rects: vec![(2, 264, 14, 16), (230, 264, 20, 16), (466, 264, 14, 16)],
            limits: LETTER,
        },
    ];
    if right_key {
        regions.push(Region {
            what: "right stub".into(),
            rects: vec![(252, 240, 212, 20), (252, 292, 212, 12)],
            limits: EXACT,
        });
        regions.push(Region {
            what: "right stub words".into(),
            rects: vec![(252, 260, 212, 32)],
            limits: INVERSE,
        });
    }
    regions
}

/// The floor in C's dress (`keys9(.., 'C')`): a 2 px frame round each key exactly, the words, and the right key
/// solid white (`solid`) or the dim outline of a key that is not offered.
fn c_floor_regions(solid: bool) -> Vec<Region> {
    let mut regions = vec![
        Region {
            what: "left key frame".into(),
            rects: ring((8, 240, 228, 64), 2),
            limits: EXACT,
        },
        Region {
            what: "left key words".into(),
            rects: vec![(10, 242, 224, 60)],
            limits: TEXT,
        },
    ];
    if solid {
        regions.push(Region {
            what: "right key".into(),
            rects: vec![(244, 240, 228, 20), (244, 292, 228, 12)],
            limits: EXACT,
        });
        regions.push(Region {
            what: "right key words".into(),
            rects: vec![(246, 260, 224, 32)],
            limits: INVERSE,
        });
    } else {
        regions.push(Region {
            what: "right outline frame".into(),
            rects: ring((244, 240, 228, 64), 2),
            limits: EXACT,
        });
        regions.push(Region {
            what: "right outline word".into(),
            rects: vec![(246, 242, 224, 60)],
            limits: TEXT,
        });
    }
    regions
}

/// The log of `entries` entries (the newest at the bottom, in the third place from the top) and its typed line.
fn log_regions(entries: usize) -> Vec<Region> {
    let mut regions = lane_regions();
    regions.extend(paper_regions(true));
    for k in 0..entries {
        let place = 2 - k;
        let y = 92 + place * 60;
        if place % 2 == 0 {
            regions.push(Region {
                what: format!("entry {k} blue bar"),
                rects: vec![
                    (26, y, 428, 6),
                    (26, y + 52, 428, 6),
                    (26, y + 6, 4, 46),
                    (448, y + 6, 6, 46),
                ],
                limits: EXACT,
            });
        }
        regions.push(Region {
            what: format!("entry {k} margin flag"),
            rects: vec![(30, y + 6, 6, 46)],
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("entry {k} precedence"),
            rects: vec![(44, y + 6, 172, 18)],
            limits: SHORT_BOLD,
        });
        regions.push(Region {
            what: format!("entry {k} device"),
            rects: vec![(216, y + 6, 232, 18)],
            limits: TEXT,
        });
        regions.push(Region {
            what: format!("entry {k} title"),
            rects: vec![(44, y + 24, 400, 30)],
            limits: TEXT,
        });
    }
    regions.push(Region {
        what: "typed line".into(),
        rects: vec![(44, 286, 246, 22)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "cursor".into(),
        rects: vec![(293, 286, 13, 20)],
        limits: LETTER,
    });
    regions.push(Region {
        what: "on the log".into(),
        rects: vec![(300, 288, 150, 18)],
        limits: TEXT,
    });
    regions
}

/// The open watch: its two lines, `NOTHING TO REPORT`, the typed line and the cursor.
fn quiet_log_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.extend(paper_regions(true));
    regions.push(Region {
        what: "the two lines".into(),
        rects: vec![(42, 110, 300, 18), (42, 136, 300, 18)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "nothing to report".into(),
        rects: vec![(41, 184, 300, 28)],
        limits: BIG,
    });
    regions.push(Region {
        what: "typed line".into(),
        rects: vec![(42, 224, 180, 24)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "cursor".into(),
        rects: vec![(225, 224, 14, 20)],
        limits: LETTER,
    });
    regions
}

/// The slip: its sides exactly, the torn edges, the head, the stamp's frame exactly and its word, the title, the
/// lines, and the floor in A's dress.
fn slip_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.extend(paper_regions(false));
    regions.push(Region {
        what: "slip sides".into(),
        rects: vec![(26, 90, 4, 134), (450, 90, 4, 134)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "torn edges".into(),
        rects: vec![(26, 82, 428, 8), (26, 224, 428, 8)],
        limits: INVERSE,
    });
    regions.push(Region {
        what: "slip head".into(),
        rects: vec![(38, 98, 200, 18)],
        limits: INVERSE,
    });
    regions.push(Region {
        what: "stamp frame".into(),
        rects: ring((340, 92, 104, 28), 3),
        limits: EXACT,
    });
    regions.push(Region {
        what: "stamp word".into(),
        rects: vec![(346, 98, 92, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "slip title".into(),
        rects: vec![(36, 124, 410, 38)],
        limits: BIG,
    });
    regions.push(Region {
        what: "slip lines".into(),
        rects: vec![(38, 164, 410, 60)],
        limits: INVERSE,
    });
    regions.extend(a_floor_regions(true));
    regions
}

/// NAMES or YOUR LIST: the header's two lines and its door, and three rows of pipe.
fn pipe_regions(rows: usize) -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "header lines".into(),
        rects: vec![(8, 88, 260, 16), (8, 110, 262, 16)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "door frame".into(),
        rects: ring((272, 80, 200, 48), 4),
        limits: EXACT,
    });
    regions.push(Region {
        what: "door label and count".into(),
        rects: vec![(286, 98, 150, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "door arrow".into(),
        rects: vec![(446, 94, 16, 20)],
        limits: LETTER,
    });
    for k in 0..rows {
        let y = 136 + 56 * k;
        regions.push(Region {
            what: format!("row {k} name"),
            rects: vec![(7, y + 3, 330, 28)],
            limits: BIG,
        });
        regions.push(Region {
            what: format!("row {k} figure"),
            rects: vec![(300, y + 4, 90, 22)],
            limits: BIG,
        });
        regions.push(Region {
            what: format!("row {k} pipe"),
            rects: vec![(8, y + 28, 384, 20)],
            limits: FRACTIONAL,
        });
        regions.push(Region {
            what: format!("row {k} valve"),
            rects: vec![(390, y + 18, 52, 40)],
            limits: LETTER,
        });
        regions.push(Region {
            what: format!("row {k} stub"),
            rects: vec![(440, y + 36, 32, 4)],
            limits: EXACT,
        });
    }
    regions
}

/// NAME: the title, the mimic from the house through the valve to the answer, the line, and the floor in C's dress.
/// `solid` says the right key is offered (a threat's is not).
fn name_regions(solid: bool) -> Vec<Region> {
    // A threat's plate stands where the stem would: its rivet is a circle, in the same box.
    let stem = if solid { EXACT } else { LETTER };
    let mut regions = lane_regions();
    regions.push(Region {
        what: "name".into(),
        rects: vec![(0, 84, 470, 40)],
        limits: BIG,
    });
    regions.push(Region {
        what: "left node frame".into(),
        rects: ring((8, 146, 150, 52), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "left node".into(),
        rects: vec![(10, 148, 146, 48)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "pipe".into(),
        rects: vec![(158, 169, 96, 6)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "stem and handle".into(),
        rects: vec![(286, 134, 4, 38), (268, 130, 40, 6)],
        limits: stem,
    });
    regions.push(Region {
        what: "valve".into(),
        rects: vec![(252, 144, 72, 56)],
        limits: LETTER,
    });
    regions.push(Region {
        what: "beyond the valve".into(),
        rects: vec![(322, 168, 30, 8)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "answer node frame".into(),
        rects: ring((350, 146, 122, 52), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "answer node".into(),
        rects: vec![(352, 148, 118, 48)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "padlock on the handle".into(),
        rects: vec![(314, 122, 20, 22)],
        limits: LETTER,
    });
    regions.push(Region {
        what: "line".into(),
        rects: vec![(8, 212, 472, 20)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "padlock on the key".into(),
        rects: vec![(266, 258, 28, 26)],
        limits: LETTER,
    });
    regions.extend(c_floor_regions(solid));
    regions
}

/// SYSTEM, the mimic: ROUTER's frame and word, the link's four dashes that are the still's and its caption and
/// word, the box's frame and its words, the two slots, the line and its rule, and the three keys under it. The
/// band, the fifth dash, the date of the update and the spare's figure are left out by the case.
fn system_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "router frame".into(),
        rects: ring((8, 124, 78, 56), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "router word".into(),
        rects: vec![(10, 148, 74, 16)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "band".into(),
        rects: vec![SYSTEM_BAND],
        limits: TEXT,
    });
    regions.push(Region {
        what: "link caption".into(),
        rects: vec![(96, 126, 80, 18)],
        limits: TEXT,
    });
    for i in 0..4 {
        regions.push(Region {
            what: format!("link dash {i}"),
            rects: vec![(92 + 18 * i, 149, 12, 6)],
            limits: EXACT,
        });
    }
    regions.push(Region {
        what: "fifth dash".into(),
        rects: vec![SYSTEM_FIFTH_DASH],
        limits: EXACT,
    });
    regions.push(Region {
        what: "link word".into(),
        rects: vec![(98, 166, 74, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "box frame".into(),
        rects: ring((180, 80, 292, 152), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "box title".into(),
        rects: vec![(190, 90, 100, 16), (400, 90, 64, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "slot A frame".into(),
        rects: ring((192, 112, 130, 56), 4),
        limits: EXACT,
    });
    regions.push(Region {
        what: "slot A words".into(),
        rects: vec![(196, 118, 122, 44)],
        limits: BIG,
    });
    regions.push(Region {
        what: "slot B frame".into(),
        rects: ring((330, 112, 130, 56), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "slot B label".into(),
        rects: vec![(334, 118, 122, 18)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "spare figure".into(),
        rects: vec![SYSTEM_SPARE],
        limits: BIG,
    });
    regions.push(Region {
        what: "update line".into(),
        rects: vec![(190, 178, 208, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "update date".into(),
        rects: vec![SYSTEM_UPDATE_DATE],
        limits: TEXT,
    });
    regions.push(Region {
        what: "rule".into(),
        rects: vec![(192, 198, 268, 2)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "readings".into(),
        rects: vec![(190, 208, 272, 16)],
        limits: TEXT,
    });
    for k in 0..3 {
        let key = (8 + 157 * k, 240, 150, 64);
        regions.push(Region {
            what: format!("key {k} frame"),
            rects: ring(key, 2),
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("key {k} caption"),
            rects: vec![(key.0 + 8, 250, 100, 16)],
            limits: TEXT,
        });
        regions.push(Region {
            what: format!("key {k} figure"),
            rects: vec![(key.0 + 6, 272, 130, 24)],
            limits: BIG,
        });
    }
    regions
}

/// WI-FI: the router and the box's frames, ROUTER's word, the ten dashes (the ninth is left out by the case), the
/// caption and the word, the two lines (the second left out), and the floor in C's dress.
fn wifi_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "router frame".into(),
        rects: ring((8, 92, 110, 64), 2),
        limits: EXACT,
    });
    regions.push(Region {
        what: "router word".into(),
        rects: vec![(10, 116, 106, 16)],
        limits: SHORT_BOLD,
    });
    regions.push(Region {
        what: "box".into(),
        rects: vec![(362, 92, 110, 20), (362, 136, 110, 20)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "box word".into(),
        rects: vec![(364, 116, 106, 16)],
        limits: INVERSE,
    });
    for i in 0..8 {
        regions.push(Region {
            what: format!("dash {i}"),
            rects: vec![(132 + 22 * i, 119, 16, 10)],
            limits: EXACT,
        });
    }
    regions.push(Region {
        what: "ninth dash".into(),
        rects: vec![WIFI_NINTH_DASH],
        limits: EXACT,
    });
    regions.push(Region {
        what: "tenth dash".into(),
        rects: vec![(132 + 22 * 9, 119, 16, 10)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "caption".into(),
        rects: vec![(150, 96, 180, 16)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "word".into(),
        rects: vec![(190, 142, 100, 22)],
        limits: BIG,
    });
    regions.push(Region {
        what: "signal line".into(),
        rects: vec![(0, 184, WIDTH, 20)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "upstream line".into(),
        rects: vec![WIFI_UPSTREAM],
        limits: TEXT,
    });
    regions.extend(c_floor_regions(true));
    regions
}

/// A plain reading (`reading()`): its label, figure and two lines, and the floor of round 8 (the left key a 4 px
/// frame, the right solid white when `right_key`).
fn reading_regions(right_key: bool) -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "label".into(),
        rects: vec![(0, 90, WIDTH, 20)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "lines".into(),
        rects: vec![(0, 186, WIDTH, 48)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "figure".into(),
        rects: vec![(0, 112, WIDTH, 72)],
        limits: BIG,
    });
    regions.extend(left_key_regions());
    if right_key {
        regions.extend(right_key_regions(INVERSE));
    }
    regions
}

/// NIGHT HOURS: two labels, two hours of 40 px, the four `-` and `+` keys' frames exactly and their glyphs, and
/// the floor.
fn night_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    for (k, y) in [80_usize, 160].into_iter().enumerate() {
        regions.push(Region {
            what: format!("label {k}"),
            rects: vec![(0, y + 6, 300, 20)],
            limits: TEXT,
        });
        regions.push(Region {
            what: format!("hour {k}"),
            rects: vec![(0, y + 32, 300, 36)],
            limits: BIG,
        });
        for (g, x) in [(0, 300), (1, 392)] {
            let key = (x, y + 4, 80, 64);
            regions.push(Region {
                what: format!("key {k}.{g} frame"),
                rects: ring(key, 4),
                limits: EXACT,
            });
            regions.push(Region {
                what: format!("key {k}.{g} glyph"),
                rects: vec![inside(key, 4)],
                limits: TEXT,
            });
        }
    }
    regions.extend(left_key_regions());
    regions.extend(right_key_regions(INVERSE));
    regions
}

/// STARTING: the lane, the paper, and each line's tick box exactly, its word, and the word at the right once it is done.
fn boot_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.extend(paper_regions(false));
    for k in 0..5 {
        let y = 84 + 44 * k;
        regions.push(Region {
            what: format!("line {k} tick box"),
            rects: vec![(34, y + 8, 24, 24)],
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("line {k} word"),
            rects: vec![(68, y + 8, 200, 28)],
            limits: TEXT,
        });
        regions.push(Region {
            what: format!("line {k} done word"),
            rects: vec![(340, y + 8, 110, 24)],
            limits: TEXT,
        });
    }
    regions
}

/// NIGHT: the paper's margins in the dark blue (the perforation exactly, the sprocket holes), the hour, the words under it.
fn night_watch_regions() -> Vec<Region> {
    vec![
        Region {
            what: "perforation".into(),
            rects: vec![(22, 4, 2, 316), (456, 4, 2, 316)],
            limits: EXACT,
        },
        Region {
            what: "sprocket holes".into(),
            rects: vec![(4, 4, 16, 316), (460, 4, 16, 316)],
            limits: LETTER,
        },
        Region {
            what: "the hour".into(),
            rects: vec![(90, 126, 300, 86)],
            limits: BIG,
        },
        Region {
            what: "night watch".into(),
            rects: vec![(170, 230, 140, 20)],
            limits: TEXT,
        },
    ]
}

/// The PIN pad: the lane, the divider's dashes exactly, the ask, the four boxes (frames and squares exactly), what is
/// signed for, the signature line exactly, and each of the twelve keys: its frame exactly and its glyph.
fn pin_regions() -> Vec<Region> {
    let mut regions = lane_regions();
    regions.push(Region {
        what: "divider".into(),
        rects: vec![(174, 80, 2, 228)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "ask".into(),
        rects: vec![(4, 92, 168, 20)],
        limits: TEXT,
    });
    for i in 0..4 {
        regions.push(Region {
            what: format!("box {i}"),
            rects: vec![(8 + 40 * i, 122, 32, 44)],
            limits: EXACT,
        });
    }
    regions.push(Region {
        what: "for".into(),
        rects: vec![(4, 184, 168, 90)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "signature line".into(),
        rects: vec![(8, 272, 150, 2)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "the owner".into(),
        rects: vec![(4, 280, 168, 20)],
        limits: TEXT,
    });
    for i in 0..12 {
        let key = ([184, 282, 381][i % 3], 80 + 58 * (i / 3), 91, 50);
        regions.push(Region {
            what: format!("key {i} frame"),
            rects: ring(key, if i == 9 { 4 } else { 2 }),
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("key {i} glyph"),
            rects: vec![inside(key, 6)],
            limits: if i == 9 { SHORT_BOLD } else { BIG },
        });
    }
    regions
}

/// UPDATE: the lane, the two slot boxes' frames and fill exactly and their words, the drops and the bus exactly, DNS's block
/// exactly with its word, dot and the notes either side.
fn update_regions(receiving: bool) -> Vec<Region> {
    let mut regions = lane_regions();
    // The slot boxes: a ring of the solid or the outline (4 px covers the trial's thick frame).
    for (side, x) in [("left", 24), ("right", 276)] {
        regions.push(Region {
            what: format!("{side} box frame"),
            rects: ring((x, 82, 180, 84), 2),
            limits: EXACT,
        });
        regions.push(Region {
            what: format!("{side} box label"),
            rects: vec![(x + 6, 90, 168, 20)],
            limits: TEXT,
        });
        regions.push(Region {
            what: format!("{side} box word"),
            rects: vec![(x + 5, 116, 170, 42)],
            limits: BIG,
        });
    }
    if receiving {
        // The fill, where the share's figure is not (it ends at x 358).
        regions.push(Region {
            what: "fill".into(),
            rects: vec![(362, 112, 90, 52)],
            limits: EXACT,
        });
    }
    regions.push(Region {
        what: "drops and bus".into(),
        rects: vec![(100, 167, 280, 74)],
        limits: EXACT,
    });
    regions.push(Region {
        what: "DNS block".into(),
        rects: ring((100, 242, 280, 60), 4),
        limits: EXACT,
    });
    regions.push(Region {
        what: "DNS dot".into(),
        rects: vec![(114, 262, 20, 20)],
        limits: LETTER,
    });
    regions.push(Region {
        what: "DNS word".into(),
        rects: vec![(140, 256, 230, 34)],
        limits: TEXT,
    });
    regions.push(Region {
        what: "notes".into(),
        rects: vec![(4, 256, 92, 40), (388, 256, 88, 40)],
        limits: TEXT,
    });
    regions
}

/// The regions a screen is compared by.
fn screen_regions(case: &ScreenCase) -> Vec<Region> {
    match case.name {
        "boot" => boot_regions(),
        "night" => night_watch_regions(),
        "pin" => pin_regions(),
        "update-receiving" => update_regions(true),
        "update-trial" | "update-confirmed" | "update-rolledback" => update_regions(false),
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
        "alerts" => log_regions(3),
        "alerts-none" => quiet_log_regions(),
        "alert" => slip_regions(),
        "names" => pipe_regions(3),
        "yours" => pipe_regions(2),
        "name" | "name-locked" => name_regions(true),
        "name-threat" => name_regions(false),
        "system" => system_regions(),
        "wifi" => wifi_regions(),
        "version" => reading_regions(false),
        "uptime" => reading_regions(true),
        "lock" => reading_regions(true),
        "nighthours" => night_regions(),
        other => panic!("no regions for {other}"),
    }
}

// It needs the mockup stills: set QBP_MOCKUP_DIR (round 8's) and QBP_MOCKUP9_DIR (round 9's) to their folders;
// a case whose folder is not named is left out.
#[test]
fn the_screens_match_the_mockup() {
    let (round8, round9) = (mockup_dir(), mockup9_dir());
    let mut bad = vec![];
    for case in &SCREEN_CASES {
        let folder = match case.mockup {
            Mockup::R8(_) => &round8,
            Mockup::R9(_) => &round9,
        };
        let Some(dir) = folder else {
            continue;
        };
        let ours = render_screen(case);
        let theirs = mockup_frame(&case.mockup.path(dir));
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

/// The states of the whole system, which have no clock and no slab, and so their own alterations.
const WHOLE_CASES: [&str; 7] = [
    "boot",
    "night",
    "pin",
    "update-receiving",
    "update-trial",
    "update-confirmed",
    "update-rolledback",
];

#[test]
fn the_metric_rejects_altered_whole_state_frames() {
    const BLACK: Rgb = [0, 0, 0];
    for name in WHOLE_CASES {
        let case = screen_case(name);
        let ours = render_screen(case);
        let reference = read_png(reference_path(case.name));
        let cases: [(&str, Vec<u8>); 3] = [
            ("the body missing", filled(&ours, (0, 72, 480, 248), BLACK)),
            (
                "the left half missing",
                filled(&ours, (0, 0, 240, 320), BLACK),
            ),
            ("the ground at 3, 3, 3", recoloured(&ours, BLACK, [3, 3, 3])),
        ];
        for (what, frame) in &cases {
            assert!(
                !close(&diff(frame, &reference)) || ours == *frame,
                "{}: {what} passed the golden limits",
                case.name
            );
        }
        // And a state's frame is not another's.
        for other in WHOLE_CASES.iter().filter(|&&other| other != name) {
            assert!(
                !close(&diff(&ours, &read_png(reference_path(other)))),
                "{name} and {other} are the same picture"
            );
        }
    }
}

#[test]
fn the_metric_rejects_altered_screen_frames() {
    const G2: Rgb = [0x8e, 0x8e, 0x8e];
    const WHITE: Rgb = [0xff, 0xff, 0xff];
    const BLACK: Rgb = [0, 0, 0];
    for case in SCREEN_CASES
        .iter()
        .filter(|case| !WHOLE_CASES.contains(&case.name))
    {
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
    // The stills are where a tap leads: HOME's gate key is GATE, its SYSTEM tile the mimic, its ALERTS tile the log, its NAMES tile NAMES.
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
    // (HOME's world holds no readings of the board, so SYSTEM is compared as it is drawn from HOME's own world.)
    let home = world(Still::Home);
    let system = screens::view_of(&Still::System.nav(&home), &home);
    assert_eq!(go(Still::Home, 400, 250), system);
    assert_eq!(go(Still::HomeAlert, 400, 130), of(Still::Alerts));
    // (HOME holds no names, so NAMES is compared as it is drawn from HOME's own world.)
    assert_eq!(
        go(Still::Home, 240, 250),
        screens::view_of(&Still::Names.nav(&home), &home)
    );
    // And from the lane's words.
    assert_eq!(go(Still::HomeAlert, 200, 40), of(Still::Alerts));
    assert_eq!(go(Still::Home, 400, 40), system);
    // A press that is not a door changes nothing.
    assert_eq!(go(Still::Home, 100, 74), of(Still::Home));
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

#[test]
fn a_press_through_alerts_and_names_gets_each_still_in_turn() {
    use qb_panel::os::Intent;
    use qb_panel::os::Verdict;
    // The view a still has, for the world it is taken in.
    let of = |still: Still| {
        let world = world(still);
        screens::view_of(&still.nav(&world), &world)
    };
    let at = |nav: &qb_panel::os::Nav, still: Still| {
        let world = world(still);
        screens::view_of(nav, &world)
    };
    // HOME's ALERTS tile → the log → its newest entry (the last printed) → the slip → the threat's name line.
    let home = world(Still::HomeAlert);
    let mut nav = Still::HomeAlert.nav(&home);
    assert_eq!(tap(&mut nav, &home, 400, 130), None);
    assert_eq!(at(&nav, Still::Alerts), of(Still::Alerts));
    assert_eq!(tap(&mut nav, &home, 200, 240), None);
    assert_eq!(at(&nav, Still::Alert), of(Still::Alert));
    assert_eq!(tap(&mut nav, &home, 200, 190), None);
    assert_eq!(at(&nav, Still::NameThreat), of(Still::NameThreat));
    // The threat has no key to allow it; BACK is the slip, and DISMISS acks it and is the log.
    let threat = world(Still::NameThreat);
    assert_eq!(tap(&mut nav, &threat, 400, 280), None);
    assert_eq!(tap(&mut nav, &threat, 100, 280), None);
    assert_eq!(at(&nav, Still::Alert), of(Still::Alert));
    assert_eq!(tap(&mut nav, &threat, 100, 280), Some(Intent::Ack(4)));
    assert_eq!(at(&nav, Still::Alerts), of(Still::Alerts));
    // HOME's NAMES tile → NAMES → its first pipe → NAME → ALLOW NAME, strong, and back to NAMES.
    let home = world(Still::Home);
    let mut nav = Still::Home.nav(&home);
    assert_eq!(tap(&mut nav, &home, 240, 250), None);
    assert_eq!(at(&nav, Still::Names), of(Still::Names));
    let names = world(Still::Names);
    assert_eq!(tap(&mut nav, &names, 100, 150), None);
    assert_eq!(at(&nav, Still::Name), of(Still::Name));
    let name = world(Still::Name);
    let intent = tap(&mut nav, &name, 400, 280).expect("ALLOW NAME");
    assert_eq!(
        intent,
        Intent::Name {
            name: "ads.example.net".into(),
            verdict: Verdict::Allow
        }
    );
    assert!(intent.is_strong());
    assert_eq!(at(&nav, Still::Names), of(Still::Names));
    // YOUR OWN → YOUR LIST.
    assert_eq!(tap(&mut nav, &name, 400, 100), None);
    assert_eq!(at(&nav, Still::Yours), of(Still::Yours));
}

#[test]
fn a_press_through_the_lock_gets_the_pin_still_and_a_right_pin_hands_the_intent_back() {
    use qb_panel::os::{Intent, Verdict, Verified, Whole, verified};
    let of = |still: Still| {
        let world = world(still);
        screens::view_of(&still.nav(&world), &world)
    };
    // NAME with the lock on: ALLOW NAME asks for the PIN instead of going out.
    let locked = world(Still::NameLocked);
    let mut nav = Still::NameLocked.nav(&locked);
    assert_eq!(tap(&mut nav, &locked, 400, 280), None);
    assert_eq!(nav.whole(), Some(Whole::Pin));
    let press_key = |nav: &mut qb_panel::os::Nav, world: &qb_panel::os::World, i: usize| {
        let key = qb_panel::os::screens::whole::pad_rect(i);
        tap(nav, world, key.x + 45, key.y + 25)
    };
    // After the digits `1` and `2` the pad is the still's.
    let pad = world(Still::Pin);
    for key in [0, 1] {
        assert_eq!(press_key(&mut nav, &pad, key), None);
    }
    assert_eq!(screens::view_of(&nav, &pad), of(Still::Pin));
    // The third digit is only typed; the fourth asks the lock to check them.
    assert_eq!(press_key(&mut nav, &pad, 2), None);
    let asked = press_key(&mut nav, &pad, 3).expect("the fourth digit asks to verify");
    assert!(matches!(asked, Intent::Verify(_)) && !asked.is_strong());
    // A right answer gives ALLOW NAME to carry out, and NAMES is under the pad again, as NAME's keys leave it.
    let allow = verified(&mut nav, Verified::Right).expect("the held intent");
    assert_eq!(
        allow,
        Intent::Name {
            name: "ads.example.net".into(),
            verdict: Verdict::Allow
        }
    );
    assert!(allow.is_strong());
    assert!(nav.whole().is_none() && nav.stack().is_empty());
    assert_eq!(nav.root(), Section::Names);
}

#[test]
fn a_press_through_system_gets_each_still_in_turn() {
    use qb_panel::os::Intent;
    // The view a still has, for the world it is taken in; the system stills share one set of readings.
    let of = |still: Still| {
        let world = world(still);
        screens::view_of(&still.nav(&world), &world)
    };
    // The mimic's doors lead to the readings; BACK is the mimic again.
    let system = world(Still::System);
    for (x, y, still) in [
        (47, 150, Still::Wifi),
        (300, 120, Still::Version),
        (300, 200, Still::Uptime),
        (80, 270, Still::NightHours),
        (240, 270, Still::Lock),
        (400, 270, Still::About),
    ] {
        let mut nav = Still::System.nav(&system);
        assert_eq!(tap(&mut nav, &system, x, y), None, "{still:?}");
        assert_eq!(
            screens::view_of(&nav, &system),
            of(still),
            "{still:?} from the mimic"
        );
        // NIGHT HOURS' left key is SLEEP NOW; every other reading's is BACK.
        if still != Still::NightHours {
            assert_eq!(tap(&mut nav, &system, 100, 280), None, "{still:?}");
            assert_eq!(
                screens::view_of(&nav, &system),
                of(Still::System),
                "{still:?}: BACK"
            );
        }
    }
    // The keys that ask: REJOIN, RESTART, SLEEP NOW, TURN OFF (with the lock on), and DONE with new hours.
    let mut nav = Still::Wifi.nav(&system);
    assert_eq!(tap(&mut nav, &system, 400, 280), Some(Intent::Rejoin));
    let mut nav = Still::Uptime.nav(&system);
    let restart = tap(&mut nav, &system, 400, 280).expect("RESTART");
    assert_eq!(restart, Intent::Restart);
    assert!(restart.is_strong());
    let mut nav = Still::NightHours.nav(&system);
    assert_eq!(tap(&mut nav, &system, 100, 280), Some(Intent::SleepNow));
    assert_eq!(tap(&mut nav, &system, 340, 196), None);
    assert_eq!(
        tap(&mut nav, &system, 400, 280),
        Some(Intent::NightHours { from: 23, to: 6 })
    );
    let mut locked = world(Still::Lock);
    locked.lock_on = true;
    let mut nav = Still::Lock.nav(&locked);
    // TURN OFF is strong: with the lock on it waits for the PIN, and a right one gives it.
    assert_eq!(tap(&mut nav, &locked, 400, 280), None);
    let off = qb_panel::os::verified(&mut nav, qb_panel::os::Verified::Right).expect("TURN OFF");
    assert_eq!(off, Intent::ClearPin);
    assert!(off.is_strong());
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
