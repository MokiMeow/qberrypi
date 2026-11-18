//! The mockup's house, alerts, names and update as data, and a `World` for each of its stills.
//!
//! The mockup's `parts.js` holds `HOUSE`, `R8_ALERTS`, `TOP`, `YOURS` and `UPD`, and round 9's `kit.js` the
//! alerts' stamps (`A9`) and the threat name (`THREAT`); `stage.js` holds the `SCENES` and the `app.still` moment
//! every still is taken at: devices 1 and 4 asking, the lamp beat 0, the log's cursor on (`blink` at frame 0),
//! the clock 10:21, 10,818 queries and 4,780 refused, 44 %. Everything here is invented: the devices are the
//! mockup's (`02:00:00:00:00:xx` addresses), the names are `example` names.
//!
//! The fixture's device, alert and name records have no `Debug`, like the live types they stand for.
//!
//! A `Still` is what a still needs of the world and of where the panel is: its `World`, and the root
//! the lane names. The screens, the stack above the root and the Drain's own state (LIVE and its
//! gate ask) belong to the code that draws them.

use crate::drain::{Drain, Fixture, Stills, drain_fixture};
use crate::os::nav::{Change, Nav, Screen, Section, Whole};
use crate::os::world::{
    AlertRow, AlertStamp, Band, DayWord, DeviceRow, DeviceState, LastUpdate, Link, Mac, Machine,
    Moment, NameClass, NameFacts, NameRow, Outcome, OwnerRow, OwnerVerdict, Pause, Slot,
    UpdatePhase, UpdateRead, WatchedName, Weekday, When, World, display_name,
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

/// An alert of the mockup: what the log, the slip and the stamp say of it (round 9's `R8_ALERTS` and `A9`).
#[derive(Clone, Copy)]
pub struct FixtureAlert {
    /// Whether it is high severity: `URGENT`, a white slip.
    pub high: bool,
    /// The title.
    pub title: &'static str,
    /// When: `hh:mm` today, or the weekday of an alert before today.
    pub at: When,
    /// What the Pi did about it, as the slip's stamp says (`A9`'s `stamp`).
    pub stamp: AlertStamp,
    /// The device it is about, as an index into `HOUSE`.
    pub device: usize,
    /// The name on the slip, for the alerts about a name: a door to NAME.
    pub name: Option<&'static str>,
    /// The slip's three lines: the subject, then two lines.
    pub why: [&'static str; 3],
}

/// The mockup's `R8_ALERTS`, newest first.
pub const R8_ALERTS: [FixtureAlert; 4] = [
    FixtureAlert {
        high: true,
        title: "PHISHING BLOCKED",
        at: When::Time(TimeOfDay {
            hour: 10,
            minute: 19,
        }),
        stamp: AlertStamp::Refused,
        device: 1,
        name: Some(THREAT.name),
        why: [
            "secure-login.example.org",
            "Sam's phone asked 3 times by 10:19",
            "on the phishing list \u{b7} never answered",
        ],
    },
    FixtureAlert {
        high: false,
        title: "NEW DEVICE",
        at: When::Time(TimeOfDay {
            hour: 9,
            minute: 52,
        }),
        stamp: AlertStamp::Admitted,
        device: 7,
        name: None,
        why: [
            "\u{b7}216 joined at 09:52",
            "no name yet \u{b7} ads pass by default",
            "it has asked 14 names since",
        ],
    },
    FixtureAlert {
        high: true,
        title: "ARP SPOOF",
        at: When::Time(TimeOfDay {
            hour: 2,
            minute: 14,
        }),
        stamp: AlertStamp::Observed,
        device: 6,
        name: None,
        why: [
            "\u{b7}143 answers as router",
            "3 times since 02:14",
            "it claims the router's address",
        ],
    },
    FixtureAlert {
        high: false,
        title: "SUSPICIOUS LOOKUPS",
        at: When::Day(Weekday::Mon),
        stamp: AlertStamp::Observed,
        device: 5,
        name: None,
        why: [
            "doorbell \u{b7} 38 random names",
            "in one minute, Mon 21:40",
            "like x7kq2d.example and p0v9aa.example",
        ],
    },
];

/// A name of the mockup's NAMES screens: one of the day's most refused.
#[derive(Clone, Copy)]
pub struct FixtureName {
    /// The name.
    pub name: &'static str,
    /// How many times it was refused today.
    pub count: u32,
    /// Which list holds it.
    pub class: NameClass,
}

/// The mockup's `TOP`: the most refused names today.
pub const TOP: [FixtureName; 3] = [
    FixtureName {
        name: "ads.example.net",
        count: 1204,
        class: NameClass::Ads,
    },
    FixtureName {
        name: "tracker.example.com",
        count: 861,
        class: NameClass::Ads,
    },
    FixtureName {
        name: "metrics.example.org",
        count: 540,
        class: NameClass::Ads,
    },
];

/// A name on the owner's own list, as the mockup's `YOURS` has it.
#[derive(Clone, Copy)]
pub struct FixtureOwn {
    /// The name.
    pub name: &'static str,
    /// Which list holds it, apart from the owner's entry.
    pub class: NameClass,
    /// What the owner decided.
    pub verdict: OwnerVerdict,
    /// The day it was set on the panel.
    pub day: DayWord,
}

/// The mockup's `YOURS`: the owner's own list, one name allowed though the lists refuse it, one refused though they do not.
pub const YOURS: [FixtureOwn; 2] = [
    FixtureOwn {
        name: "shop-ads.example.net",
        class: NameClass::Ads,
        verdict: OwnerVerdict::Allow,
        day: DayWord::On(Weekday::Mon),
    },
    FixtureOwn {
        name: "forum.example.com",
        class: NameClass::None,
        verdict: OwnerVerdict::Refuse,
        day: DayWord::On(Weekday::Sun),
    },
];

/// The mockup's `THREAT`: a name on a threat list, which the slip of the phishing alert names. Refused for every
/// device, and never to be allowed.
pub const THREAT: FixtureName = FixtureName {
    name: "secure-login.example.org",
    count: 3,
    class: NameClass::Threat,
};

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

/// The stills of the mockup that Phases 1 to 3 draw, named as `r8-wall-<name>@2x.png` or round 9's
/// `r9-<A|C>-<name>@2x.png`, and `os-<name>.png`.
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
    /// ALERTS: the watch log, four waiting, the newest three printed.
    Alerts,
    /// ALERTS with none waiting: nothing to report.
    AlertsNone,
    /// ALERT: the newest, high, as its slip with its stamp.
    Alert,
    /// NAMES: the day's three most refused, as the manifold.
    Names,
    /// YOUR LIST: the valves the owner set.
    Yours,
    /// NAME: `ads.example.net`, refused, with `ALLOW NAME`.
    Name,
    /// NAME with the lock on: the padlock on the strong key and on the valve's handle.
    NameLocked,
    /// NAME of a threat, from the alert's slip: sealed, `NEVER ALLOWED`.
    NameThreat,
    /// ABOUT, a reading over SYSTEM.
    About,
    /// SYSTEM: the mimic of the router, the link and the box.
    System,
    /// VERSION: the plain reading, `BACK` only.
    Version,
    /// WI-FI: the link drawn large, with `REJOIN`.
    Wifi,
    /// UP: the uptime, with `RESTART`.
    Uptime,
    /// NIGHT HOURS: `DARK FROM 23:00` until `07:00`.
    NightHours,
    /// LOCK: off, with `SET A PIN`.
    Lock,
    /// STARTING: the checklist, Wi-Fi under way.
    Boot,
    /// NIGHT: the dark clock at 02:14.
    Night,
    /// The PIN pad over NAME, `PIN TO ALLOW NAME`, two digits typed.
    Pin,
    /// UPDATE, receiving: 62 %.
    UpdateReceiving,
    /// UPDATE, on trial: 1:47 to go.
    UpdateTrial,
    /// UPDATE, confirmed.
    UpdateConfirmed,
    /// UPDATE, rolled back.
    UpdateRolledBack,
}

impl Still {
    /// Every still, in the order the mockup lists them.
    pub const ALL: [Self; 32] = [
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
        Self::Alerts,
        Self::AlertsNone,
        Self::Alert,
        Self::Names,
        Self::Yours,
        Self::Name,
        Self::NameLocked,
        Self::NameThreat,
        Self::About,
        Self::System,
        Self::Version,
        Self::Wifi,
        Self::Uptime,
        Self::NightHours,
        Self::Lock,
        Self::Boot,
        Self::Night,
        Self::Pin,
        Self::UpdateReceiving,
        Self::UpdateTrial,
        Self::UpdateConfirmed,
        Self::UpdateRolledBack,
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
            Self::Alerts => "alerts",
            Self::AlertsNone => "alerts-none",
            Self::Alert => "alert",
            Self::Names => "names",
            Self::Yours => "yours",
            Self::Name => "name",
            Self::NameLocked => "name-locked",
            Self::NameThreat => "name-threat",
            Self::About => "about",
            Self::System => "system",
            Self::Version => "version",
            Self::Wifi => "wifi",
            Self::Uptime => "uptime",
            Self::NightHours => "nighthours",
            Self::Lock => "lock",
            Self::Boot => "boot",
            Self::Night => "night",
            Self::Pin => "pin",
            Self::UpdateReceiving => "update-receiving",
            Self::UpdateTrial => "update-trial",
            Self::UpdateConfirmed => "update-confirmed",
            Self::UpdateRolledBack => "update-rolledback",
        }
    }

    /// The still of this name, if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|still| still.name() == name)
    }

    /// The root the lane names on this still. The states of the whole system draw their own lane: the root under
    /// them is where they came from, HOME for STARTING, UPDATE and NIGHT, and NAMES for the pad over NAME.
    pub const fn section(self) -> Section {
        match self {
            Self::Home
            | Self::HomeAlert
            | Self::HomeOpen
            | Self::Boot
            | Self::Night
            | Self::UpdateReceiving
            | Self::UpdateTrial
            | Self::UpdateConfirmed
            | Self::UpdateRolledBack => Section::Home,
            Self::Gate => Section::Gate,
            Self::Live | Self::LiveGate => Section::Live,
            Self::Devices | Self::Device | Self::Confirm | Self::Pause => Section::Devices,
            Self::Alerts | Self::AlertsNone | Self::Alert | Self::NameThreat => Section::Alerts,
            Self::Names | Self::Yours | Self::Name | Self::NameLocked | Self::Pin => Section::Names,
            Self::About
            | Self::System
            | Self::Version
            | Self::Wifi
            | Self::Uptime
            | Self::NightHours
            | Self::Lock => Section::System,
        }
    }

    /// Where the panel is on this still: the root the lane names, and the screens open above it.
    ///
    /// SYSTEM is its own root; its readings are opened over it (`System` is the mimic, the rest are what its doors open).
    /// The ALERT screen is about the world's top alert; NAME about the name the world holds facts for (the
    /// threat's, over the slip it was opened from); DEVICE is about the still's device, and CONFIRM and
    /// ITS GATE are opened over it as the mockup's `SCENES` open them: CONFIRM asks Sam's phone, whose ads
    /// pass, whether to block them; ITS GATE is the living room TV's.
    pub fn nav(self, world: &World) -> Nav {
        let mut nav = Nav::at(self.section());
        if matches!(self, Self::Alert | Self::NameThreat)
            && let Some(top) = world.top_alert()
        {
            nav.open(Screen::Alert(top.id));
        }
        match (self, &world.name_facts) {
            (Self::Name | Self::NameLocked | Self::NameThreat | Self::Pin, Some(facts)) => {
                nav.open(Screen::Name(facts.name.clone()));
            }
            (Self::Yours, _) => nav.open(Screen::Yours),
            (Self::About, _) => nav.open(Screen::About),
            (Self::Version, _) => nav.open(Screen::Version),
            (Self::Wifi, _) => nav.open(Screen::Wifi),
            (Self::Uptime, _) => nav.open(Screen::Uptime),
            (Self::NightHours, _) => nav.open(Screen::NightHours),
            (Self::Lock, _) => nav.open(Screen::Lock),
            _ => {}
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
        match self {
            Self::Boot => nav.set_whole(Some(Whole::Starting)),
            Self::Night => nav.set_whole(Some(Whole::Night)),
            Self::UpdateReceiving
            | Self::UpdateTrial
            | Self::UpdateConfirmed
            | Self::UpdateRolledBack => nav.set_whole(Some(Whole::Update)),
            // The mockup's `pin` scene, by the taps that make it: ALLOW NAME on the first of the day's names with the
            // lock on opens the pad holding it, and the digits `1` and `2` are typed.
            Self::Pin => {
                let press = |nav: &mut Nav, (x, y): (u16, u16)| {
                    let _ = crate::os::nav::tap(nav, world, x, y);
                };
                press(&mut nav, (400, 280));
                for digit in [1_usize, 2] {
                    let key = crate::os::screens::whole::pad_rect(digit - 1);
                    press(&mut nav, (key.x + 45, key.y + 25));
                }
            }
            _ => {}
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

/// The mockup's alerts as the world holds them: all four waiting, the newest three in the log. Ids count down
/// from the newest, which has the highest (4, 3, 2 and 1).
fn with_alerts(world: &mut World) {
    let waiting = u32::try_from(R8_ALERTS.len()).unwrap_or(u32::MAX);
    world.alerts_waiting = waiting;
    world.alert_newest = match R8_ALERTS[0].at {
        When::Time(time) => Some(time),
        When::Day(_) => None,
    };
    world.alerts = R8_ALERTS
        .iter()
        .zip((1..=u64::from(waiting)).rev())
        .take(crate::os::world::MAX_ALERT_ROWS)
        .map(|(alert, id)| {
            let mut row = AlertRow::new(id, alert.high, alert.title, &alert.why);
            row.at = Some(alert.at);
            row.who = world
                .devices
                .get(alert.device)
                .map_or_else(String::new, |device| device.name.clone());
            row.stamp = alert.stamp;
            row.name = alert.name.map(str::to_string);
            row.device = world.devices.get(alert.device).map(|device| device.mac);
            row
        })
        .collect();
}

/// The facts of a name of the fixture, as the loop would read them: no entry of the owner's, the day's count.
fn facts_of(name: FixtureName) -> NameFacts {
    NameFacts {
        name: name.name.to_string(),
        class: name.class,
        owner: None,
        count: Some(name.count),
    }
}

/// The mockup's names as the world holds them: the day's three most refused, and the owner's two entries
/// newest first (as `m.yours.unshift` keeps them: the one set last is first). `owner_total` says two.
fn with_names(world: &mut World) {
    world.top_names = TOP
        .iter()
        .map(|top| NameRow {
            name: top.name.to_string(),
            count: top.count,
            open: false,
        })
        .collect();
    world.owner_total = u32::try_from(YOURS.len()).unwrap_or(u32::MAX);
    world.owner_newest = YOURS
        .iter()
        .map(|own| OwnerRow {
            name: own.name.to_string(),
            verdict: own.verdict,
            day: Some(own.day),
        })
        .collect();
}

/// What SYSTEM and its readings read, as `cc.js`'s sample has it and as the Pi would fill it: the link at -54 dBm on
/// 5 GHz, held 12 days, the upstream answering in 9 ms (spec §11.4, not the router's 2 ms); slot A runs 0.4.1, the
/// last update was confirmed; up 12 days and 4 hours at a load of 0.21, memory 38 %, 47 degrees; the night hours
/// 23 to 07; the lock off.
fn with_system(world: &mut World) {
    world.link = Link {
        band: Band::from_mhz(5_180),
        signal_dbm: Some(-54),
        linked_days: Some(12),
        upstream_ms: Some(9),
    };
    world.machine = Machine {
        slot: Some(Slot::A),
        built: "2026-11-12".to_string(),
        last_update: LastUpdate::Confirmed,
        uptime_hours: Some(4),
        load_hundredths: Some(21),
        memory_pct: Some(38),
        temp_c: Some(47),
        last_restart: format!("the update to {}", UPD.from),
    };
    world.night = Some((23, 7));
}

/// The update `stage.js`'s scenes play, from slot A to slot B: where it stands, and the version that runs (the
/// old one while it is received and after a rollback, the new one on trial and once confirmed).
fn with_update(world: &mut World, phase: UpdatePhase, running: &str) {
    world.update = Some(UpdateRead {
        phase,
        before: Slot::A,
        after: Slot::B,
    });
    world.version = running.to_string();
    world.update_busy = matches!(
        phase,
        UpdatePhase::Receiving { .. } | UpdatePhase::Trial { .. }
    );
    world.machine.slot = Some(if running == UPD.to { Slot::B } else { Slot::A });
}

/// The world at `app.still`'s moment, which every still starts from: nothing waiting, the gate shut,
/// devices 1 and 4 asking, the lamp beat 0, the log's cursor on, the clock 10:21, 10,818 queries of which 4,780
/// refused (44 %).
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
        cursor_on: true,
        ..World::default()
    }
}

/// The world of a still: the moment, and what the scene adds (`stage.js`'s `SCENES`).
pub fn world(still: Still) -> World {
    let mut world = at_the_moment();
    match still {
        Still::HomeAlert | Still::Alerts | Still::Alert => with_alerts(&mut world),
        Still::AlertsNone => {
            // The store's last alert was Monday at 21:40, and this boot's watch opened at midnight on Tuesday.
            world.last_entry = Some(Moment {
                day: Weekday::Mon,
                time: TimeOfDay {
                    hour: 21,
                    minute: 40,
                },
            });
            world.watch_opened = Some(Moment {
                day: Weekday::Tue,
                time: TimeOfDay { hour: 0, minute: 0 },
            });
        }
        Still::Names | Still::Yours => with_names(&mut world),
        Still::Name | Still::NameLocked | Still::Pin => {
            with_names(&mut world);
            world.name_facts = Some(facts_of(TOP[0]));
            world.lock_on = matches!(still, Still::NameLocked | Still::Pin);
        }
        Still::NameThreat => {
            // Opened from the phishing alert's slip: the alert is waiting, and the threat's facts are read.
            with_alerts(&mut world);
            world.name_facts = Some(facts_of(THREAT));
        }
        Still::System
        | Still::Version
        | Still::Wifi
        | Still::Uptime
        | Still::NightHours
        | Still::Lock => with_system(&mut world),
        Still::HomeOpen => {
            world.blocking = Some(Pause {
                left_secs: 277,
                total_secs: 300,
            });
        }
        Still::Boot => {
            // `m.boot = 12`: the panel is on and Wi-Fi is the line under way; the others are still to come.
            world.wifi_joined = false;
            world.time = None;
            world.list_names = None;
            world.dns_answering = false;
        }
        Still::Night => {
            // `m.nightClock = 2 * 3600 + 14 * 60`.
            world.time = Some(TimeOfDay {
                hour: 2,
                minute: 14,
            });
        }
        Still::UpdateReceiving => with_update(
            &mut world,
            UpdatePhase::Receiving {
                received: 3_100_000,
                length: 5_000_000,
            },
            UPD.from,
        ),
        Still::UpdateTrial => with_update(
            &mut world,
            UpdatePhase::Trial {
                left_secs: 107,
                waiting_for: None,
            },
            UPD.to,
        ),
        Still::UpdateConfirmed => with_update(&mut world, UpdatePhase::Confirmed, UPD.to),
        Still::UpdateRolledBack => with_update(&mut world, UpdatePhase::RolledBack, UPD.from),
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
        assert!(world.alerts.is_empty() && world.top_alert().is_none());
        assert!(world.cursor_on, "the log's cursor is on at frame 0");
        assert_eq!(world.newcomer, Newcomer::AdsPass);
        assert!(world.watched.is_empty());
        assert!(!world.lock_on);
    }

    #[test]
    fn each_still_adds_what_its_scene_adds() {
        // The alert stills have four waiting, the newest first.
        for still in [Still::HomeAlert, Still::Alerts, Still::Alert] {
            let world = world(still);
            assert_eq!(world.alerts_waiting, 4, "{still:?}");
            assert_eq!(
                world.alert_newest,
                Some(TimeOfDay {
                    hour: 10,
                    minute: 19
                })
            );
            let top = world.top_alert().expect("a top alert");
            assert!(top.high);
            assert_eq!(top.title, "PHISHING BLOCKED");
            assert_eq!(top.lines.len(), 3);
            assert_eq!(top.lines[0], "secure-login.example.org");
            assert_eq!(top.stamp, AlertStamp::Refused);
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
        for still in [Still::Alerts, Still::AlertsNone, Still::NameThreat] {
            assert_eq!(still.section(), Section::Alerts, "{still:?}");
        }
        for still in [
            Still::Names,
            Still::Yours,
            Still::Name,
            Still::NameLocked,
            Still::Pin,
        ] {
            assert_eq!(still.section(), Section::Names, "{still:?}");
        }
        // The other states of the whole system are over HOME.
        for still in [
            Still::Boot,
            Still::Night,
            Still::UpdateReceiving,
            Still::UpdateTrial,
            Still::UpdateConfirmed,
            Still::UpdateRolledBack,
        ] {
            assert_eq!(still.section(), Section::Home, "{still:?}");
        }
        for still in [
            Still::About,
            Still::System,
            Still::Version,
            Still::Wifi,
            Still::Uptime,
            Still::NightHours,
            Still::Lock,
        ] {
            assert_eq!(still.section(), Section::System, "{still:?}");
        }
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
            // The states of the whole system cover the screen: the checklist, the changeover, the dark clock, the pad.
            let whole = match still {
                Still::Boot => Some(Whole::Starting),
                Still::Night => Some(Whole::Night),
                Still::Pin => Some(Whole::Pin),
                Still::UpdateReceiving
                | Still::UpdateTrial
                | Still::UpdateConfirmed
                | Still::UpdateRolledBack => Some(Whole::Update),
                _ => None,
            };
            assert_eq!(nav.whole(), whole, "{still:?}");
            // ALERT, and the screens over a device, have screens open above their root.
            let open: Vec<Screen> = match still {
                Still::Alert => vec![Screen::Alert(4)],
                Still::Yours => vec![Screen::Yours],
                Still::Name | Still::NameLocked | Still::Pin => {
                    vec![Screen::Name("ads.example.net".into())]
                }
                Still::NameThreat => vec![
                    Screen::Alert(4),
                    Screen::Name("secure-login.example.org".into()),
                ],
                Still::Device => vec![Screen::Device(mac_of(31))],
                Still::Confirm => vec![
                    Screen::Device(mac_of(23)),
                    Screen::Confirm {
                        mac: mac_of(23),
                        pass: false,
                    },
                ],
                Still::Pause => vec![Screen::Device(mac_of(31)), Screen::ItsGate(mac_of(31))],
                Still::About => vec![Screen::About],
                Still::Version => vec![Screen::Version],
                Still::Wifi => vec![Screen::Wifi],
                Still::Uptime => vec![Screen::Uptime],
                Still::NightHours => vec![Screen::NightHours],
                Still::Lock => vec![Screen::Lock],
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
        assert_eq!(R8_ALERTS[3].at, When::Day(Weekday::Mon));
        assert_eq!(
            R8_ALERTS[0].at,
            When::Time(TimeOfDay {
                hour: 10,
                minute: 19
            })
        );
        // The stamps are `A9`'s: what the panel did about each.
        assert_eq!(
            R8_ALERTS.map(|a| a.stamp),
            [
                AlertStamp::Refused,
                AlertStamp::Admitted,
                AlertStamp::Observed,
                AlertStamp::Observed
            ]
        );
        assert_eq!(
            R8_ALERTS.map(|a| a.name.is_some()),
            [true, false, false, false]
        );
        assert_eq!(TOP[0].name, "ads.example.net");
        assert_eq!(TOP.map(|t| t.count), [1204, 861, 540]);
        assert!(TOP.iter().all(|t| t.class == NameClass::Ads));
        assert_eq!(
            YOURS.map(|y| (y.name, y.verdict)),
            [
                ("shop-ads.example.net", OwnerVerdict::Allow),
                ("forum.example.com", OwnerVerdict::Refuse)
            ]
        );
        assert_eq!(THREAT.name, "secure-login.example.org");
        assert_eq!(THREAT.class, NameClass::Threat);
        assert_eq!(R8_ALERTS[0].name, Some(THREAT.name));
        assert_eq!((UPD.from, UPD.to, UPD.trial_secs), ("0.4.1", "0.4.2", 120));
        // The alerts' devices are in the house; no name has a device (spec §12.1).
        assert!(R8_ALERTS.iter().all(|a| a.device < HOUSE.len()));
    }

    #[test]
    fn the_names_stills_hold_the_day_the_owners_list_and_the_facts_of_the_name_they_show() {
        for still in [Still::Names, Still::Yours, Still::Name, Still::NameLocked] {
            let world = world(still);
            let tops: Vec<(&str, u32)> = world
                .top_names
                .iter()
                .map(|row| (row.name.as_str(), row.count))
                .collect();
            assert_eq!(
                tops,
                [
                    ("ads.example.net", 1204),
                    ("tracker.example.com", 861),
                    ("metrics.example.org", 540)
                ],
                "{still:?}"
            );
            assert!(world.top_names.iter().all(|row| !row.open));
            assert_eq!(world.owner_total, 2);
            let own: Vec<(&str, OwnerVerdict)> = world
                .owner_newest
                .iter()
                .map(|row| (row.name.as_str(), row.verdict))
                .collect();
            assert_eq!(
                own,
                [
                    ("shop-ads.example.net", OwnerVerdict::Allow),
                    ("forum.example.com", OwnerVerdict::Refuse)
                ]
            );
            assert_eq!(world.owner_newest[0].day, Some(DayWord::On(Weekday::Mon)));
        }
        let name = world(Still::Name);
        assert!(!name.lock_on);
        let facts = name.name_facts.expect("facts");
        assert_eq!(
            (facts.name.as_str(), facts.class, facts.owner, facts.count),
            ("ads.example.net", NameClass::Ads, None, Some(1204))
        );
        assert!(world(Still::NameLocked).lock_on);
        let threat = world(Still::NameThreat);
        assert_eq!(
            threat.alerts_waiting, 4,
            "opened from the slip of a waiting alert"
        );
        assert_eq!(
            threat.name_facts.map(|f| (f.name, f.class)),
            Some(("secure-login.example.org".to_string(), NameClass::Threat))
        );
        // The alert stills carry no names, and the others no alerts.
        assert!(
            world(Still::Alert).top_names.is_empty() && world(Still::Alert).name_facts.is_none()
        );
        assert!(world(Still::Home).owner_newest.is_empty());
    }

    #[test]
    fn the_alert_stills_hold_the_log_the_idle_log_holds_its_two_moments() {
        let log = world(Still::Alerts);
        assert_eq!(log.alerts.len(), 3, "the newest three of four");
        let ids: Vec<u64> = log.alerts.iter().map(|row| row.id).collect();
        assert_eq!(ids, [4, 3, 2]);
        let who: Vec<&str> = log.alerts.iter().map(|row| row.who.as_str()).collect();
        assert_eq!(who, ["Sam's phone", "\u{b7}216", "\u{b7}143"]);
        // The slip's lines are the mockup's, and the first alert names the threat and Sam's phone.
        let top = log.top_alert().expect("a top alert");
        assert_eq!(top.name.as_deref(), Some("secure-login.example.org"));
        assert_eq!(top.device, Some(mac_of(23)));
        assert!(log.cursor_on, "the stills are taken with the cursor on");
        let quiet = world(Still::AlertsNone);
        assert!(quiet.alerts.is_empty() && quiet.alerts_waiting == 0);
        assert_eq!(
            quiet.last_entry.map(Moment::word).as_deref(),
            Some("MON 21:40")
        );
        assert_eq!(
            quiet.watch_opened.map(Moment::word).as_deref(),
            Some("TUE 00:00")
        );
        assert!(world(Still::Home).last_entry.is_none());
    }

    #[test]
    fn the_system_stills_hold_the_mockups_readings() {
        for still in [
            Still::System,
            Still::Version,
            Still::Wifi,
            Still::Uptime,
            Still::NightHours,
            Still::Lock,
        ] {
            let world = world(still);
            // The link at -54 dBm on 5 GHz, held 12 days; the upstream answering in 9 ms (not the router's 2).
            assert_eq!(world.link.band, Some(Band::Ghz5), "{still:?}");
            assert_eq!(
                (
                    world.link.signal_dbm,
                    world.link.linked_days,
                    world.link.upstream_ms
                ),
                (Some(-54), Some(12), Some(9)),
                "{still:?}"
            );
            // Slot A runs 0.4.1, confirmed; up 12 D 4 H; load 0.21, 38 %, 47 degrees; dark 23 to 07; the lock off.
            let machine = &world.machine;
            assert_eq!(machine.slot, Some(Slot::A));
            assert_eq!(machine.last_update, LastUpdate::Confirmed);
            assert_eq!(world.version, "0.4.1");
            assert_eq!(
                (world.uptime_days, machine.uptime_hours),
                (Some(12), Some(4))
            );
            assert_eq!(
                (machine.load_hundredths, machine.memory_pct, machine.temp_c),
                (Some(21), Some(38), Some(47))
            );
            assert_eq!(world.night, Some((23, 7)));
            assert!(!world.lock_on && !world.update_busy);
            assert_eq!(world.rejoin, crate::os::world::Rejoin::Idle);
            assert_eq!(machine.last_restart, "the update to 0.4.1");
        }
        // The other stills know none of it.
        assert_eq!(world(Still::Home).link, Link::default());
        assert_eq!(world(Still::About).machine, Machine::default());
    }
}
