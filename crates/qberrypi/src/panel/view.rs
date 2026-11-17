//! Turning what the system knows into what the panel shows.
//!
//! Everything here is a pure function of what it is given (a snapshot, a copy of the inventory, an alert, the
//! clock): the loop in `panel/mod.rs` reads the system on the cadences of spec §5.2 and hands the
//! copies to these. What the OS screens read is the `World` of `qb_panel::os::world`: the wall of devices
//! (spec §3.4, ruling R10), the gate's total (R7) and the alert's lines (spec §5.3, R11) are made here.
//!
//! The types that hold a name, a label or an address (`WallEntry`) have no `Debug`.

use std::net::{IpAddr, Ipv4Addr};

use qb_panel::drain::Stills;
use qb_panel::format::{self, fit_site, fit_text, gate_fill};
use qb_panel::os::screens::parts::GATE_CHOICES;
use qb_panel::os::world::{
    ALERT_LINE_CELLS, AlertRow, DeviceRow, DeviceState, MAX_DEVICES, Newcomer, display_name,
};
use qb_panel::{Blocking, LaneWords, Share, TimeOfDay};

use crate::live::Snapshot;
use crate::panel::alerts::{PanelAlert, Severity};
use crate::watch::devices::{WallCopy, WallDevice};
use crate::watch::frames::Mac as HardwareAddress;

/// The time of day at `unix_secs` in a zone `utc_offset_secs` seconds from UTC.
pub fn time_of_day(unix_secs: u64, utc_offset_secs: i32) -> TimeOfDay {
    // Wide enough that an offset before the epoch cannot underflow.
    let local = i128::from(unix_secs) + i128::from(utc_offset_secs);
    let secs_of_day = local.rem_euclid(86_400) as u32;
    TimeOfDay {
        hour: (secs_of_day / 3_600) as u8,
        minute: (secs_of_day % 3_600 / 60) as u8,
    }
}

/// Holds a counter to what the panel can show.
pub fn held(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// What the key shows when `paused_left` seconds of a pause are left, or blocking is on.
pub fn blocking(paused_left: Option<u16>) -> Blocking {
    paused_left.map_or(Blocking::On, |left_secs| Blocking::Paused { left_secs })
}

/// The total of the main gate's pause, for the figures that count against it (ruling R7).
///
/// The control reports only the time left. The panel remembers the total of the gate it opened itself
/// (`remembered`); for a pause it did not open (the API's) or whose `left` is more than what it
/// remembers, the total is the smallest of the gate's lengths, 5, 30 and 60 minutes, that is at least
/// `left`, and `left` itself past the longest. Display only: nothing on the answer path reads it.
pub fn gate_total(remembered: Option<u16>, left: u16) -> u16 {
    match remembered {
        Some(total) if left <= total => total,
        _ => GATE_CHOICES
            .iter()
            .map(|&(_, secs)| secs)
            .find(|&secs| secs >= left)
            .unwrap_or(left),
    }
}

/// How the gate's refill shows when `paused_left` seconds of a pause of `total` seconds are left: steps of
/// 12 pixels, none while blocking is on.
pub fn refill(paused_left: Option<u16>, total: u16) -> u8 {
    paused_left.map_or(0, |left| gate_fill(left, total))
}

/// Whether the board is associated with its access point, as far as anything knows.
pub fn joined(snapshot: &Snapshot) -> bool {
    snapshot.wifi.as_ref().is_some_and(|wifi| wifi.associated)
}

/// How long without a name on the lane makes the lane say it is quiet, in seconds.
pub const QUIET_AFTER_SECS: u64 = 120;

/// The minutes of quiet that the lane's words show for `idle_secs` seconds without a lane name, rounded
/// down to the unit the words are written in: whole minutes below two hours, whole hours below 48
/// hours, whole days beyond. The words (`QUIET 47 MIN`, `QUIET 3 H`, `QUIET 2 D`) then change only when
/// their text does, so a still is spent on them no more often than that.
fn quiet_minutes(idle_secs: u64) -> u32 {
    const HOUR: u32 = 60;
    const DAY: u32 = 24 * HOUR;
    // Past what a `u32` of minutes holds, the figure is held there: the text is the same.
    let minutes = u32::try_from(idle_secs / 60).unwrap_or(u32::MAX);
    match minutes {
        0..120 => minutes,
        120..2880 => minutes / HOUR * HOUR,
        _ => minutes / DAY * DAY,
    }
}

/// The words the lane says while it has nothing to carry, for a snapshot that has been `idle_secs`
/// seconds without a lane name.
///
/// `JOINING WI-FI` until the Wi-Fi is joined, again after a drop; then `LISTENING` until the resolver
/// has answered a query today; then, once queries have come, `QUIET n MIN` after two minutes without a
/// name (minutes, then hours, then days, as `LaneWords` says, and rounded down to that unit). Otherwise
/// none: names are going by.
fn words(snapshot: &Snapshot, idle_secs: u64) -> Option<LaneWords> {
    if !joined(snapshot) {
        return Some(LaneWords::Joining);
    }
    match snapshot.today {
        None => Some(LaneWords::Listening),
        Some(today) if today.queries == 0 => Some(LaneWords::Listening),
        Some(_) if idle_secs >= QUIET_AFTER_SECS => Some(LaneWords::Quiet {
            minutes: quiet_minutes(idle_secs),
        }),
        Some(_) => None,
    }
}

/// The slow figures of the panel for a snapshot: what the screen is to come to show, one region at a time.
///
/// `paused_left` is the seconds of a pause that are left and `paused_total` how long it was for
/// (`gate_total`), `now_unix` is the clock in Unix seconds, and `idle_secs` is how long ago the panel
/// last took a name from the lane's feed.
///
/// Whatever is not known is not shown: no percentage while the resolver has not answered anything,
/// and no time of day until the clock is set. While the Wi-Fi is not joined the percentage and the
/// count are dashes whatever the counters hold, and the lane says so. The count is the snapshot's
/// figure of blocked queries; the panel's loop replaces it with the Drain's own count, which holds
/// back the names still on their way. Nothing here keeps a name or a subject past the `Stills` it
/// returns.
pub fn wanted_stills(
    snapshot: &Snapshot,
    paused_left: Option<u16>,
    paused_total: u16,
    now_unix: u64,
    idle_secs: u64,
) -> Stills {
    let today = snapshot.today.filter(|_| joined(snapshot));
    Stills {
        percent: today.and_then(|today| Share::of(today.blocked, today.queries)),
        clock: snapshot
            .clock_set
            .then(|| time_of_day(now_unix, snapshot.utc_offset_secs)),
        blocking: blocking(paused_left),
        gate_fill: refill(paused_left, paused_total),
        count: today.map(|today| held(today.blocked)),
        gate_ask: false,
        words: words(snapshot, idle_secs),
    }
}

/// A Unix time before this (1 January 2020) is from a clock that was not set: no time of day is shown for it.
pub const CLOCK_SANE_UNIX: u64 = 1_577_836_800;

/// The local day of `unix_secs` in a zone `utc_offset_secs` from UTC, as a count of days.
fn local_day(unix_secs: u64, utc_offset_secs: i32) -> i128 {
    (i128::from(unix_secs) + i128::from(utc_offset_secs)).div_euclid(86_400)
}

/// The time of day of `unix_secs`, or `None` for a time from before the clock was set.
pub fn clock_of(unix_secs: u64, utc_offset_secs: i32) -> Option<TimeOfDay> {
    (unix_secs >= CLOCK_SANE_UNIX).then(|| time_of_day(unix_secs, utc_offset_secs))
}

/// The time of day of `unix_secs` when it is today (as `now_unix` has it); `None` for any other day, and
/// for a time from before the clock was set: HOME says `BEFORE TODAY` then.
pub fn today_clock(unix_secs: u64, now_unix: u64, utc_offset_secs: i32) -> Option<TimeOfDay> {
    (local_day(unix_secs, utc_offset_secs) == local_day(now_unix, utc_offset_secs))
        .then(|| clock_of(unix_secs, utc_offset_secs))
        .flatten()
}

/// How long ago a device asked for the wall to say it asks now, in seconds: the last second or two, at
/// the resolver's whole-second clock.
pub const ASKING_SECS: u64 = 2;

/// How long a device that the owner gave no label stays on the wall after it was last seen, in seconds.
pub const WALL_WINDOW_SECS: u64 = 86_400;

/// What the wall's Pi place is called: the Pi is no device of the inventory, so it has no hostname there.
const PI_NAME: &str = "qberrypi";

/// One place of the wall, with what the panel looks a device up by: its addresses, for the lamp, the watch
/// of its names and the alert that names its address; and when its own gate shuts on the panel's clock,
/// so that the countdown steps every second between two reads of the inventory.
///
/// It has no `Debug`: it holds a name and addresses.
#[derive(Clone)]
pub struct WallEntry {
    /// What the screens read.
    pub row: DeviceRow,
    /// The IPv4 addresses it was seen with, the newest first.
    pub ipv4: Vec<Ipv4Addr>,
    /// The panel's millisecond at which its own gate shuts; `None` with the gate shut.
    pub gate_ends_ms: Option<u64>,
}

impl WallEntry {
    /// Steps the open gate's seconds to `now_ms`: at least 1 until the next read of the inventory says it is shut.
    pub fn tick(&mut self, now_ms: u64) {
        if let (Some(ends), DeviceState::Open { .. }) = (self.gate_ends_ms, self.row.state) {
            let left = ends.saturating_sub(now_ms).div_ceil(1_000).max(1);
            self.row.state = DeviceState::Open {
                left_secs: u16::try_from(left).unwrap_or(u16::MAX),
            };
        }
    }
}

/// How ads stand on a device: the router and the Pi are `Always`; a gate that is open makes it `Open`;
/// otherwise it is exempt (`AdsPass`) or not (`Blocked`).
fn state_of(device: &WallDevice) -> DeviceState {
    if device.always {
        DeviceState::Always
    } else if let Some(left_secs) = device.gate_left_secs {
        DeviceState::Open { left_secs }
    } else if device.exempt {
        DeviceState::AdsPass
    } else {
        DeviceState::Blocked
    }
}

/// The last octet of the newest of `addresses`; 0 for a device with none yet.
fn octet_of(addresses: &[Ipv4Addr]) -> u8 {
    addresses.first().map_or(0, |ip| ip.octets()[3])
}

/// Whether any of `addresses` is one of those that asked.
pub fn is_asking(addresses: &[Ipv4Addr], asking: &[Ipv4Addr]) -> bool {
    addresses.iter().any(|ip| asking.contains(ip))
}

/// The wall's place for one device of the inventory.
fn entry_of(device: &WallDevice, asking: &[Ipv4Addr], now_ms: u64) -> WallEntry {
    let octet = octet_of(device.ipv4());
    let (name, named) = display_name(device.label(), device.hostname(), octet);
    let state = state_of(device);
    let gate_ends_ms = match state {
        DeviceState::Open { left_secs } => {
            Some(now_ms.saturating_add(u64::from(left_secs) * 1_000))
        }
        _ => None,
    };
    WallEntry {
        row: DeviceRow {
            mac: device.mac.0,
            name,
            named,
            octet,
            state,
            fixed: device.fixed,
            asking: is_asking(device.ipv4(), asking),
        },
        ipv4: device.ipv4().to_vec(),
        gate_ends_ms,
    }
}

/// Whether the wall still shows a device that has no label and was last seen at `last_unix`, with the clock at
/// `now_unix` (`None` while it is not set: then no device is too old for the wall). A device last seen before
/// the clock was set is not too old either: its time says nothing.
pub fn recent(last_unix: u64, now_unix: Option<u64>) -> bool {
    now_unix.is_none_or(|now| {
        last_unix < CLOCK_SANE_UNIX || now.saturating_sub(last_unix) <= WALL_WINDOW_SECS
    })
}

/// The wall from the panel's copy of the inventory (spec §3.4, ruling R10): at most `MAX_DEVICES` places, the
/// devices with an owner label first in the label order qnet gave, then the others seen in the last 24 hours by
/// first sighting, then the router and the Pi (the devices that are `always`, and the Pi's own place). The
/// order is stable, so a tile never moves while the screen shows.
///
/// The copy (`Devices::wall_copy`) holds the devices the wall can show, in no order, and this puts them in the
/// wall's: sorting, names and fitted texts are made here, with the inventory's lock let go. A copy that holds
/// more (a test's) is cut to the same wall.
///
/// `asking` is the addresses that asked in the last second or two, and `now_ms` the panel's clock, for the
/// open gates' countdown. `now_unix` is the clock in Unix seconds, `None` while it is not set (`recent`). When
/// more than `MAX_DEVICES` would be on the wall the router and the Pi stay and the last of the others go.
pub fn wall(
    inventory: &WallCopy,
    asking: &[Ipv4Addr],
    now_ms: u64,
    now_unix: Option<u64>,
) -> Vec<WallEntry> {
    let mut labelled: Vec<&WallDevice> = Vec::new();
    let mut others: Vec<&WallDevice> = Vec::new();
    let mut always: Vec<&WallDevice> = Vec::new();
    for device in &inventory.rows {
        if device.always {
            always.push(device);
        } else if device.label().is_some() {
            labelled.push(device);
        } else if recent(device.last_unix, now_unix) {
            others.push(device);
        }
    }
    labelled.sort_by_key(|device| (device.label_rank, device.mac));
    others.sort_by_key(|device| (device.first_unix, device.mac));
    always.sort_by_key(|device| (device.first_unix, device.mac));

    let mut tail: Vec<WallEntry> = always
        .into_iter()
        .map(|device| entry_of(device, asking, now_ms))
        .collect();
    // The Pi is no device of the inventory: its place is made from what the inventory knows of it.
    if let Some(mac) = inventory.own_mac
        && !inventory.rows.iter().any(|device| device.mac == mac)
    {
        let addresses: Vec<Ipv4Addr> = inventory.own_ip.into_iter().collect();
        let octet = octet_of(&addresses);
        let (name, named) = display_name(None, Some(PI_NAME), octet);
        tail.push(WallEntry {
            row: DeviceRow {
                mac: mac.0,
                name,
                named,
                octet,
                state: DeviceState::Always,
                fixed: false,
                asking: is_asking(&addresses, asking),
            },
            ipv4: addresses,
            gate_ends_ms: None,
        });
    }
    tail.truncate(MAX_DEVICES);
    let room = MAX_DEVICES - tail.len();
    let mut wall: Vec<WallEntry> = labelled
        .into_iter()
        .chain(others)
        .take(room)
        .map(|device| entry_of(device, asking, now_ms))
        .collect();
    wall.extend(tail);
    wall
}

/// Keeps a countdown from stepping back up when the inventory is read again.
///
/// The inventory counts a gate in whole seconds on the resolver's clock, and the panel counts from the
/// moment it read; the two are up to a second apart, differently at each read, so a new read may say one
/// second more than the panel was showing. A gate whose end moved by less than a second keeps the end it had.
/// A gate that was opened again, or for longer, is a new end: it moved by more.
pub fn keep_countdowns(old: &[WallEntry], new: &mut [WallEntry], now_ms: u64) {
    for entry in new {
        let Some(end) = entry.gate_ends_ms else {
            continue;
        };
        let before = old
            .iter()
            .find(|was| was.row.mac == entry.row.mac)
            .and_then(|was| was.gate_ends_ms);
        if let Some(was) = before
            && end > was
            && end - was < 1_000
        {
            entry.gate_ends_ms = Some(was);
            entry.tick(now_ms);
        }
    }
}

/// What a newcomer gets in the mode in force: in the exempt-by-default mode ads pass until the owner blocks
/// them; in the list mode they are blocked until the owner lets them pass.
pub fn newcomer_of(inventory: &WallCopy) -> Newcomer {
    if inventory.exempt_default {
        Newcomer::AdsPass
    } else {
        Newcomer::Blocked
    }
}

/// The title of an alert's slab, in capitals, for its kind; a kind that is not known is spelt out of
/// itself.
fn alert_title(kind: &str) -> String {
    match kind {
        "threat" => "THREAT BLOCKED".to_owned(),
        "rebind" => "REBINDING BLOCKED".to_owned(),
        "dga" => "SUSPICIOUS LOOKUPS".to_owned(),
        "tunnel" => "TUNNEL SUSPECTED".to_owned(),
        "beacon" => "REGULAR CALLS".to_owned(),
        "burst" => "LOOKUP BURST".to_owned(),
        "new_device" => "NEW DEVICE".to_owned(),
        "arp_spoof" => "ARP SPOOF".to_owned(),
        other => other.replace('_', " ").to_uppercase(),
    }
}

/// `n times`, or `once`.
fn times(count: u64) -> String {
    match count {
        0 | 1 => "once".to_owned(),
        n => format!(
            "{} times",
            format::count(u32::try_from(n).unwrap_or(u32::MAX))
        ),
    }
}

/// `n times since hh:mm`, `once at hh:mm`, and the same without the time when it is not known.
fn since_line(count: u64, since: Option<TimeOfDay>) -> String {
    match (count, since) {
        (0 | 1, Some(at)) => format!("once at {}", format::clock(Some(at))),
        (n, Some(at)) => format!("{} since {}", times(n), format::clock(Some(at))),
        (n, None) => times(n),
    }
}

/// What the panel calls the device an alert is about: for a `new_device` and an `arp_spoof` the alert's
/// subject is the device's hardware address; for any other kind the alert's client is its address. The
/// label, the hostname or `·octet` as the wall has it; `·octet` of the client's address when the wall does
/// not hold the device; `a device` when it is not known at all.
fn device_of(alert: &PanelAlert, wall: &[WallEntry]) -> String {
    if matches!(alert.kind.as_str(), "new_device" | "arp_spoof") {
        let found = alert
            .subject
            .parse::<HardwareAddress>()
            .ok()
            .and_then(|mac| wall.iter().find(|entry| entry.row.mac == mac.0));
        return found.map_or_else(|| "a device".to_owned(), |entry| entry.row.name.clone());
    }
    match alert.client {
        Some(IpAddr::V4(ip)) => wall
            .iter()
            .find(|entry| entry.ipv4.contains(&ip))
            .map_or_else(
                || format!("\u{b7}{}", ip.octets()[3]),
                |entry| entry.row.name.clone(),
            ),
        _ => "a device".to_owned(),
    }
}

/// The alert the panel opens, as the slab shows it (spec §5.3, ruling R11): the title for its kind, up to
/// three lines, white for a high alert. The lines are made from what the store carries: the subject, how
/// many times and since when, and the device as the wall names it (`wall` is the panel's last copy of it).
///
/// `threat`: the name; `<device> asked n times by hh:mm`; `on the threat lists · never answered`.
/// `new_device`: `<device> joined at hh:mm`; `no name yet · <default word> by default`. `arp_spoof`:
/// `<device> answers as router`; `n times since hh:mm`. Others: the subject and `n times since hh:mm`.
/// A time is left out while the clock was not set when it was taken. Nothing here is kept, or logged.
pub fn alert_row(
    alert: &PanelAlert,
    wall: &[WallEntry],
    newcomer: Newcomer,
    utc_offset_secs: i32,
) -> AlertRow {
    let at = |unix| clock_of(unix, utc_offset_secs).map(|time| format::clock(Some(time)));
    let (first, last) = (at(alert.first_unix), at(alert.last_unix));
    let who = device_of(alert, wall);
    let site = fit_site(&alert.subject, ALERT_LINE_CELLS[0]);
    let text = fit_text(&alert.subject, ALERT_LINE_CELLS[0]);
    let lines: Vec<String> = match alert.kind.as_str() {
        "threat" => vec![
            site,
            match last {
                Some(last) => format!("{who} asked {} by {last}", times(alert.count)),
                None => format!("{who} asked {}", times(alert.count)),
            },
            "on the threat lists \u{b7} never answered".to_owned(),
        ],
        "new_device" => vec![
            match first {
                Some(first) => format!("{who} joined at {first}"),
                None => format!("{who} joined"),
            },
            format!(
                "no name yet \u{b7} {} by default",
                newcomer.word().to_lowercase()
            ),
        ],
        "arp_spoof" => vec![
            format!("{who} answers as router"),
            since_line(alert.count, clock_of(alert.first_unix, utc_offset_secs)),
        ],
        _ => vec![
            text,
            since_line(alert.count, clock_of(alert.first_unix, utc_offset_secs)),
        ],
    };
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    AlertRow::new(
        alert.id,
        alert.severity == Severity::High,
        &alert_title(&alert.kind),
        &lines,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use crate::live::{Counts, LastBlocked, WifiStatus};
    use qb_panel::Blocking;

    /// A Unix time that is midnight at UTC.
    pub(crate) const MIDNIGHT_UTC: u64 = 1_799_971_200;

    /// 10:20 in the morning at UTC, the reference clock.
    pub(crate) const TEN_TWENTY_UTC: u64 = MIDNIGHT_UTC + 37_200;

    /// A Wi-Fi link that is up.
    fn up() -> Option<WifiStatus> {
        Some(WifiStatus {
            associated: true,
            signal_dbm: Some(-63),
            ap: None,
            freq_mhz: None,
        })
    }

    /// A snapshot of a joined board whose resolver has counted `blocked` of `queries` today.
    fn counted(blocked: u64, queries: u64) -> Snapshot {
        Snapshot {
            wifi: up(),
            today: Some(Counts {
                queries,
                blocked,
                blocked_threat: 0,
                cache_hits: 0,
                forwarded: 0,
            }),
            ..Snapshot::default()
        }
    }

    /// The readings the reference stills show: 44% refused, and 10:20.
    pub(crate) fn reference_snapshot() -> Snapshot {
        Snapshot {
            last_blocked: Some(LastBlocked {
                name: "tracking-intake.logs.example.com".into(),
                unix_secs: TEN_TWENTY_UTC,
                count_24h: 4_419,
            }),
            upstream_ms: Some(8),
            clock_set: true,
            ..counted(4_780, 10_818)
        }
    }

    /// The stills for `snapshot` with blocking on and a lane that has just seen a name.
    fn stills_of(snapshot: &Snapshot) -> Stills {
        wanted_stills(snapshot, None, 0, TEN_TWENTY_UTC, 0)
    }

    #[test]
    fn the_reference_readings_give_44_percent_and_ten_twenty() {
        let wanted = stills_of(&reference_snapshot());
        assert_eq!(wanted.percent, Some(Share::Percent(44)));
        assert_eq!(
            wanted.clock,
            Some(TimeOfDay {
                hour: 10,
                minute: 20
            })
        );
        assert_eq!(wanted.blocking, Blocking::On);
        assert_eq!(wanted.gate_fill, 0);
        assert_eq!(wanted.count, Some(4_780));
        assert!(!wanted.gate_ask);
        assert_eq!(wanted.words, None, "names are going by");
    }

    #[test]
    fn nothing_known_gives_the_stills_of_dashes_and_the_joining_words() {
        // The Wi-Fi is the first thing to be known: until it is, every figure is a dash and the lane says why.
        assert_eq!(
            stills_of(&Snapshot::default()),
            Stills {
                words: Some(LaneWords::Joining),
                ..Stills::default()
            }
        );
    }

    #[test]
    fn nothing_counted_is_never_shown_while_the_wifi_is_not_joined() {
        // The counters of a board whose link is down or not yet up are not shown, joined or never known.
        let counters = Snapshot {
            wifi: None,
            ..reference_snapshot()
        };
        let dropped = Snapshot {
            wifi: Some(WifiStatus {
                associated: false,
                ..WifiStatus::default()
            }),
            ..reference_snapshot()
        };
        for snapshot in [counters, dropped] {
            let wanted = stills_of(&snapshot);
            assert_eq!(
                (wanted.percent, wanted.count, wanted.words),
                (None, None, Some(LaneWords::Joining))
            );
            // The clock follows its own flag.
            assert!(wanted.clock.is_some());
            // The words stay joining however long the lane has been quiet.
            assert_eq!(
                wanted_stills(&snapshot, None, 0, TEN_TWENTY_UTC, 100_000).words,
                Some(LaneWords::Joining)
            );
        }
    }

    #[test]
    fn a_joined_board_with_no_query_yet_is_listening_with_a_true_zero() {
        let listening = Some(LaneWords::Listening);
        let wanted = stills_of(&counted(0, 0));
        assert_eq!(
            (wanted.percent, wanted.count, wanted.words),
            (None, Some(0), listening)
        );
        // The resolver has not said anything about today: still listening, with the count a dash.
        let unknown = stills_of(&Snapshot {
            wifi: up(),
            ..Snapshot::default()
        });
        assert_eq!(
            (unknown.percent, unknown.count, unknown.words),
            (None, None, listening)
        );
        // However long ago a name was seen: nothing has come today.
        let long = wanted_stills(&counted(0, 0), None, 0, TEN_TWENTY_UTC, 100_000);
        assert_eq!(long.words, listening);
    }

    #[test]
    fn quiet_words_follow_the_idle_time_in_minutes_hours_and_days() {
        let words = |idle_secs| {
            wanted_stills(&reference_snapshot(), None, 0, TEN_TWENTY_UTC, idle_secs)
                .words
                .map(LaneWords::text)
        };
        assert_eq!(words(0), None);
        assert_eq!(words(119), None, "two minutes, not one second less");
        assert_eq!(words(120).as_deref(), Some("QUIET 2 MIN"));
        assert_eq!(words(179).as_deref(), Some("QUIET 2 MIN"));
        assert_eq!(words(47 * 60 + 59).as_deref(), Some("QUIET 47 MIN"));
        assert_eq!(words(119 * 60).as_deref(), Some("QUIET 119 MIN"));
        assert_eq!(words(2 * 3_600).as_deref(), Some("QUIET 2 H"));
        assert_eq!(words(3 * 3_600 + 59).as_deref(), Some("QUIET 3 H"));
        assert_eq!(words(47 * 3_600 + 3_599).as_deref(), Some("QUIET 47 H"));
        assert_eq!(words(48 * 3_600).as_deref(), Some("QUIET 2 D"));
        assert_eq!(words(9 * 86_400 + 5).as_deref(), Some("QUIET 9 D"));
        // Idle for longer than a clock can count: no panic, and the words still fit their 15 cells.
        let longest = words(u64::MAX).unwrap();
        assert_eq!(longest.chars().count(), 15, "{longest}");
    }

    #[test]
    fn quiet_words_change_only_when_their_text_does() {
        let words = |idle_secs| {
            wanted_stills(&reference_snapshot(), None, 0, TEN_TWENTY_UTC, idle_secs).words
        };
        let minutes = |idle_secs| match words(idle_secs) {
            Some(LaneWords::Quiet { minutes }) => minutes,
            other => panic!("{other:?}"),
        };
        // Whole hours, then whole days: the figure holds for the whole hour or day.
        assert_eq!(minutes(2 * 3_600), 120);
        assert_eq!(minutes(2 * 3_600 + 59 * 60 + 59), 120);
        assert_eq!(minutes(3 * 3_600), 180);
        assert_eq!(minutes(47 * 3_600 + 3_599), 47 * 60);
        assert_eq!(minutes(48 * 3_600), 2 * 24 * 60);
        assert_eq!(minutes(71 * 3_600 + 3_599), 2 * 24 * 60);
        assert_eq!(minutes(72 * 3_600), 3 * 24 * 60);
        // Whole minutes below two hours still tick over every minute.
        assert_eq!(minutes(120), 2);
        assert_eq!(minutes(179), 2);
        assert_eq!(minutes(180), 3);
        assert_eq!(minutes(119 * 60 + 59), 119);
        // Over the first four days, the still changes exactly when the text does.
        let (mut at, mut previous) = (QUIET_AFTER_SECS, words(QUIET_AFTER_SECS));
        let mut changes = 0;
        while at + 13 < 4 * 86_400 {
            at += 13;
            let now = words(at);
            let text = |words: Option<LaneWords>| words.map(LaneWords::text);
            assert_eq!(now == previous, text(now) == text(previous), "at {at} s");
            changes += usize::from(now != previous);
            previous = now;
        }
        // 2 MIN to 119 MIN is 117 changes, then 2 H (1), 3 H to 47 H (45), 2 D (1) and 3 D (1).
        assert_eq!(
            changes,
            117 + 1 + 45 + 1 + 1,
            "one still per change of text"
        );
    }

    #[test]
    fn the_percent_forms_reach_the_screen() {
        let of = |blocked, queries| stills_of(&counted(blocked, queries)).percent;
        let text = |blocked, queries| of(blocked, queries).map(Share::text);
        // No queries is no share: a dash. Nothing blocked is a true 0%.
        assert_eq!(text(0, 0), None);
        assert_eq!(text(0, 3).as_deref(), Some("0%"));
        // Something blocked that rounds to nothing is under one, not zero.
        assert_eq!(text(1, 1_000).as_deref(), Some("<1%"));
        assert_eq!(text(1, 201).as_deref(), Some("<1%"));
        assert_eq!(text(1, 200).as_deref(), Some("1%"));
        assert_eq!(text(4_780, 10_818).as_deref(), Some("44%"));
        // Never 100% unless it is all of them, and then it says ALL.
        assert_eq!(text(999, 1_000).as_deref(), Some("99%"));
        assert_eq!(text(5, 5).as_deref(), Some("ALL"));
        // More blocked than asked cannot be, but is not shown as more than all.
        assert_eq!(text(9, 5).as_deref(), Some("ALL"));
        assert_eq!(of(u64::MAX, u64::MAX), Some(Share::All));
        assert_eq!(of(0, u64::MAX), Some(Share::Zero));
        // No form of the share without the link.
        let dropped = Snapshot {
            wifi: None,
            ..counted(5, 5)
        };
        assert_eq!(stills_of(&dropped).percent, None);
    }

    #[test]
    fn the_share_rounds_to_the_nearest_and_never_says_more_than_the_truth() {
        let of = |blocked, queries| stills_of(&counted(blocked, queries)).percent;
        assert_eq!(of(0, 3), Some(Share::Zero));
        assert_eq!(of(1, 3), Some(Share::Percent(33)));
        assert_eq!(of(2, 3), Some(Share::Percent(67)));
        assert_eq!(of(1, 200), Some(Share::Percent(1)));
        assert_eq!(of(1, 201), Some(Share::UnderOne));
        assert_eq!(of(999, 1_000), Some(Share::Percent(99)));
        assert_eq!(of(5, 5), Some(Share::All));
        assert_eq!(of(9, 5), Some(Share::All));
        assert_eq!(of(u64::MAX, u64::MAX), Some(Share::All));
    }

    #[test]
    fn the_gate_refills_with_the_pause_running_out_and_not_otherwise() {
        let fill = |left| wanted_stills(&reference_snapshot(), left, 300, 0, 0).gate_fill;
        assert_eq!(fill(None), 0, "blocking is on");
        assert_eq!(fill(Some(300)), 0);
        assert_eq!(fill(Some(285)), 0);
        assert_eq!(fill(Some(277)), 1, "the mockup's 4:37");
        assert_eq!(fill(Some(150)), 9);
        assert_eq!(fill(Some(1)), 18);
        assert_eq!(fill(Some(0)), 18);
        assert_eq!(fill(Some(u16::MAX)), 0);
        // The refill is the same figure the loop gives the gate between two snapshots.
        assert_eq!(refill(Some(277), 300), 1);
        assert_eq!(refill(None, 300), 0);
        // Against the total of the pause: 30 and 60 minute gates refill from empty too.
        let filled =
            |left, total| wanted_stills(&reference_snapshot(), Some(left), total, 0, 0).gate_fill;
        assert_eq!(
            (filled(1_800, 1_800), filled(900, 1_800), filled(1, 1_800)),
            (0, 9, 18)
        );
        assert_eq!(
            (filled(3_600, 3_600), filled(1_800, 3_600), filled(1, 3_600)),
            (0, 9, 18)
        );
    }

    #[test]
    fn the_time_is_local_and_unknown_until_the_clock_is_set() {
        let unset = stills_of(&Snapshot {
            clock_set: false,
            ..reference_snapshot()
        });
        assert_eq!(unset.clock, None);
        let west = stills_of(&Snapshot {
            utc_offset_secs: -18_000,
            ..reference_snapshot()
        });
        assert_eq!(
            west.clock,
            Some(TimeOfDay {
                hour: 5,
                minute: 20
            })
        );
        // No underflow at the epoch.
        assert_eq!(
            time_of_day(0, -3_600),
            TimeOfDay {
                hour: 23,
                minute: 0
            }
        );
    }

    #[test]
    fn counters_past_the_panels_range_stop_at_its_end_and_a_pause_shows_its_seconds() {
        let huge = counted(u64::MAX, u64::MAX);
        let wanted = wanted_stills(&huge, Some(299), 300, 0, 0);
        assert_eq!(
            (wanted.count, wanted.percent, wanted.blocking),
            (
                Some(u32::MAX),
                Some(Share::All),
                Blocking::Paused { left_secs: 299 }
            )
        );
    }

    // ---- the world's parts

    use crate::panel::alerts::PanelAlert;
    use crate::watch::devices::{DeviceView, DevicesView, KnownList, Label, WallCopy, WallDevice};
    use qb_panel::os::world::MAX_DEVICES;

    /// The clock the wall tests are at, in Unix seconds: 10:20 at UTC.
    const NOW: u64 = TEN_TWENTY_UTC;

    /// The hardware address `02:00:00:00:00:<last>`.
    fn mac(last: u8) -> HardwareAddress {
        HardwareAddress([2, 0, 0, 0, 0, last])
    }

    /// A device at `192.0.2.<octet>` seen first at `first` and last just now, exempt or not, with no
    /// name.
    fn device(octet: u8, first: u64, exempt: bool) -> DeviceView {
        DeviceView {
            mac: mac(octet),
            ipv4: vec![Ipv4Addr::new(192, 0, 2, octet)],
            hostname: None,
            first_unix: first,
            last_unix: NOW,
            known: true,
            exempt,
            blocked: !exempt,
            label: None,
            label_rank: None,
            gate_left_secs: None,
            fixed: false,
            always: false,
        }
    }

    /// The same, with the owner's label, at `rank` in the label order.
    fn labelled(octet: u8, first: u64, text: &str, rank: usize) -> DeviceView {
        DeviceView {
            label: Label::new(text),
            label_rank: Some(rank),
            ..device(octet, first, true)
        }
    }

    /// The panel's copy of an inventory of `devices` in the list mode.
    fn inventory(devices: Vec<DeviceView>) -> WallCopy {
        WallCopy::of_view(&DevicesView {
            rev: 1,
            known_list: KnownList::Received,
            devices,
            exempt_macs: Vec::new(),
            exempt_pushed: Vec::new(),
            exempt_default: false,
            block_macs: Vec::new(),
            block_pushed: Vec::new(),
            own_mac: None,
            own_ip: None,
        })
    }

    /// The names of the wall, in order.
    fn names(wall: &[WallEntry]) -> Vec<&str> {
        wall.iter().map(|entry| entry.row.name.as_str()).collect()
    }

    #[test]
    fn the_gate_total_is_what_the_panel_opened_it_for_or_the_smallest_length_that_holds_what_is_left()
     {
        // The gate the panel opened: its own total, for as long as what is left fits it.
        assert_eq!(gate_total(Some(1_800), 1_800), 1_800);
        assert_eq!(gate_total(Some(1_800), 1_000), 1_800);
        assert_eq!(gate_total(Some(1_800), 1), 1_800);
        assert_eq!(gate_total(Some(300), 300), 300);
        // A pause that came from elsewhere: the smallest of 5, 30 and 60 minutes that holds what is left.
        for (left, total) in [
            (1, 300),
            (300, 300),
            (301, 1_800),
            (1_800, 1_800),
            (1_801, 3_600),
            (3_600, 3_600),
            // Past the longest key there is nothing to round up to: the pause is its own total.
            (3_601, 3_601),
            (7_200, 7_200),
        ] {
            assert_eq!(gate_total(None, left), total, "{left} s left");
        }
        // What is left is more than the panel remembers: the pause was replaced by another.
        assert_eq!(gate_total(Some(300), 1_700), 1_800);
        assert_eq!(gate_total(Some(300), 301), 1_800);
    }

    #[test]
    fn only_a_time_from_today_and_after_the_clock_was_set_is_a_time_of_day() {
        let ten = TimeOfDay {
            hour: 10,
            minute: 20,
        };
        assert_eq!(today_clock(NOW, NOW, 0), Some(ten));
        assert_eq!(
            today_clock(NOW - 3_600, NOW, 0),
            Some(TimeOfDay {
                hour: 9,
                minute: 20
            })
        );
        // Yesterday, and from before the clock was set, are none.
        assert_eq!(today_clock(NOW - 86_400, NOW, 0), None);
        assert_eq!(today_clock(3, NOW, 0), None);
        // The day is the local one: 10:20 at UTC is 05:20 at UTC-5 h. 06:20 at UTC is 01:20 the same day
        // there; 04:20 at UTC is 23:20 the day before.
        assert_eq!(
            today_clock(NOW - 4 * 3_600, NOW, -18_000),
            Some(TimeOfDay {
                hour: 1,
                minute: 20
            })
        );
        assert_eq!(today_clock(NOW - 6 * 3_600, NOW, -18_000), None);
        assert_eq!(clock_of(3, 0), None);
        assert_eq!(clock_of(NOW, 0), Some(ten));
    }

    #[test]
    fn the_wall_is_the_labelled_devices_in_their_order_then_the_others_by_first_sighting_then_the_router()
     {
        let mut router = device(1, NOW - 100_000, false);
        router.always = true;
        router.blocked = false;
        let mut printer = device(57, 1_799_000_100, true);
        printer.hostname = Some("printer".to_owned());
        let wall = wall(
            &inventory(vec![
                // The inventory lists the device seen last first; the wall does not.
                device(216, 1_799_000_900, true),
                router,
                printer,
                labelled(31, 1_799_000_300, "Living room TV", 2),
                device(143, 1_799_000_500, true),
                labelled(12, 1_799_000_800, "Desk PC", 0),
                labelled(23, 1_799_000_700, "Sam's phone", 1),
            ]),
            &[],
            0,
            Some(NOW),
        );
        assert_eq!(
            names(&wall),
            [
                "Desk PC",
                "Sam's phone",
                "Living room TV",
                "printer",
                "\u{b7}143",
                "\u{b7}216",
                "\u{b7}1"
            ]
        );
        // Each place knows the device by its hardware address and its octet.
        assert_eq!(wall[0].row.mac, [2, 0, 0, 0, 0, 12]);
        assert_eq!(wall[0].row.octet, 12);
        assert_eq!(wall[0].ipv4, [Ipv4Addr::new(192, 0, 2, 12)]);
        assert!(wall[0].row.named && wall[3].row.named && !wall[4].row.named);
        // The router is always so, and nothing else is.
        assert_eq!(wall[6].row.state, DeviceState::Always);
        assert!(
            wall[..6]
                .iter()
                .all(|entry| entry.row.state != DeviceState::Always)
        );
    }

    #[test]
    fn a_device_is_blocked_or_ads_pass_or_open_or_always_and_the_configuration_may_fix_it() {
        let mut open = device(2, 10, false);
        open.gate_left_secs = Some(277);
        open.exempt = true;
        let mut fixed = device(3, 20, true);
        fixed.fixed = true;
        let wall = wall(
            &inventory(vec![device(1, 5, false), open, fixed, device(4, 30, true)]),
            &[],
            1_000,
            Some(NOW),
        );
        let states: Vec<(DeviceState, bool)> = wall
            .iter()
            .map(|entry| (entry.row.state, entry.row.fixed))
            .collect();
        assert_eq!(
            states,
            [
                (DeviceState::Blocked, false),
                (DeviceState::Open { left_secs: 277 }, false),
                (DeviceState::AdsPass, true),
                (DeviceState::AdsPass, false),
            ]
        );
        // The open gate shuts at 277 s from the read, on the panel's own clock.
        assert_eq!(wall[1].gate_ends_ms, Some(1_000 + 277_000));
        assert_eq!(wall[0].gate_ends_ms, None);
    }

    #[test]
    fn an_open_gates_countdown_steps_every_second_between_reads() {
        let mut open = device(2, 10, false);
        open.gate_left_secs = Some(30);
        let mut wall = wall(&inventory(vec![open]), &[], 5_000, Some(NOW));
        let left = |entry: &WallEntry| match entry.row.state {
            DeviceState::Open { left_secs } => left_secs,
            other => panic!("{other:?}"),
        };
        assert_eq!(left(&wall[0]), 30);
        for (now_ms, secs) in [
            (5_000, 30),
            (5_999, 30),
            (6_000, 29),
            (6_001, 29),
            (34_000, 1),
            // Past its end until the next read says it is shut: not zero, which the key would show as `0:00`.
            (35_000, 1),
            (99_000, 1),
        ] {
            wall[0].tick(now_ms);
            assert_eq!(left(&wall[0]), secs, "at {now_ms}");
        }
        // A shut gate is left alone.
        let mut shut = wall_of_one(device(3, 10, false));
        shut.tick(50_000);
        assert_eq!(shut.row.state, DeviceState::Blocked);
    }

    /// The wall's one place for `device`.
    fn wall_of_one(device: DeviceView) -> WallEntry {
        wall(&inventory(vec![device]), &[], 0, Some(NOW)).remove(0)
    }

    #[test]
    fn a_device_is_asking_when_one_of_its_addresses_asked() {
        let mut two = device(8, 10, true);
        two.ipv4.push(Ipv4Addr::new(192, 0, 2, 80));
        let asking = [Ipv4Addr::new(192, 0, 2, 80), Ipv4Addr::new(192, 0, 2, 99)];
        let wall = wall(
            &inventory(vec![device(7, 5, true), two, device(9, 20, true)]),
            &asking,
            0,
            Some(NOW),
        );
        let lamps: Vec<bool> = wall.iter().map(|entry| entry.row.asking).collect();
        assert_eq!(lamps, [false, true, false]);
        // Its octet is the newest address's.
        assert_eq!(wall[1].row.octet, 8);
    }

    #[test]
    fn a_device_not_seen_for_a_day_is_off_the_wall_unless_it_has_a_label_or_the_clock_is_not_set() {
        let mut old = device(5, 10, true);
        old.last_unix = NOW - 86_401;
        let mut edge = device(6, 20, true);
        edge.last_unix = NOW - 86_400;
        let mut old_labelled = labelled(7, 30, "Attic", 0);
        old_labelled.last_unix = NOW - 10 * 86_400;
        // Seen before the clock was set: its time says nothing, so it stays.
        let mut early = device(8, 40, true);
        early.last_unix = 3_000;
        let devices = vec![old, edge, old_labelled, early];
        let on_wall = |now: Option<u64>| {
            wall(&inventory(devices.clone()), &[], 0, now)
                .iter()
                .map(|entry| entry.row.octet)
                .collect::<Vec<u8>>()
        };
        assert_eq!(on_wall(Some(NOW)), [7, 6, 8]);
        // With no clock nothing is too old.
        assert_eq!(on_wall(None), [7, 5, 6, 8]);
    }

    #[test]
    fn at_most_thirty_three_places_and_the_router_and_the_pi_are_the_last_to_go() {
        let mut devices: Vec<DeviceView> = (10..60)
            .map(|octet| device(octet, 1_000 + u64::from(octet), true))
            .collect();
        let mut router = device(1, 5, false);
        router.always = true;
        devices.push(router);
        let mut view = inventory(devices);
        view.own_mac = Some(mac(2));
        view.own_ip = Some(Ipv4Addr::new(192, 0, 2, 2));
        let wall = wall(&view, &[], 0, Some(NOW));
        assert_eq!(wall.len(), MAX_DEVICES);
        // The first of the others by first sighting, 31 of them, then the router, then the Pi.
        assert_eq!(wall[0].row.octet, 10);
        assert_eq!(wall[30].row.octet, 40);
        assert_eq!(wall[31].row.octet, 1);
        assert_eq!(wall[32].row.name, "qberrypi");
        assert_eq!(wall[32].row.state, DeviceState::Always);
        assert_eq!(wall[32].row.octet, 2);
        assert_eq!(wall[32].row.mac, [2, 0, 0, 0, 0, 2]);
        assert_eq!(wall[32].ipv4, [Ipv4Addr::new(192, 0, 2, 2)]);
        // The Pi asks too: its place has a lamp.
        let asking = wall_of_pi(&view, &[Ipv4Addr::new(192, 0, 2, 2)]);
        assert!(asking);
    }

    /// Whether the Pi's place on the wall is asking.
    fn wall_of_pi(view: &WallCopy, asking: &[Ipv4Addr]) -> bool {
        wall(view, asking, 0, Some(NOW)).last().unwrap().row.asking
    }

    #[test]
    fn the_pi_is_a_place_of_its_own_once_its_hardware_address_is_known_and_never_twice() {
        let mut view = inventory(vec![device(12, 1, true)]);
        assert_eq!(names(&wall(&view, &[], 0, Some(NOW))), ["\u{b7}12"]);
        view.own_mac = Some(mac(2));
        let with_pi = wall(&view, &[], 0, Some(NOW));
        // No address given: the octet is 0, as for any device with none.
        assert_eq!(names(&with_pi), ["\u{b7}12", "qberrypi"]);
        // A Pi that the inventory lists as a device (it was heard) is the one place, from the inventory.
        let mut listed = device(2, 2, false);
        listed.always = true;
        view.rows.push(WallDevice::of_view(&listed));
        let wall = wall(&view, &[], 0, Some(NOW));
        assert_eq!(wall.len(), 2);
        assert_eq!(wall[1].row.state, DeviceState::Always);
    }

    #[test]
    fn a_newcomer_is_blocked_in_the_list_mode_and_lets_ads_pass_in_the_exempt_by_default_mode() {
        let mut view = inventory(Vec::new());
        assert_eq!(newcomer_of(&view), Newcomer::Blocked);
        view.exempt_default = true;
        assert_eq!(newcomer_of(&view), Newcomer::AdsPass);
    }

    /// A threat alert the way the store carries one, about the device at `192.0.2.23`.
    fn alert(kind: &str, client: Option<&str>, subject: &str) -> PanelAlert {
        PanelAlert {
            id: 9,
            kind: kind.to_owned(),
            severity: if kind == "new_device" {
                Severity::Warn
            } else {
                Severity::High
            },
            client: client.map(|text| text.parse().unwrap()),
            subject: subject.to_owned(),
            count: 3,
            first_unix: NOW - 3_600,
            last_unix: NOW - 60,
        }
    }

    /// The wall of the house of these tests: Sam's phone is `192.0.2.23`.
    fn house_wall() -> Vec<WallEntry> {
        wall(
            &inventory(vec![
                labelled(23, 10, "Sam's phone", 0),
                device(216, 20, true),
            ]),
            &[],
            0,
            Some(NOW),
        )
    }

    fn lines(row: &AlertRow) -> Vec<&str> {
        row.lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn a_threat_alert_says_the_name_who_asked_how_often_and_that_it_was_never_answered() {
        let row = alert_row(
            &alert("threat", Some("192.0.2.23"), "secure-login-verify.example"),
            &house_wall(),
            Newcomer::AdsPass,
            0,
        );
        assert!(row.high);
        assert_eq!(row.id, 9);
        assert_eq!(row.title, "THREAT BLOCKED");
        assert_eq!(
            lines(&row),
            [
                "secure-login-verify.example",
                "Sam's phone asked 3 times by 10:19",
                "on the threat lists \u{b7} never answered"
            ]
        );
        // A device the wall does not hold is its octet; one that asked once says so.
        let mut once = alert("threat", Some("192.0.2.77"), "ads.example");
        once.count = 1;
        let row = alert_row(&once, &house_wall(), Newcomer::AdsPass, 0);
        assert_eq!(lines(&row)[1], "\u{b7}77 asked once by 10:19");
        // No device at all.
        let row = alert_row(
            &alert("threat", None, "ads.example"),
            &[],
            Newcomer::AdsPass,
            0,
        );
        assert_eq!(lines(&row)[1], "a device asked 3 times by 10:19");
    }

    #[test]
    fn a_new_device_alert_says_when_it_joined_and_what_it_gets_by_default() {
        let row = alert_row(
            &alert("new_device", None, "02:00:00:00:00:d8"),
            &house_wall(),
            Newcomer::AdsPass,
            0,
        );
        assert!(!row.high);
        assert_eq!(row.title, "NEW DEVICE");
        // 02:00:00:00:00:d8 is 216: the wall has it, nameless.
        assert_eq!(
            lines(&row),
            [
                "\u{b7}216 joined at 09:20",
                "no name yet \u{b7} ads pass by default"
            ]
        );
        let row = alert_row(
            &alert("new_device", None, "02:00:00:00:00:99"),
            &house_wall(),
            Newcomer::Blocked,
            0,
        );
        assert_eq!(
            lines(&row),
            [
                "a device joined at 09:20",
                "no name yet \u{b7} blocked by default"
            ]
        );
    }

    #[test]
    fn an_arp_spoof_alert_names_the_device_that_answers_as_the_router_and_how_often() {
        let row = alert_row(
            &alert("arp_spoof", Some("192.0.2.1"), "02:00:00:00:00:17"),
            &house_wall(),
            Newcomer::AdsPass,
            0,
        );
        assert!(row.high);
        assert_eq!(row.title, "ARP SPOOF");
        // 0x17 is 23: Sam's phone.
        assert_eq!(
            lines(&row),
            ["Sam's phone answers as router", "3 times since 09:20"]
        );
    }

    #[test]
    fn any_other_alert_says_its_subject_and_how_often_since_when_and_a_title_for_its_kind() {
        for (kind, title) in [
            ("rebind", "REBINDING BLOCKED"),
            ("dga", "SUSPICIOUS LOOKUPS"),
            ("tunnel", "TUNNEL SUSPECTED"),
            ("beacon", "REGULAR CALLS"),
            ("burst", "LOOKUP BURST"),
            ("some_new_kind", "SOME NEW KIND"),
        ] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.23"), "random NXDOMAIN names"),
                &house_wall(),
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.title, title);
            assert_eq!(
                lines(&row),
                ["random NXDOMAIN names", "3 times since 09:20"]
            );
        }
    }

    #[test]
    fn the_alerts_times_are_local_left_out_before_the_clock_was_set_and_a_long_subject_is_fitted() {
        // At UTC-5 h the alert's 09:20 and 10:19 are 04:20 and 05:19.
        let row = alert_row(
            &alert("threat", Some("192.0.2.23"), "ads.example"),
            &house_wall(),
            Newcomer::AdsPass,
            -18_000,
        );
        assert_eq!(lines(&row)[1], "Sam's phone asked 3 times by 05:19");
        // Raised before the clock was set: no time is better than the wrong one.
        let mut early = alert("beacon", None, "ads.example");
        early.first_unix = 40;
        early.last_unix = 90;
        assert_eq!(
            lines(&alert_row(&early, &[], Newcomer::AdsPass, 0))[1],
            "3 times"
        );
        early.count = 1;
        assert_eq!(
            lines(&alert_row(&early, &[], Newcomer::AdsPass, 0))[1],
            "once"
        );
        let mut threat = alert("threat", None, "ads.example");
        threat.last_unix = 90;
        assert_eq!(
            lines(&alert_row(&threat, &[], Newcomer::AdsPass, 0))[1],
            "a device asked 3 times"
        );
        // A subject of 200 letters is fitted to its line, keeping the site.
        let long = format!("{}.example.org", "x".repeat(200));
        let row = alert_row(&alert("threat", None, &long), &[], Newcomer::AdsPass, 0);
        assert!(row.lines[0].chars().count() <= 36, "{}", row.lines[0]);
        assert!(row.lines[0].ends_with("example.org") || row.lines[0].ends_with('~'));
        // A count that is large is written as the panel writes counts.
        let mut many = alert("burst", None, "question rate");
        many.count = 1_204;
        assert_eq!(
            lines(&alert_row(&many, &[], Newcomer::AdsPass, 0))[1],
            "1,204 times since 09:20"
        );
    }

    #[test]
    fn a_countdown_does_not_step_back_up_when_the_inventory_is_read_again_but_a_new_gate_is_new() {
        let open = |secs: u16| {
            let mut device = device(2, 10, false);
            device.gate_left_secs = Some(secs);
            device
        };
        let first = wall(&inventory(vec![open(60)]), &[], 10_000, Some(NOW));
        assert_eq!(first[0].gate_ends_ms, Some(70_000));
        // Read again two seconds later, at 12.9 s: the resolver's whole seconds say 58, 0.9 s later than
        // the panel counted. It was showing 58 (ends 70.0 s), and is to go on doing so.
        let mut second = wall(&inventory(vec![open(58)]), &[], 12_900, Some(NOW));
        assert_eq!(second[0].gate_ends_ms, Some(70_900));
        keep_countdowns(&first, &mut second, 12_900);
        assert_eq!(second[0].gate_ends_ms, Some(70_000));
        assert_eq!(second[0].row.state, DeviceState::Open { left_secs: 58 });
        // A read that says it ends sooner is believed.
        let mut sooner = wall(&inventory(vec![open(55)]), &[], 12_900, Some(NOW));
        keep_countdowns(&first, &mut sooner, 12_900);
        assert_eq!(sooner[0].gate_ends_ms, Some(67_900));
        // So is one that says it was opened again: it ends much later.
        let mut again = wall(&inventory(vec![open(300)]), &[], 12_900, Some(NOW));
        keep_countdowns(&first, &mut again, 12_900);
        assert_eq!(again[0].gate_ends_ms, Some(312_900));
        // Another device's gate, or none before, is left as it is.
        let mut other = wall(
            &inventory(vec![device(3, 10, false), open(58)]),
            &[],
            12_900,
            Some(NOW),
        );
        keep_countdowns(&first, &mut other, 12_900);
        // (The wall is in the order of first sighting, then address: the open one is the first.)
        assert_eq!(
            other[0].gate_ends_ms,
            Some(70_000),
            "the same device is the same"
        );
        assert_eq!(other[1].gate_ends_ms, None);
    }
}
