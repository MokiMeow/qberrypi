//! What the screens read: the panel's view of the house, as plain data the caller fills in (spec §5.2).
//!
//! The caller (qberrypi's panel loop) copies it out of the resolver, the device inventory, the alert
//! store and the update controller on the cadences of the spec, and hands it to the screens; they
//! never read a lock. Everything a name rides in (a device's name, a watched name, an alert's lines)
//! is already fitted to its place, and none of those types prints a name: `Debug` shows counts and
//! states only.
//!
//! An `Option` is a figure the Pi may not know yet: the screens draw it as `—` (or `--:--`), never a zero.

use std::fmt;

use crate::format::{fit_site, fit_text};
use crate::state::{Share, TimeOfDay};

/// A device's hardware address.
pub type Mac = [u8; 6];

/// The most devices the world holds: three pages of eleven places (the twelfth is `MORE`).
pub const MAX_DEVICES: usize = 33;

/// The most names of the watched device the world holds: the last two it asked.
pub const MAX_WATCHED: usize = 2;

/// The most lines an alert's slab holds under its title.
pub const MAX_ALERT_LINES: usize = 3;

/// How many cells a device's name has: the owner's labels are at most this, and the 30 px name on
/// its own screen still fits the width.
pub const DEVICE_NAME_CELLS: usize = 24;

/// How many cells the slab's title has: the 30 px title the slab sets past 18 letters.
pub const ALERT_TITLE_CELLS: usize = 24;

/// How many cells the alert slab's lines have: the subject (20 px bold, the first) and the others (18 px).
pub const ALERT_LINE_CELLS: [usize; MAX_ALERT_LINES] = [36, 40, 40];

/// How many cells a watched name has: 24 px on the device's screen, as `fit_site` fits it.
pub const WATCHED_NAME_CELLS: usize = 22;

/// How many cells the version text has.
pub const VERSION_CELLS: usize = 12;

/// How the ad blocking stands on one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    /// Ads are refused for it.
    Blocked,
    /// Ads pass: the device is let off the ads lists. Threats stay blocked.
    AdsPass,
    /// Its own gate is open for a while: ads pass until it shuts.
    Open {
        /// Seconds until the gate shuts.
        left_secs: u16,
    },
    /// The router and the Pi: they are never changed.
    Always,
}

/// One device on the wall.
///
/// `Debug` says its state and nothing else: not its name, its address or its place.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceRow {
    /// Its hardware address, which is how a screen knows which device it is about.
    pub mac: Mac,
    /// What the screens call it: the owner's label, else its hostname, else `·` and the last octet of
    /// its address, fitted to `DEVICE_NAME_CELLS`. See `display_name`.
    pub name: String,
    /// Whether `name` is a label or a hostname rather than the `·octet` made for a device with neither.
    pub named: bool,
    /// The last octet of its address.
    pub octet: u8,
    /// Where ad blocking stands on it.
    pub state: DeviceState,
    /// Whether the configuration, not the owner, decides its state: its key says `SET IN CONFIG` and is inert.
    pub fixed: bool,
    /// Whether it asked something in the last second or so: the lamp.
    pub asking: bool,
}

impl fmt::Debug for DeviceRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceRow")
            .field("state", &self.state)
            .field("fixed", &self.fixed)
            .field("asking", &self.asking)
            .finish_non_exhaustive()
    }
}

/// A device's name as the wall shows it, and whether it is a real name: the label the owner gave, else
/// the hostname, else `·` and the last octet. A label or hostname of nothing but spaces is none.
pub fn display_name(label: Option<&str>, hostname: Option<&str>, octet: u8) -> (String, bool) {
    [label, hostname]
        .into_iter()
        .flatten()
        .map(|text| fit_text(text, DEVICE_NAME_CELLS))
        .find(|text| !text.is_empty())
        .map_or_else(|| (format!("\u{b7}{octet}"), false), |text| (text, true))
}

/// What a watched device's name came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Blocked: no answer.
    Refused,
    /// An ad name that was answered, because the device's ads pass.
    Passed,
    /// Answered as any name is.
    Answered,
}

impl Outcome {
    /// The word on the device's screen, in capitals.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Refused => "REFUSED",
            Self::Passed => "PASSED",
            Self::Answered => "ANSWERED",
        }
    }
}

/// A name the watched device asked, with what came of it.
///
/// `Debug` shows the outcome only.
#[derive(Clone, PartialEq, Eq)]
pub struct WatchedName {
    /// The name, as `fit_site` fits it to `WATCHED_NAME_CELLS`.
    pub name: String,
    /// What came of it.
    pub outcome: Outcome,
}

impl WatchedName {
    /// A name and its outcome; the name is fitted.
    pub fn new(name: &str, outcome: Outcome) -> Self {
        Self {
            name: fit_site(name, WATCHED_NAME_CELLS),
            outcome,
        }
    }
}

impl fmt::Debug for WatchedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WatchedName")
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// The alert the panel can open: the newest waiting one, as the store's `top()` gives it.
///
/// `Debug` shows the id and the severity only.
#[derive(Clone, PartialEq, Eq)]
pub struct AlertRow {
    /// The store's id: what `Intent::Ack` carries, and how the screen knows its alert is gone.
    pub id: u64,
    /// Whether the slab is white (high severity) rather than blue.
    pub high: bool,
    /// The title, in capitals, fitted to `ALERT_TITLE_CELLS`.
    pub title: String,
    /// Up to three lines under it: the first is the subject (20 px bold), the others 18 px; each fitted
    /// to its place in `ALERT_LINE_CELLS`.
    pub lines: Vec<String>,
}

impl AlertRow {
    /// An alert row; the title and the lines are fitted, and lines past the third are dropped.
    pub fn new(id: u64, high: bool, title: &str, lines: &[&str]) -> Self {
        Self {
            id,
            high,
            title: fit_text(title, ALERT_TITLE_CELLS),
            lines: lines
                .iter()
                .zip(ALERT_LINE_CELLS)
                .map(|(line, cells)| fit_text(line, cells))
                .collect(),
        }
    }
}

impl fmt::Debug for AlertRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlertRow")
            .field("id", &self.id)
            .field("high", &self.high)
            .finish_non_exhaustive()
    }
}

/// What a device that is new to the house gets in the mode in force: the word on the wall's first empty place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Newcomer {
    /// Ads pass until the owner blocks them.
    #[default]
    AdsPass,
    /// Ads are blocked until the owner lets them pass.
    Blocked,
}

impl Newcomer {
    /// The word, in capitals.
    pub const fn word(self) -> &'static str {
        match self {
            Self::AdsPass => "ADS PASS",
            Self::Blocked => "BLOCKED",
        }
    }
}

/// The main gate while it is open: blocking paused for the house.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pause {
    /// Seconds until blocking turns on again.
    pub left_secs: u16,
    /// How long the pause was for, as chosen: 300, 1,800 or 3,600. The gate refills by `left / total`.
    pub total_secs: u16,
}

/// Everything the OS screens read.
///
/// `Debug` shows counts, never a name.
#[derive(Clone, PartialEq)]
pub struct World {
    /// The time of day; `None` while the clock is not set.
    pub time: Option<TimeOfDay>,
    /// Whether Wi-Fi is joined.
    pub wifi_joined: bool,
    /// Queries answered since midnight; `None` while unknown.
    pub queries_today: Option<u32>,
    /// Queries refused since midnight; `None` while unknown.
    pub refused_today: Option<u32>,
    /// The share of today's queries that were refused; `None` while unknown or with no queries.
    pub share: Option<Share>,
    /// How many names the lists hold; `None` while unknown.
    pub list_names: Option<u32>,
    /// The running version, as `0.4.1`.
    pub version: String,
    /// Whole days since the board started; `None` while unknown.
    pub uptime_days: Option<u32>,
    /// The main gate: `None` while blocking is on (the gate is shut).
    pub blocking: Option<Pause>,
    /// The devices on the wall, in the wall's order, at most `MAX_DEVICES`.
    pub devices: Vec<DeviceRow>,
    /// What a newcomer gets, in the mode in force.
    pub newcomer: Newcomer,
    /// The LIVE tile's lamp: white for 3 frames after a name went by in the lane.
    pub beat: bool,
    /// The last names the watched device asked, newest first, at most `MAX_WATCHED`; empty unless DEVICE is open.
    pub watched: Vec<WatchedName>,
    /// How many alerts wait.
    pub alerts_waiting: u32,
    /// When the top waiting alert last happened; `None` with none waiting, or when its time is not today's.
    pub alert_newest: Option<TimeOfDay>,
    /// The newest waiting alert, for the ALERT screen.
    pub top_alert: Option<AlertRow>,
    /// Whether the lock is on. It is not in P1, and stays `false`.
    pub lock_on: bool,
}

impl Default for World {
    /// A world that knows nothing: no time, no figures, blocking on, no devices, no alerts.
    fn default() -> Self {
        Self {
            time: None,
            wifi_joined: false,
            queries_today: None,
            refused_today: None,
            share: None,
            list_names: None,
            version: String::new(),
            uptime_days: None,
            blocking: None,
            devices: Vec::new(),
            newcomer: Newcomer::default(),
            beat: false,
            watched: Vec::new(),
            alerts_waiting: 0,
            alert_newest: None,
            top_alert: None,
            lock_on: false,
        }
    }
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World")
            .field("devices", &self.devices.len())
            .field("alerts_waiting", &self.alerts_waiting)
            .field("gate_open", &self.blocking.is_some())
            .field("lock_on", &self.lock_on)
            .finish_non_exhaustive()
    }
}

impl World {
    /// Adds a device to the wall's end; `false`, and not added, when the wall already has `MAX_DEVICES`.
    pub fn push_device(&mut self, row: DeviceRow) -> bool {
        if self.devices.len() >= MAX_DEVICES {
            return false;
        }
        self.devices.push(row);
        true
    }

    /// The device with this address, if it is on the wall: a screen about a device that is gone finds none and pops.
    pub fn device(&self, mac: &Mac) -> Option<&DeviceRow> {
        self.devices.iter().find(|row| row.mac == *mac)
    }

    /// How many devices have ads blocked by their state: the figure under `DEVICES` on HOME. The router and
    /// the Pi are `Always` and not counted.
    pub fn blocked_count(&self) -> usize {
        self.devices
            .iter()
            .filter(|row| row.state == DeviceState::Blocked)
            .count()
    }

    /// Whether any device is asking now: DEVICES' lamp.
    pub fn any_asking(&self) -> bool {
        self.devices.iter().any(|row| row.asking)
    }

    /// Whether the main gate is open.
    pub fn gate_open(&self) -> bool {
        self.blocking.is_some()
    }

    /// The bytes of a world that holds the most the types above allow: the struct, every row and every
    /// string full (a name's cells at four bytes each, the widest a character is in UTF-8). The spec's
    /// budget for a world (§6.7) is 64 KB.
    pub const fn bound_bytes() -> usize {
        use std::mem::size_of;
        const CHAR: usize = 4;
        let alert_lines = ALERT_LINE_CELLS[0] + ALERT_LINE_CELLS[1] + ALERT_LINE_CELLS[2];
        size_of::<Self>()
            + VERSION_CELLS * CHAR
            + MAX_DEVICES * (size_of::<DeviceRow>() + DEVICE_NAME_CELLS * CHAR)
            + MAX_WATCHED * (size_of::<WatchedName>() + WATCHED_NAME_CELLS * CHAR)
            + (ALERT_TITLE_CELLS + alert_lines) * CHAR
            + MAX_ALERT_LINES * size_of::<String>()
    }

    /// The bytes of memory this world holds now: the struct and what its vectors and strings have room for.
    pub fn bytes(&self) -> usize {
        use std::mem::size_of;
        size_of::<Self>()
            + self.version.capacity()
            + self.devices.capacity() * size_of::<DeviceRow>()
            + self
                .devices
                .iter()
                .map(|d| d.name.capacity())
                .sum::<usize>()
            + self.watched.capacity() * size_of::<WatchedName>()
            + self
                .watched
                .iter()
                .map(|w| w.name.capacity())
                .sum::<usize>()
            + self.top_alert.as_ref().map_or(0, |alert| {
                alert.title.capacity()
                    + alert.lines.capacity() * size_of::<String>()
                    + alert.lines.iter().map(String::capacity).sum::<usize>()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(octet: u8, state: DeviceState) -> DeviceRow {
        let (name, named) = display_name(None, None, octet);
        DeviceRow {
            mac: [2, 0, 0, 0, 0, octet],
            name,
            named,
            octet,
            state,
            fixed: false,
            asking: false,
        }
    }

    #[test]
    fn a_world_holding_the_most_is_within_its_bound_and_the_bound_within_64_kb() {
        // The widest a name's cells are in bytes is four each; use that for every name.
        let cells = |n: usize| "\u{1F600}".repeat(n);
        let mut world = World {
            version: cells(VERSION_CELLS),
            devices: Vec::with_capacity(MAX_DEVICES),
            watched: Vec::with_capacity(MAX_WATCHED),
            ..World::default()
        };
        for octet in 0..MAX_DEVICES as u8 {
            let mut full = row(octet, DeviceState::Blocked);
            full.name = cells(DEVICE_NAME_CELLS);
            assert!(world.push_device(full));
        }
        for _ in 0..MAX_WATCHED {
            world.watched.push(WatchedName {
                name: cells(WATCHED_NAME_CELLS),
                outcome: Outcome::Refused,
            });
        }
        world.top_alert = Some(AlertRow {
            id: 1,
            high: true,
            title: cells(ALERT_TITLE_CELLS),
            lines: ALERT_LINE_CELLS.iter().map(|&n| cells(n)).collect(),
        });

        let bound = World::bound_bytes();
        assert!(world.bytes() <= bound, "{} B over {bound} B", world.bytes());
        assert!(bound <= 64 * 1024, "{bound} B");
        // The empty world is the struct and nothing else.
        assert_eq!(World::default().bytes(), std::mem::size_of::<World>());
    }

    #[test]
    fn a_device_is_called_by_its_label_then_its_hostname_then_its_octet() {
        assert_eq!(
            display_name(Some("Desk PC"), Some("pc"), 12),
            ("Desk PC".into(), true)
        );
        assert_eq!(
            display_name(None, Some("printer"), 57),
            ("printer".into(), true)
        );
        assert_eq!(display_name(None, None, 143), ("\u{b7}143".into(), false));
        // Nothing but spaces is no name at all.
        assert_eq!(
            display_name(Some("  "), Some(""), 7),
            ("\u{b7}7".into(), false)
        );
        assert_eq!(
            display_name(Some(""), Some("doorbell"), 64),
            ("doorbell".into(), true)
        );
        // A name is fitted to its place, with a tilde.
        let long = display_name(Some("A very long label for a very small tile"), None, 1).0;
        assert_eq!(long.chars().count(), DEVICE_NAME_CELLS);
        assert!(long.ends_with('~'));
    }

    #[test]
    fn a_watched_name_is_fitted_like_a_site_and_an_alert_to_its_lines() {
        let name = WatchedName::new(
            "a.very.long.host.name.of.a.tracker.example.org",
            Outcome::Refused,
        );
        assert_eq!(
            name.name, "a.tracker.example.org",
            "host labels go first, whole"
        );
        assert_eq!(
            WatchedName::new("i.ytimg.com", Outcome::Answered).name,
            "i.ytimg.com"
        );
        assert_eq!(Outcome::Passed.word(), "PASSED");
        let alert = AlertRow::new(
            7,
            true,
            "PHISHING BLOCKED",
            &[
                "secure-login-verify.top",
                "second",
                "third",
                "a fourth line is dropped",
            ],
        );
        assert_eq!(alert.lines.len(), MAX_ALERT_LINES);
        assert_eq!(alert.lines[0], "secure-login-verify.top");
        let tall = AlertRow::new(1, false, &"T".repeat(80), &[&"x".repeat(80)]);
        assert_eq!(tall.title.chars().count(), ALERT_TITLE_CELLS);
        assert_eq!(tall.lines[0].chars().count(), ALERT_LINE_CELLS[0]);
        assert!(AlertRow::new(1, false, "T", &[]).lines.is_empty());
    }

    #[test]
    fn debug_never_prints_a_name_an_address_or_a_line() {
        let mut world = World::default();
        let mut sam = row(23, DeviceState::AdsPass);
        sam.name = "Sam's phone".into();
        world.push_device(sam.clone());
        world.watched.push(WatchedName::new(
            "secret-tracker.example.org",
            Outcome::Refused,
        ));
        world.top_alert = Some(AlertRow::new(
            9,
            true,
            "PHISHING BLOCKED",
            &["secure-login-verify.top"],
        ));
        let said = format!(
            "{world:?} {sam:?} {:?} {:?}",
            world.watched, world.top_alert
        );
        for secret in [
            "Sam",
            "phone",
            "secret-tracker",
            "example.org",
            "PHISHING",
            "secure-login",
            "02:00",
            "[2, 0",
        ] {
            assert!(!said.contains(secret), "{said} holds {secret}");
        }
        assert!(said.contains("Refused") && said.contains("AdsPass"));
    }

    #[test]
    fn the_wall_holds_33_devices_and_finds_one_by_its_address() {
        let mut world = World::default();
        for octet in 0..MAX_DEVICES as u8 {
            assert!(world.push_device(row(octet, DeviceState::AdsPass)));
        }
        assert!(
            !world.push_device(row(200, DeviceState::Blocked)),
            "the 34th is refused"
        );
        assert_eq!(world.devices.len(), MAX_DEVICES);
        assert_eq!(world.device(&[2, 0, 0, 0, 0, 5]).map(|d| d.octet), Some(5));
        assert_eq!(world.device(&[2, 0, 0, 0, 0, 200]), None);
    }

    #[test]
    fn the_counts_the_home_tiles_read() {
        let mut world = World::default();
        assert!(!world.gate_open() && !world.any_asking());
        assert_eq!(world.blocked_count(), 0);
        world.push_device(row(1, DeviceState::Blocked));
        world.push_device(row(2, DeviceState::AdsPass));
        world.push_device(row(3, DeviceState::Always));
        world.push_device(row(4, DeviceState::Open { left_secs: 30 }));
        world.push_device(row(5, DeviceState::Blocked));
        assert_eq!(
            world.blocked_count(),
            2,
            "only the ones whose ads are blocked"
        );
        world.devices[3].asking = true;
        assert!(world.any_asking());
        world.blocking = Some(Pause {
            left_secs: 277,
            total_secs: 300,
        });
        assert!(world.gate_open());
    }

    #[test]
    fn the_newcomer_words_are_ads_pass_and_blocked() {
        assert_eq!(Newcomer::default(), Newcomer::AdsPass);
        assert_eq!(Newcomer::AdsPass.word(), "ADS PASS");
        assert_eq!(Newcomer::Blocked.word(), "BLOCKED");
    }

    #[test]
    fn a_world_is_small() {
        // The spec's memory budget: a world is at most 64 KB. Thirty-three rows of the longest names, an
        // alert and two watched names are well inside it.
        let mut world = World::default();
        for octet in 0..MAX_DEVICES as u8 {
            let mut full = row(octet, DeviceState::Blocked);
            full.name = "n".repeat(DEVICE_NAME_CELLS);
            world.push_device(full);
        }
        let bytes = world.devices.len() * (size_of::<DeviceRow>() + DEVICE_NAME_CELLS)
            + size_of::<World>()
            + 2 * (size_of::<WatchedName>() + WATCHED_NAME_CELLS)
            + size_of::<AlertRow>()
            + ALERT_TITLE_CELLS
            + ALERT_LINE_CELLS.iter().sum::<usize>();
        assert!(bytes < 64 * 1024, "{bytes} bytes");
    }
}
