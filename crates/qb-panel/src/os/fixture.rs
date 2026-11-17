//! The round 8 mockup's house, alerts, names and update as data, and a `World` for each of its stills.
//!
//! The mockup's `parts.js` holds `HOUSE`, `R8_ALERTS`, `TOP`, `YOURS` and `UPD`; `stage.js` holds the
//! `SCENES` and the `app.still` moment every still is taken at: devices 1 and 4 asking, the lamp
//! beat 0, the clock 10:21, 10,818 queries and 4,780 refused, 44 %. Everything here is invented: the
//! devices are the mockup's (`02:00:00:00:00:xx` addresses), the names are public ad networks and
//! `example` names.
//!
//! The fixture's device, alert and name records have no `Debug`, like the live types they stand for.
//!
//! A `Still` is what a still needs of the world and of where the panel is: its `World`, and the root
//! the lane names. The screens, the stack above the root and the Drain's own state (LIVE and its
//! gate ask) belong to the code that draws them.

use crate::drain::{Drain, Fixture, Stills, drain_fixture};
use crate::os::nav::{Change, Nav, Screen, Section};
use crate::os::world::{
    AlertRow, DeviceRow, DeviceState, Mac, Outcome, Pause, WatchedName, World, display_name,
};
use crate::state::{Share, TimeOfDay};

/// A device of the mockup's house: a label the owner gave it (often none), a hostname (often none), the
/// last octet of its address and its state.
#[derive(Clone, Copy)]
pub struct HouseEntry {
    /// The owner's label.
    pub label: Option<&'static str>,
    /// The hostname.
    pub host: Option<&'static str>,
    /// The last octet of its address.
    pub octet: u8,
    /// Its state. The mockup's `free` is `AdsPass`.
    pub state: DeviceState,
}

const fn entry(
    label: Option<&'static str>,
    host: Option<&'static str>,
    octet: u8,
    state: DeviceState,
) -> HouseEntry {
    HouseEntry {
        label,
        host,
        octet,
        state,
    }
}

/// The mockup's `HOUSE`, in the wall's order: ten devices, two blocked, the router and the Pi last.
pub const HOUSE: [HouseEntry; 10] = [
    entry(Some("Desk PC"), None, 12, DeviceState::AdsPass),
    entry(Some("Sam's phone"), None, 23, DeviceState::AdsPass),
    entry(Some("Living room TV"), None, 31, DeviceState::Blocked),
    entry(Some("Kid's tablet"), None, 40, DeviceState::Blocked),
    entry(None, Some("printer"), 57, DeviceState::AdsPass),
    entry(None, Some("doorbell"), 64, DeviceState::AdsPass),
    entry(None, None, 143, DeviceState::AdsPass),
    entry(None, None, 216, DeviceState::AdsPass),
    entry(Some("router"), None, 1, DeviceState::Always),
    entry(Some("qberrypi"), None, 2, DeviceState::Always),
];

/// The hardware address of a fixture device: `02:00:00:00:00:xx`, a locally administered address that is no one's, `xx` the octet.
pub const fn mac_of(octet: u8) -> Mac {
    [0x02, 0, 0, 0, 0, octet]
}

/// An alert of the mockup: what the lane, the list and the slab say of it.
#[derive(Clone, Copy)]
pub struct FixtureAlert {
    /// Whether it is high severity: a white slab.
    pub high: bool,
    /// The title.
    pub title: &'static str,
    /// What it is about, as the list's slab says it.
    pub name: &'static str,
    /// Who or when, as the list's slab says it.
    pub who: &'static str,
    /// When: `hh:mm`, or a weekday for an alert before today.
    pub at: &'static str,
    /// The device it is about, as an index into `HOUSE`.
    pub device: usize,
    /// The ALERT screen's three lines: the subject, then two lines.
    pub why: [&'static str; 3],
}

/// The mockup's `R8_ALERTS`, newest first.
pub const R8_ALERTS: [FixtureAlert; 4] = [
    FixtureAlert {
        high: true,
        title: "PHISHING BLOCKED",
        name: "secure-login-verify.top",
        who: "asked by Sam's phone",
        at: "10:19",
        device: 1,
        why: [
            "secure-login-verify.top",
            "Sam's phone asked 3 times by 10:19",
            "on the phishing list \u{b7} never answered",
        ],
    },
    FixtureAlert {
        high: false,
        title: "NEW DEVICE",
        name: "\u{b7}216 joined",
        who: "first seen 09:52",
        at: "09:52",
        device: 7,
        why: [
            "\u{b7}216 joined at 09:52",
            "no name yet \u{b7} ads pass by default",
            "it has asked 14 names since",
        ],
    },
    FixtureAlert {
        high: true,
        title: "ARP SPOOF",
        name: "\u{b7}143 answers as router",
        who: "3 times since 02:14",
        at: "02:14",
        device: 6,
        why: [
            "\u{b7}143 answers as router",
            "3 times since 02:14",
            "it claims the router's address",
        ],
    },
    FixtureAlert {
        high: false,
        title: "SUSPICIOUS LOOKUPS",
        name: "doorbell \u{b7} 38 random names",
        who: "in one minute, Mon 21:40",
        at: "MON",
        device: 5,
        why: [
            "doorbell \u{b7} 38 random names",
            "in one minute, Mon 21:40",
            "like x7kq2d.info and p0v9aa.net",
        ],
    },
];

/// Whether a name is refused by the lists, as the mockup's `kind` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// The lists refuse it (`ref`).
    Refused,
    /// The lists answer it (`ok`).
    Answered,
}

/// A name of the mockup's NAMES screens: the most refused, or on the owner's own list.
#[derive(Clone, Copy)]
pub struct FixtureName {
    /// The name.
    pub name: &'static str,
    /// How many times it was refused today.
    pub count: u32,
    /// The device that mostly asks it, as an index into `HOUSE`.
    pub by: usize,
    /// What the lists say of it.
    pub kind: ListKind,
    /// Whether the owner turned that round: an entry on the owner's list.
    pub flipped: bool,
    /// Since when, for an entry on the owner's list: a weekday.
    pub since: &'static str,
}

/// The mockup's `TOP`: the most refused names today.
pub const TOP: [FixtureName; 3] = [
    FixtureName {
        name: "doubleclick.net",
        count: 1204,
        by: 2,
        kind: ListKind::Refused,
        flipped: false,
        since: "",
    },
    FixtureName {
        name: "googlesyndication.com",
        count: 861,
        by: 2,
        kind: ListKind::Refused,
        flipped: false,
        since: "",
    },
    FixtureName {
        name: "app-measurement.com",
        count: 540,
        by: 1,
        kind: ListKind::Refused,
        flipped: false,
        since: "",
    },
];

/// The mockup's `YOURS`: the owner's own list, one name allowed though the lists refuse it, one refused though they do not.
pub const YOURS: [FixtureName; 2] = [
    FixtureName {
        name: "amazon-adsystem.com",
        count: 31,
        by: 0,
        kind: ListKind::Refused,
        flipped: true,
        since: "MON",
    },
    FixtureName {
        name: "reddit.com",
        count: 0,
        by: 3,
        kind: ListKind::Answered,
        flipped: true,
        since: "SUN",
    },
];

/// The mockup's update: the version it runs, the version coming, and the trial's length in seconds.
#[derive(Debug, Clone, Copy)]
pub struct Update {
    /// The version that runs.
    pub from: &'static str,
    /// The version that arrives.
    pub to: &'static str,
    /// The trial boot's length, in seconds.
    pub trial_secs: u16,
}

/// The mockup's `UPD`.
pub const UPD: Update = Update {
    from: "0.4.1",
    to: "0.4.2",
    trial_secs: 120,
};

/// The mockup's `recentOf`: the last two names a device asked, by the device's state.
pub fn recent_of(state: DeviceState) -> [(&'static str, Outcome); 2] {
    match state {
        DeviceState::Blocked => [
            ("googlesyndication.com", Outcome::Refused),
            ("i.ytimg.com", Outcome::Answered),
        ],
        DeviceState::AdsPass | DeviceState::Open { .. } => [
            ("doubleclick.net", Outcome::Passed),
            ("api.spotify.com", Outcome::Answered),
        ],
        DeviceState::Always => [
            ("pool.ntp.org", Outcome::Answered),
            ("archlinux.org", Outcome::Answered),
        ],
    }
}

/// The stills of the mockup that Phase 1 draws, named as `r8-wall-<name>@2x.png` and `os-<name>.png`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Still {
    /// HOME, all well.
    Home,
    /// HOME with four alerts waiting.
    HomeAlert,
    /// HOME with the gate open, 277 s left.
    HomeOpen,
    /// The gate's choices, from HOME.
    Gate,
    /// LIVE, the Drain.
    Live,
    /// LIVE with the gate's choices unfolded over the field.
    LiveGate,
    /// DEVICES, the wall, in the ADS PASS words.
    Devices,
    /// DEVICE: the living room TV (blocked).
    Device,
    /// CONFIRM on Sam's phone: block ads?
    Confirm,
    /// ITS GATE: open the living room TV's gate.
    Pause,
    /// ALERT: the newest, high, with its three lines.
    Alert,
    /// ABOUT, the reading of SYSTEM.
    About,
}

impl Still {
    /// Every still, in the order the mockup lists them.
    pub const ALL: [Self; 12] = [
        Self::Home,
        Self::HomeAlert,
        Self::HomeOpen,
        Self::Gate,
        Self::Live,
        Self::LiveGate,
        Self::Devices,
        Self::Device,
        Self::Confirm,
        Self::Pause,
        Self::Alert,
        Self::About,
    ];

    /// The still's name: the mockup's, and the reference's (`os-<name>.png`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::HomeAlert => "home-alert",
            Self::HomeOpen => "home-open",
            Self::Gate => "gate",
            Self::Live => "live",
            Self::LiveGate => "live-gate",
            Self::Devices => "devices",
            Self::Device => "device",
            Self::Confirm => "confirm",
            Self::Pause => "pause",
            Self::Alert => "alert",
            Self::About => "about",
        }
    }

    /// The still of this name, if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|still| still.name() == name)
    }

    /// The root the lane names on this still.
    pub const fn section(self) -> Section {
        match self {
            Self::Home | Self::HomeAlert | Self::HomeOpen => Section::Home,
            Self::Gate => Section::Gate,
            Self::Live | Self::LiveGate => Section::Live,
            Self::Devices | Self::Device | Self::Confirm | Self::Pause => Section::Devices,
            Self::Alert => Section::Alerts,
            Self::About => Section::System,
        }
    }

    /// Where the panel is on this still: the root the lane names, and the screens open above it.
    ///
    /// The ALERT screen is about the world's top alert; DEVICE is about the still's device, and CONFIRM and
    /// ITS GATE are opened over it as the mockup's `SCENES` open them: CONFIRM asks Sam's phone, whose ads
    /// pass, whether to block them; ITS GATE is the living room TV's.
    pub fn nav(self, world: &World) -> Nav {
        let mut nav = Nav::at(self.section());
        if self == Self::Alert
            && let Some(top) = &world.top_alert
        {
            nav.open(Screen::Alert(top.id));
        }
        if self == Self::LiveGate {
            nav.apply(Change::OpenGateAsk);
        }
        if let Some(mac) = self.device() {
            nav.open(Screen::Device(mac));
            match self {
                Self::Confirm => nav.open(Screen::Confirm { mac, pass: false }),
                Self::Pause => nav.open(Screen::ItsGate(mac)),
                _ => {}
            }
        }
        nav
    }

    /// The device the still's screen is about, if it is about one: the house's entry's address.
    pub fn device(self) -> Option<Mac> {
        let index = match self {
            Self::Device | Self::Pause => 2,
            Self::Confirm => 1,
            _ => return None,
        };
        HOUSE.get(index).map(|house| mac_of(house.octet))
    }
}

/// The Drain and its figures for the LIVE stills, `Still::Live` and `Still::LiveGate`: the Drain's `Normal`
/// scene (a refusal half melted over a full funnel, 10:21, 44 %, 4,780, as `app.settle` leaves it) and, for
/// the gate ask, its `Stills::gate_ask` as the `Nav` of the still holds it. `None` for every other still.
pub fn live_scene(still: Still) -> Option<(Drain, Stills)> {
    if still.section() != Section::Live {
        return None;
    }
    let (drain, mut stills) = drain_fixture(Fixture::Normal);
    stills.gate_ask = still.nav(&world(still)).gate_ask();
    Some((drain, stills))
}

/// The house as the wall's rows, with the devices at the indexes in `asking` asking now.
pub fn house_devices(asking: &[usize]) -> Vec<DeviceRow> {
    HOUSE
        .iter()
        .enumerate()
        .map(|(at, house)| {
            let (name, named) = display_name(house.label, house.host, house.octet);
            DeviceRow {
                mac: mac_of(house.octet),
                name,
                named,
                octet: house.octet,
                state: house.state,
                fixed: false,
                asking: asking.contains(&at),
            }
        })
        .collect()
}

/// The time of an `hh:mm` alert; `None` for a weekday.
fn hhmm(text: &str) -> Option<TimeOfDay> {
    let (hour, minute) = text.split_once(':')?;
    Some(TimeOfDay {
        hour: hour.parse().ok()?,
        minute: minute.parse().ok()?,
    })
}

/// The mockup's alerts as the world holds them: all waiting, the newest as the one to open. Ids count down from the newest, which has the highest.
fn with_alerts(world: &mut World) {
    let top = &R8_ALERTS[0];
    let waiting = u32::try_from(R8_ALERTS.len()).unwrap_or(u32::MAX);
    world.alerts_waiting = waiting;
    world.alert_newest = hhmm(top.at);
    world.top_alert = Some(AlertRow::new(
        u64::from(waiting),
        top.high,
        top.title,
        &top.why,
    ));
}

/// The world at `app.still`'s moment, which every still starts from: nothing waiting, the gate shut,
/// devices 1 and 4 asking, the lamp beat 0, the clock 10:21, 10,818 queries of which 4,780 refused (44 %).
fn at_the_moment() -> World {
    let (queries, refused) = (10_818_u32, 4_780_u32);
    World {
        time: Some(TimeOfDay {
            hour: 10,
            minute: 21,
        }),
        wifi_joined: true,
        queries_today: Some(queries),
        refused_today: Some(refused),
        share: Share::of(u64::from(refused), u64::from(queries)),
        list_names: Some(212_000),
        version: UPD.from.to_string(),
        uptime_days: Some(12),
        devices: house_devices(&[1, 4]),
        ..World::default()
    }
}

/// The world of a still: the moment, and what the scene adds (`stage.js`'s `SCENES`).
pub fn world(still: Still) -> World {
    let mut world = at_the_moment();
    match still {
        Still::HomeAlert | Still::Alert => with_alerts(&mut world),
        Still::HomeOpen => {
            world.blocking = Some(Pause {
                left_secs: 277,
                total_secs: 300,
            });
        }
        Still::Device => {
            // The device's screen watches it: its last two names.
            if let Some(device) = still.device().and_then(|mac| world.device(&mac)) {
                world.watched = recent_of(device.state)
                    .iter()
                    .map(|&(name, outcome)| WatchedName::new(name, outcome))
                    .collect();
            }
        }
        Still::Home
        | Still::Gate
        | Still::Live
        | Still::LiveGate
        | Still::Devices
        | Still::Confirm
        | Still::Pause
        | Still::About => {}
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::world::{MAX_DEVICES, Newcomer};

    #[test]
    fn the_house_is_the_mockups_ten_devices() {
        let rows = house_devices(&[]);
        assert_eq!(rows.len(), 10);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Desk PC",
                "Sam's phone",
                "Living room TV",
                "Kid's tablet",
                "printer",
                "doorbell",
                "\u{b7}143",
                "\u{b7}216",
                "router",
                "qberrypi"
            ]
        );
        assert_eq!(
            rows.iter().map(|row| row.named).collect::<Vec<_>>(),
            [true, true, true, true, true, true, false, false, true, true]
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.state == DeviceState::Blocked)
                .count(),
            2
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.state == DeviceState::Always)
                .count(),
            2
        );
        assert!(rows.len() <= MAX_DEVICES);
        // Every address is the fixture's own kind, and distinct.
        let mut macs: Vec<Mac> = rows.iter().map(|row| row.mac).collect();
        assert!(macs.iter().all(|mac| mac[..5] == [2, 0, 0, 0, 0]));
        macs.sort_unstable();
        macs.dedup();
        assert_eq!(macs.len(), rows.len());
    }

    #[test]
    fn the_moment_is_app_stills() {
        let world = world(Still::Home);
        assert_eq!(
            world.time,
            Some(TimeOfDay {
                hour: 10,
                minute: 21
            })
        );
        assert_eq!(world.queries_today, Some(10_818));
        assert_eq!(world.refused_today, Some(4_780));
        assert_eq!(world.share, Some(Share::Percent(44)));
        assert_eq!(world.version, "0.4.1");
        assert_eq!(world.uptime_days, Some(12));
        assert_eq!(world.devices.len(), 10);
        let asking: Vec<usize> = world
            .devices
            .iter()
            .enumerate()
            .filter(|(_, row)| row.asking)
            .map(|(at, _)| at)
            .collect();
        assert_eq!(asking, [1, 4], "Sam's phone and the printer");
        assert!(!world.beat, "the lamp beat is 0");
        assert!(!world.gate_open());
        assert_eq!(world.alerts_waiting, 0);
        assert_eq!(world.top_alert, None);
        assert_eq!(world.newcomer, Newcomer::AdsPass);
        assert!(world.watched.is_empty());
        assert!(!world.lock_on);
    }

    #[test]
    fn each_still_adds_what_its_scene_adds() {
        // The alert stills have four waiting, the newest first.
        for still in [Still::HomeAlert, Still::Alert] {
            let world = world(still);
            assert_eq!(world.alerts_waiting, 4, "{still:?}");
            assert_eq!(
                world.alert_newest,
                Some(TimeOfDay {
                    hour: 10,
                    minute: 19
                })
            );
            let top = world.top_alert.as_ref().expect("a top alert");
            assert!(top.high);
            assert_eq!(top.title, "PHISHING BLOCKED");
            assert_eq!(top.lines.len(), 3);
            assert_eq!(top.lines[0], "secure-login-verify.top");
            assert_eq!(top.id, 4);
        }
        // The open gate has 277 s left of five minutes.
        assert_eq!(
            world(Still::HomeOpen).blocking,
            Some(Pause {
                left_secs: 277,
                total_secs: 300
            })
        );
        // The device's screen watches the living room TV, which is blocked.
        let device = world(Still::Device);
        let names: Vec<(&str, Outcome)> = device
            .watched
            .iter()
            .map(|w| (w.name.as_str(), w.outcome))
            .collect();
        assert_eq!(
            names,
            [
                ("googlesyndication.com", Outcome::Refused),
                ("i.ytimg.com", Outcome::Answered)
            ]
        );
        // Nothing else is watched or waiting.
        for still in [
            Still::Home,
            Still::Gate,
            Still::Live,
            Still::LiveGate,
            Still::Devices,
            Still::Confirm,
            Still::Pause,
            Still::About,
        ] {
            let world = world(still);
            assert!(
                world.watched.is_empty() && world.alerts_waiting == 0 && !world.gate_open(),
                "{still:?}"
            );
        }
    }

    #[test]
    fn the_stills_have_unique_names_that_come_back() {
        let mut names: Vec<&str> = Still::ALL.iter().map(|s| s.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Still::ALL.len());
        for still in Still::ALL {
            assert_eq!(Still::from_name(still.name()), Some(still));
        }
        assert_eq!(Still::from_name("nope"), None);
        assert_eq!(
            Still::from_name("devices-adspass"),
            None,
            "ADS PASS is the only word (ruling R4)"
        );
    }

    #[test]
    fn a_still_says_its_root_and_its_device() {
        assert_eq!(Still::Home.section(), Section::Home);
        assert_eq!(Still::HomeOpen.section(), Section::Home);
        assert_eq!(Still::Gate.section(), Section::Gate);
        assert_eq!(Still::LiveGate.section(), Section::Live);
        assert_eq!(Still::Pause.section(), Section::Devices);
        assert_eq!(Still::Alert.section(), Section::Alerts);
        assert_eq!(Still::About.section(), Section::System);
        assert_eq!(Still::Device.device(), Some(mac_of(31)));
        assert_eq!(Still::Pause.device(), Some(mac_of(31)));
        assert_eq!(Still::Confirm.device(), Some(mac_of(23)));
        assert_eq!(Still::Home.device(), None);
        // A screen's device is on the wall.
        for still in Still::ALL {
            if let Some(mac) = still.device() {
                assert!(world(still).device(&mac).is_some(), "{still:?}");
            }
        }
    }

    #[test]
    fn a_still_says_where_the_panel_is() {
        for still in Still::ALL {
            let world = world(still);
            let nav = still.nav(&world);
            assert_eq!(nav.root(), still.section(), "{still:?}");
            assert!(nav.whole().is_none(), "{still:?}");
            // ALERT, and the screens over a device, have screens open above their root.
            let open: Vec<Screen> = match still {
                Still::Alert => vec![Screen::Alert(4)],
                Still::Device => vec![Screen::Device(mac_of(31))],
                Still::Confirm => vec![
                    Screen::Device(mac_of(23)),
                    Screen::Confirm {
                        mac: mac_of(23),
                        pass: false,
                    },
                ],
                Still::Pause => vec![Screen::Device(mac_of(31)), Screen::ItsGate(mac_of(31))],
                _ => vec![],
            };
            assert_eq!(nav.stack(), open.as_slice(), "{still:?}");
        }
        assert!(Still::Home.nav(&world(Still::Home)).is_home());
        assert!(Still::HomeAlert.nav(&world(Still::HomeAlert)).is_home());
    }

    #[test]
    fn the_names_and_the_update_are_the_mockups() {
        assert_eq!(R8_ALERTS.len(), 4);
        assert_eq!(R8_ALERTS[3].at, "MON");
        assert_eq!(
            hhmm("10:19"),
            Some(TimeOfDay {
                hour: 10,
                minute: 19
            })
        );
        assert_eq!(hhmm("MON"), None);
        assert_eq!(TOP[0].name, "doubleclick.net");
        assert_eq!(TOP.map(|t| t.count), [1204, 861, 540]);
        assert!(YOURS.iter().all(|y| y.flipped));
        assert_eq!(YOURS[0].kind, ListKind::Refused);
        assert_eq!(YOURS[1].kind, ListKind::Answered);
        assert_eq!((UPD.from, UPD.to, UPD.trial_secs), ("0.4.1", "0.4.2", 120));
        // The alerts' devices and the names' askers are in the house.
        assert!(R8_ALERTS.iter().all(|a| a.device < HOUSE.len()));
        assert!(TOP.iter().chain(&YOURS).all(|n| n.by < HOUSE.len()));
    }
}
