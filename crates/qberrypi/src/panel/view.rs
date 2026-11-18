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
    ALERT_LINE_CELLS, AlertRow, AlertStamp, Band, DEVICE_NAME_CELLS, DayWord, DeviceRow,
    DeviceState, LastUpdate, Link, MAX_DEVICES, MAX_OWNER_ROWS, MAX_TOP_NAMES, Moment,
    NameClass as WorldNameClass, NameFacts, NameRow, Newcomer, OwnerEntry as WorldOwnerEntry,
    OwnerRow, OwnerVerdict, Slot as WorldSlot, UpdatePhase, UpdateRead, Waiting as WorldWaiting,
    Weekday, When, display_name,
};
use qb_panel::{Blocking, LaneWords, Share, TimeOfDay};

use crate::live::{Snapshot, WifiStatus};
use crate::names::{OwnerEntry, OwnerHold};
use crate::panel::alerts::{PanelAlert, Severity};
use crate::panel::names::{NameClass, OwnerSide, TopName};
use crate::slots::Slot;
use crate::update::{OutcomeKind, View as UpdateView, Waiting};
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

/// The device an alert is about, as the wall has it: for a `new_device` and an `arp_spoof` the alert's subject is
/// the device's hardware address; for any other kind the alert's client is its address. The wall's place for it,
/// if the wall holds it.
fn device_entry<'a>(alert: &PanelAlert, wall: &'a [WallEntry]) -> Option<&'a WallEntry> {
    if matches!(alert.kind.as_str(), "new_device" | "arp_spoof") {
        let mac = alert.subject.parse::<HardwareAddress>().ok()?;
        return wall.iter().find(|entry| entry.row.mac == mac.0);
    }
    match alert.client {
        Some(IpAddr::V4(ip)) => wall.iter().find(|entry| entry.ipv4.contains(&ip)),
        _ => None,
    }
}

/// What the panel calls the device an alert is about: the label, the hostname or `·octet` as the wall has it;
/// `·octet` of the client's address when the wall does not hold the device; `None` when it is not known at all
/// (a device alert whose hardware address the wall does not hold, or an alert with no client).
fn device_word(alert: &PanelAlert, wall: &[WallEntry]) -> Option<String> {
    if let Some(entry) = device_entry(alert, wall) {
        return Some(entry.row.name.clone());
    }
    match alert.client {
        Some(IpAddr::V4(ip)) if !matches!(alert.kind.as_str(), "new_device" | "arp_spoof") => {
            Some(format!("\u{b7}{}", ip.octets()[3]))
        }
        _ => None,
    }
}

/// A name as the lists and the owner's entries hold it: lower case, no trailing dot.
fn plain_name(text: &str) -> String {
    text.trim_end_matches('.').to_ascii_lowercase()
}

/// The weekday of `unix_secs` in a zone `utc_offset_secs` from UTC. The epoch's day, 1 January 1970, was a Thursday.
pub fn weekday_of(unix_secs: u64, utc_offset_secs: i32) -> Weekday {
    match (local_day(unix_secs, utc_offset_secs) + 3).rem_euclid(7) {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// The weekday and the time of day of `unix_secs`, or `None` for a time from before the clock was set.
pub fn moment_of(unix_secs: u64, utc_offset_secs: i32) -> Option<Moment> {
    clock_of(unix_secs, utc_offset_secs).map(|time| Moment {
        day: weekday_of(unix_secs, utc_offset_secs),
        time,
    })
}

/// When an alert last happened, as the log says it: the time of day for today's, the weekday for an older one;
/// `None` for a time from before the clock was set. With the clock not set now (`now_unix` before it), nothing can
/// be told from today, and the time of day is given.
pub fn when_of(unix_secs: u64, now_unix: u64, utc_offset_secs: i32) -> Option<When> {
    let time = clock_of(unix_secs, utc_offset_secs)?;
    let older = now_unix >= CLOCK_SANE_UNIX
        && local_day(unix_secs, utc_offset_secs) != local_day(now_unix, utc_offset_secs);
    Some(if older {
        When::Day(weekday_of(unix_secs, utc_offset_secs))
    } else {
        When::Time(time)
    })
}

/// The day an entry of the owner's list was set on the panel, as YOUR LIST says it (spec §12.8): `TODAY`, or the
/// weekday; `None` for an entry the desktop pushed, or one set before the clock was.
pub fn day_word(set_unix: Option<u64>, now_unix: u64, utc_offset_secs: i32) -> Option<DayWord> {
    let set = set_unix.filter(|&unix| unix >= CLOCK_SANE_UNIX)?;
    Some(
        if now_unix >= CLOCK_SANE_UNIX
            && local_day(set, utc_offset_secs) != local_day(now_unix, utc_offset_secs)
        {
            DayWord::On(weekday_of(set, utc_offset_secs))
        } else {
            DayWord::Today
        },
    )
}

/// The alert the panel shows, as the log's line and the slip it opens (spec §5.3, §12.2, ruling R11, R8): the title
/// for its kind, up to three lines, `URGENT` (white) for a high alert, the stamp of what the Pi did, the device
/// as the wall names it and its address for `SEE DEVICE`, and, for an alert about a name, the name the slip's name
/// line opens. `wall` is the panel's last copy of the wall, `now_unix` the clock.
///
/// One `match` on the kind decides all of it:
///
/// - `threat`: the name; `<device> asked n times by hh:mm`; `on the threat lists · never answered`. Stamp
///   `REFUSED`; the name is a door to NAME.
/// - `rebind`: the subject and `n times since hh:mm`. Stamp `REFUSED`; the name is a door.
/// - `new_device`: `<device> joined at hh:mm`; `no name yet · <default word> by default`. Stamp `ADMITTED`.
/// - `arp_spoof`: `<device> answers as router`; `n times since hh:mm`. Stamp `OBSERVED`.
/// - `dga`, `tunnel`, `beacon`, `burst` and any other kind: the subject and `n times since hh:mm`. `OBSERVED`.
///
/// A time is left out while the clock was not set when it was taken. Nothing here is kept, or logged.
pub fn alert_row(
    alert: &PanelAlert,
    wall: &[WallEntry],
    newcomer: Newcomer,
    utc_offset_secs: i32,
    now_unix: u64,
) -> AlertRow {
    let at = |unix| clock_of(unix, utc_offset_secs).map(|time| format::clock(Some(time)));
    let (first, last) = (at(alert.first_unix), at(alert.last_unix));
    let who = device_word(alert, wall);
    let device = who.as_deref().unwrap_or("a device");
    let subject = || fit_text(&alert.subject, ALERT_LINE_CELLS[0]);
    let since = || since_line(alert.count, clock_of(alert.first_unix, utc_offset_secs));
    // The title, the stamp, the lines, and whether the subject is a name the slip opens.
    let (title, stamp, lines, door): (String, AlertStamp, Vec<String>, bool) =
        match alert.kind.as_str() {
            "threat" => (
                "THREAT BLOCKED".to_owned(),
                AlertStamp::Refused,
                vec![
                    fit_site(&alert.subject, ALERT_LINE_CELLS[0]),
                    match last {
                        Some(last) => format!("{device} asked {} by {last}", times(alert.count)),
                        None => format!("{device} asked {}", times(alert.count)),
                    },
                    "on the threat lists \u{b7} never answered".to_owned(),
                ],
                true,
            ),
            "rebind" => (
                "REBINDING BLOCKED".to_owned(),
                AlertStamp::Refused,
                vec![subject(), since()],
                true,
            ),
            "new_device" => (
                "NEW DEVICE".to_owned(),
                AlertStamp::Admitted,
                vec![
                    match first {
                        Some(first) => format!("{device} joined at {first}"),
                        None => format!("{device} joined"),
                    },
                    format!(
                        "no name yet \u{b7} {} by default",
                        newcomer.word().to_lowercase()
                    ),
                ],
                false,
            ),
            "arp_spoof" => (
                "ARP SPOOF".to_owned(),
                AlertStamp::Observed,
                vec![format!("{device} answers as router"), since()],
                false,
            ),
            "dga" => observed("SUSPICIOUS LOOKUPS", subject(), since()),
            "tunnel" => observed("TUNNEL SUSPECTED", subject(), since()),
            "beacon" => observed("REGULAR CALLS", subject(), since()),
            "burst" => observed("LOOKUP BURST", subject(), since()),
            // A kind that is not known is spelt out of itself.
            other => observed(&other.replace('_', " ").to_uppercase(), subject(), since()),
        };
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut row = AlertRow::new(alert.id, alert.severity == Severity::High, &title, &lines);
    row.at = when_of(alert.last_unix, now_unix, utc_offset_secs);
    row.who = fit_text(who.as_deref().unwrap_or(""), DEVICE_NAME_CELLS);
    row.stamp = stamp;
    row.name = door.then(|| plain_name(&alert.subject));
    row.device = device_entry(alert, wall).map(|entry| entry.row.mac);
    row
}

/// The slip of a kind that blocked nothing because of it: its subject and how often, stamped `OBSERVED`, with no
/// name to open.
fn observed(
    title: &str,
    subject: String,
    since: String,
) -> (String, AlertStamp, Vec<String>, bool) {
    (
        title.to_owned(),
        AlertStamp::Observed,
        vec![subject, since],
        false,
    )
}

/// The world's word for which list holds a name.
fn world_class(class: NameClass) -> WorldNameClass {
    match class {
        NameClass::None => WorldNameClass::None,
        NameClass::Ads => WorldNameClass::Ads,
        NameClass::Deny => WorldNameClass::Deny,
        NameClass::Threat => WorldNameClass::Threat,
    }
}

/// What the owner decided, as the world says it.
fn world_verdict(side: OwnerSide) -> OwnerVerdict {
    match side {
        OwnerSide::Allow => OwnerVerdict::Allow,
        OwnerSide::Refuse => OwnerVerdict::Refuse,
    }
}

/// The pipes of NAMES from the day's most refused names, most refused first (`top` is as the resolver counted
/// them, at most eight): the first `MAX_TOP_NAMES`, each with its count and whether the owner has let it through
/// (`holds` says what the owner's list holds of a name). Names are whole; the screen fits them.
pub fn top_rows(top: &[TopName], holds: &dyn Fn(&str) -> Option<OwnerHold>) -> Vec<NameRow> {
    top.iter()
        .take(MAX_TOP_NAMES)
        .map(|row| NameRow {
            name: row.name.to_string(),
            count: row.count,
            open: holds(&row.name).is_some_and(|hold| hold.side == OwnerSide::Allow),
        })
        .collect()
}

/// The valves of YOUR LIST from the newest entries of the owner's list, the newest first: the name, the side and
/// the day it was set on the panel (none for an entry the desktop pushed, spec §12.8).
pub fn owner_rows(newest: &[OwnerEntry], now_unix: u64, utc_offset_secs: i32) -> Vec<OwnerRow> {
    newest
        .iter()
        .take(MAX_OWNER_ROWS)
        .map(|entry| OwnerRow {
            name: entry.name.to_string(),
            verdict: world_verdict(entry.side),
            day: day_word(entry.set_unix, now_unix, utc_offset_secs),
        })
        .collect()
}

/// The facts NAME shows of `name` (spec §3.7, §12.7): which list holds it, the owner's entry on it with the day,
/// and how many times it was refused today when the Pi counts that (the day's most refused only, in `top`) and
/// the name is refused now. An answered name is not counted: the Pi counts only what it blocks (spec §11.3).
///
/// An owner's entry that covers the name through a parent counts as the owner's word on it, as the resolver
/// decides it; only an entry of the name itself can be taken back (`NamesControl::forget`).
pub fn name_facts(
    name: &str,
    class: NameClass,
    hold: Option<OwnerHold>,
    top: &[TopName],
    now_unix: u64,
    utc_offset_secs: i32,
) -> NameFacts {
    let mut facts = NameFacts {
        name: name.to_owned(),
        class: world_class(class),
        owner: hold.map(|hold| WorldOwnerEntry {
            verdict: world_verdict(hold.side),
            day: day_word(hold.set_unix, now_unix, utc_offset_secs),
        }),
        count: None,
    };
    if facts.shut() {
        facts.count = top
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(name))
            .map(|row| row.count);
    }
    facts
}

// ---------------------------------------------------------------------------------------------------------------
// SYSTEM, the states of the whole system and the night (phase 3)

/// Whether `time` is in the night hours `night`, `(from, to)` as hours of the day: from `from` until `to`, past
/// midnight when `from` is the later (`23-07`). No night, the two hours equal, or a clock that is not set is
/// never in the hours.
pub fn in_night(night: Option<(u8, u8)>, time: Option<TimeOfDay>) -> bool {
    let (Some((from, to)), Some(time)) = (night, time) else {
        return false;
    };
    match from.cmp(&to) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => (from..to).contains(&time.hour),
        std::cmp::Ordering::Greater => time.hour >= from || time.hour < to,
    }
}

/// What WI-FI and SYSTEM's link read of the Wi-Fi: the band from the frequency, the signal, how many whole days the
/// link has held at the monotonic second `now_secs` (the clock `WifiStatus::joined_secs` is on) and the upstream's
/// answer time. Nothing is known of a link that is not up: the figures are the dashes.
pub fn link_of(wifi: Option<&WifiStatus>, upstream_ms: Option<u16>, now_secs: u64) -> Link {
    let Some(wifi) = wifi.filter(|wifi| wifi.associated) else {
        return Link::default();
    };
    Link {
        band: wifi.freq_mhz.and_then(Band::from_mhz),
        signal_dbm: wifi.signal_dbm.map(i16::from),
        linked_days: wifi.linked_secs(now_secs).map(|secs| held(secs / 86_400)),
        upstream_ms: upstream_ms.map(u32::from),
    }
}

/// The date of a Unix time at UTC as `2026-11-12`: the day a build was made, which VERSION says.
pub fn date_of(unix_secs: u64) -> String {
    // Howard Hinnant's `civil_from_days`, counted from 1 March of year 0.
    let z = unix_secs / 86_400 + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A slot as the screens name it.
pub fn slot_of(slot: Slot) -> WorldSlot {
    match slot {
        Slot::A => WorldSlot::A,
        Slot::B => WorldSlot::B,
    }
}

/// The slot that is not `slot`.
fn other_slot(slot: WorldSlot) -> WorldSlot {
    match slot {
        WorldSlot::A => WorldSlot::B,
        WorldSlot::B => WorldSlot::A,
    }
}

/// The update that is under way, as UPDATE draws it, from what the update machinery says: one being received
/// (the slot that serves is the left box, the other the right), or this boot's trial (the slot a rollback boots is
/// the left box, the running one the right, with the seconds to the trial's minimum time and, once they are up,
/// what it still waits for). `None` while none is going on, and on the old layout, which has no slots.
pub fn update_read(view: &UpdateView) -> Option<UpdateRead> {
    let running = slot_of(view.slot?);
    match (view.state, &view.trial) {
        ("receiving", _) => Some(UpdateRead {
            phase: UpdatePhase::Receiving {
                received: view.received,
                length: view.length,
            },
            before: running,
            after: other_slot(running),
        }),
        ("trial", Some(trial)) => {
            let waiting = |what: Waiting| trial.waiting_for.contains(&what);
            let waiting_for = if waiting(Waiting::Wifi) {
                Some(WorldWaiting::Wifi)
            } else if waiting(Waiting::Clock) {
                Some(WorldWaiting::Clock)
            } else if waiting(Waiting::Upstream) {
                Some(WorldWaiting::Upstream)
            } else {
                None
            };
            Some(UpdateRead {
                phase: UpdatePhase::Trial {
                    left_secs: u16::try_from(trial.min_left_secs).unwrap_or(u16::MAX),
                    waiting_for,
                },
                before: slot_of(trial.from),
                after: running,
            })
        }
        _ => None,
    }
}

/// How the last trial ended, as UPDATE shows it once: the finished update with its slots (the box that served before
/// is the old build's, which is the running slot after a rollback and the other after a confirm), and the outcome's
/// identity, the kind and the build, by which the panel knows it has shown it. `None` while an update is under way
/// (that screen is the live one), when no trial this boot knows of has ended, and on the old layout.
pub fn finished_update(view: &UpdateView) -> Option<(UpdateRead, (OutcomeKind, u64))> {
    if view.state != "idle" {
        return None;
    }
    let outcome = view.last.as_ref()?;
    let running = slot_of(view.slot?);
    let (phase, before, after) = match outcome.kind {
        OutcomeKind::Confirmed => (UpdatePhase::Confirmed, other_slot(running), running),
        OutcomeKind::RolledBack => (UpdatePhase::RolledBack, running, other_slot(running)),
    };
    Some((
        UpdateRead {
            phase,
            before,
            after,
        },
        (outcome.kind, outcome.build),
    ))
}

/// How the last update ended, as the box on SYSTEM and VERSION say it.
pub fn last_update_of(view: &UpdateView) -> LastUpdate {
    match view.last.as_ref().map(|outcome| outcome.kind) {
        Some(OutcomeKind::Confirmed) => LastUpdate::Confirmed,
        Some(OutcomeKind::RolledBack) => LastUpdate::RolledBack,
        None => LastUpdate::None,
    }
}

/// What the board last restarted for, in the words UP says, as far as this boot knows: the board keeps no record
/// across a restart (the Pi writes nothing to the card for it), so only an update shows, in the trial it began and
/// the outcome it left. Empty when nothing says; UP then shows a dash.
pub fn restart_cause(view: &UpdateView) -> &'static str {
    match (&view.trial, view.last.as_ref().map(|outcome| outcome.kind)) {
        (Some(_), _) | (None, Some(OutcomeKind::Confirmed)) => "an update",
        (None, Some(OutcomeKind::RolledBack)) => "an update that rolled back",
        (None, None) => "",
    }
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
            joined_secs: None,
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

    /// The row of an alert with the clock at `NOW`.
    fn alert_row(
        alert: &PanelAlert,
        wall: &[WallEntry],
        newcomer: Newcomer,
        utc_offset_secs: i32,
    ) -> AlertRow {
        super::alert_row(alert, wall, newcomer, utc_offset_secs, NOW)
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
    fn the_stamp_is_what_the_pi_did_decided_by_the_kind_alone() {
        // Spec §12.2: a blocked answer is `REFUSED`, a device that joined `ADMITTED`, and whatever blocked
        // nothing `OBSERVED`.
        for (kind, stamp) in [
            ("threat", AlertStamp::Refused),
            ("rebind", AlertStamp::Refused),
            ("new_device", AlertStamp::Admitted),
            ("arp_spoof", AlertStamp::Observed),
            ("dga", AlertStamp::Observed),
            ("tunnel", AlertStamp::Observed),
            ("beacon", AlertStamp::Observed),
            ("burst", AlertStamp::Observed),
            ("some_new_kind", AlertStamp::Observed),
        ] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.23"), "02:00:00:00:00:17"),
                &house_wall(),
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.stamp, stamp, "{kind}");
        }
    }

    #[test]
    fn a_high_alert_is_urgent_and_the_others_routine() {
        let high = alert_row(
            &alert("burst", None, "question rate"),
            &[],
            Newcomer::AdsPass,
            0,
        );
        assert_eq!((high.high, high.precedence()), (true, "URGENT"));
        let low = alert_row(
            &PanelAlert {
                severity: Severity::Info,
                ..alert("burst", None, "question rate")
            },
            &[],
            Newcomer::AdsPass,
            0,
        );
        assert_eq!((low.high, low.precedence()), (false, "ROUTINE"));
    }

    #[test]
    fn only_a_threat_and_a_rebind_open_their_name_and_it_is_whole_and_plain() {
        for kind in ["threat", "rebind"] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.23"), "Secure-Login.Example."),
                &house_wall(),
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.name.as_deref(), Some("secure-login.example"), "{kind}");
        }
        // A name too long for its line is still opened whole.
        let long = format!("{}.example.org", "x".repeat(200));
        let row = alert_row(&alert("threat", None, &long), &[], Newcomer::AdsPass, 0);
        assert_eq!(row.name.as_deref(), Some(long.as_str()));
        for kind in [
            "new_device",
            "arp_spoof",
            "dga",
            "tunnel",
            "beacon",
            "burst",
            "other",
        ] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.23"), "02:00:00:00:00:17"),
                &house_wall(),
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.name, None, "{kind}");
        }
    }

    #[test]
    fn see_device_names_the_device_on_the_wall_by_its_address_or_its_hardware_address_and_none_otherwise()
     {
        let sam = [2, 0, 0, 0, 0, 23];
        let wall = house_wall();
        // The client's address, for the kinds about a name.
        for kind in ["threat", "rebind", "dga", "beacon"] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.23"), "ads.example"),
                &wall,
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.device, Some(sam), "{kind}");
            assert_eq!(row.who, "Sam's phone", "{kind}");
        }
        // The subject's hardware address, for the kinds about a device.
        for kind in ["new_device", "arp_spoof"] {
            let row = alert_row(
                &alert(kind, Some("192.0.2.1"), "02:00:00:00:00:17"),
                &wall,
                Newcomer::AdsPass,
                0,
            );
            assert_eq!(row.device, Some(sam), "{kind}");
        }
        // A client the wall does not hold: named by its octet, and no door to open.
        let stranger = alert_row(
            &alert("threat", Some("192.0.2.77"), "ads.example"),
            &wall,
            Newcomer::AdsPass,
            0,
        );
        assert_eq!((stranger.device, stranger.who.as_str()), (None, "\u{b7}77"));
        // A hardware address the wall does not hold, and an alert with no client at all: no device.
        for (kind, client, subject) in [
            ("new_device", None, "02:00:00:00:00:99"),
            ("arp_spoof", Some("192.0.2.1"), "02:00:00:00:00:98"),
            ("threat", None, "ads.example"),
            ("threat", Some("fe80::1"), "ads.example"),
        ] {
            let row = alert_row(&alert(kind, client, subject), &wall, Newcomer::AdsPass, 0);
            assert_eq!((row.device, row.who.as_str()), (None, ""), "{kind}");
        }
        // The first reading of an alert with no wall yet is what the loop makes again once the wall comes.
        let early = alert_row(
            &alert("threat", Some("192.0.2.23"), "ads.example"),
            &[],
            Newcomer::AdsPass,
            0,
        );
        assert_eq!((early.device, early.who.as_str()), (None, "\u{b7}23"));
    }

    #[test]
    fn an_alert_says_when_it_last_happened_as_a_time_today_and_as_a_weekday_before() {
        let at = |last: u64, offset: i32| when_of(last, NOW, offset).map(When::word);
        assert_eq!(at(NOW - 60, 0).as_deref(), Some("10:19"));
        // Yesterday 10:20 was a Thursday; the day before that a Wednesday.
        assert_eq!(at(NOW - 86_400, 0).as_deref(), Some("THU"));
        assert_eq!(at(NOW - 2 * 86_400, 0).as_deref(), Some("WED"));
        // Midnight itself is today; a second before it is not.
        assert_eq!(at(MIDNIGHT_UTC, 0).as_deref(), Some("00:00"));
        assert_eq!(at(MIDNIGHT_UTC - 1, 0).as_deref(), Some("THU"));
        // The day is the local one: at UTC-5 h it is still yesterday for the first five hours.
        assert_eq!(at(MIDNIGHT_UTC + 3_600, -18_000).as_deref(), Some("THU"));
        // Before the clock was set there is no time, and with it not set now there is no day to tell.
        assert_eq!(at(90, 0), None);
        assert_eq!(
            when_of(CLOCK_SANE_UNIX + 5, 100, 0)
                .map(When::word)
                .as_deref(),
            Some("00:00")
        );
        let row = alert_row(
            &PanelAlert {
                last_unix: NOW - 2 * 86_400,
                ..alert("threat", None, "ads.example")
            },
            &[],
            Newcomer::AdsPass,
            0,
        );
        assert_eq!(row.at.map(When::word).as_deref(), Some("WED"));
    }

    #[test]
    fn weekdays_moments_and_the_owners_day_words_are_told_in_the_local_zone() {
        // 15 January 2027 was a Friday.
        assert_eq!(weekday_of(MIDNIGHT_UTC, 0), Weekday::Fri);
        assert_eq!(weekday_of(MIDNIGHT_UTC - 1, 0), Weekday::Thu);
        assert_eq!(weekday_of(MIDNIGHT_UTC, -3_600), Weekday::Thu);
        assert_eq!(weekday_of(MIDNIGHT_UTC, 86_399), Weekday::Fri);
        assert_eq!(weekday_of(MIDNIGHT_UTC + 86_400 * 3, 0), Weekday::Mon);
        assert_eq!(weekday_of(0, 0), Weekday::Thu);
        assert_eq!(
            moment_of(NOW, 0).map(Moment::word).as_deref(),
            Some("FRI 10:20")
        );
        assert_eq!(moment_of(90, 0), None, "before the clock was set");
        // The owner's day: `TODAY`, a weekday, none for a push from the desktop or one set before the clock.
        let day = |set: Option<u64>| day_word(set, NOW, 0);
        assert_eq!(day(Some(NOW - 3_600)), Some(DayWord::Today));
        assert_eq!(day(Some(NOW - 86_400)), Some(DayWord::On(Weekday::Thu)));
        assert_eq!(day(Some(NOW - 4 * 86_400)), Some(DayWord::On(Weekday::Mon)));
        assert_eq!(day(None), None);
        assert_eq!(day(Some(90)), None);
    }

    /// What the day's counts of a house look like: three names, most refused first.
    fn top() -> Vec<TopName> {
        [
            ("ads.example.net", 1_204),
            ("tracker.example.com", 612),
            ("telemetry.example.org", 31),
            ("fourth.example.org", 9),
        ]
        .map(|(name, count)| TopName {
            name: name.into(),
            count,
        })
        .to_vec()
    }

    #[test]
    fn the_pipes_are_the_first_three_of_the_day_with_the_valve_open_for_an_owners_allow() {
        let allowed = |name: &str| {
            (name == "tracker.example.com").then_some(OwnerHold {
                side: OwnerSide::Allow,
                exact: true,
                set_unix: None,
            })
        };
        let rows = top_rows(&top(), &allowed);
        let seen: Vec<(&str, u32, bool)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.count, row.open))
            .collect();
        assert_eq!(
            seen,
            [
                ("ads.example.net", 1_204, false),
                ("tracker.example.com", 612, true),
                ("telemetry.example.org", 31, false)
            ]
        );
        assert!(top_rows(&[], &allowed).is_empty());
        // A refusal by the owner is no open valve.
        let refused = |_: &str| {
            Some(OwnerHold {
                side: OwnerSide::Refuse,
                exact: true,
                set_unix: None,
            })
        };
        assert!(top_rows(&top(), &refused).iter().all(|row| !row.open));
    }

    #[test]
    fn your_list_shows_the_newest_three_with_their_side_and_the_day_set_on_the_panel() {
        let entry = |name: &str, side, set_unix| OwnerEntry {
            name: name.into(),
            side,
            set_unix,
        };
        let newest = [
            entry("a.example.org", OwnerSide::Refuse, Some(NOW - 60)),
            entry("b.example.org", OwnerSide::Allow, Some(NOW - 3 * 86_400)),
            entry("c.example.org", OwnerSide::Refuse, None),
            entry("d.example.org", OwnerSide::Allow, None),
        ];
        let rows = owner_rows(&newest, NOW, 0);
        let seen: Vec<(&str, OwnerVerdict, Option<DayWord>)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.verdict, row.day))
            .collect();
        assert_eq!(
            seen,
            [
                ("a.example.org", OwnerVerdict::Refuse, Some(DayWord::Today)),
                (
                    "b.example.org",
                    OwnerVerdict::Allow,
                    Some(DayWord::On(Weekday::Tue))
                ),
                ("c.example.org", OwnerVerdict::Refuse, None)
            ]
        );
    }

    #[test]
    fn the_facts_of_a_name_are_its_list_the_owners_entry_and_the_count_of_a_refused_one() {
        let hold = |side, set_unix| {
            Some(OwnerHold {
                side,
                exact: true,
                set_unix,
            })
        };
        let facts = |name: &str, class, hold| name_facts(name, class, hold, &top(), NOW, 0);
        // A refused name of the day's top: its count.
        let ads = facts("ads.example.net", NameClass::Ads, None);
        assert_eq!(
            (ads.class, ads.owner, ads.count, ads.shut()),
            (WorldNameClass::Ads, None, Some(1_204), true)
        );
        assert_eq!(ads.name, "ads.example.net");
        // A refused name that is not among the eight: refused, but the Pi has no count for it.
        let other = facts("other.example.net", NameClass::Deny, None);
        assert_eq!((other.class, other.count), (WorldNameClass::Deny, None));
        // An answered name has no count: the Pi counts only what it blocks, whatever the top holds of it.
        let answered = facts("ads.example.net", NameClass::None, None);
        assert_eq!((answered.shut(), answered.count), (false, None));
        // The owner's allow opens an ads name, with the day it was set; the day is none for a push.
        let allowed = facts(
            "ads.example.net",
            NameClass::Ads,
            hold(OwnerSide::Allow, Some(NOW - 60)),
        );
        assert_eq!(
            allowed.owner.map(|entry| (entry.verdict, entry.day)),
            Some((OwnerVerdict::Allow, Some(DayWord::Today)))
        );
        assert!(!allowed.shut());
        let pushed = facts(
            "ads.example.net",
            NameClass::Ads,
            hold(OwnerSide::Refuse, None),
        );
        assert_eq!(
            pushed.owner.map(|entry| (entry.verdict, entry.day)),
            Some((OwnerVerdict::Refuse, None))
        );
        // A threat is refused whatever the owner says, and the case of the name does not hide its count.
        let threat = facts(
            "ADS.example.net",
            NameClass::Threat,
            hold(OwnerSide::Allow, None),
        );
        assert_eq!(
            (threat.class, threat.shut(), threat.count),
            (WorldNameClass::Threat, true, Some(1_204))
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

    // -------------------------------------------------------------------------------------------- phase 3

    fn at(hour: u8) -> Option<TimeOfDay> {
        Some(TimeOfDay { hour, minute: 30 })
    }

    #[test]
    fn the_night_hours_run_from_the_first_hour_to_the_second_past_midnight_when_they_must() {
        // 23-07: dark from 23:00 until 07:00.
        let night = Some((23, 7));
        for hour in [23, 0, 3, 6] {
            assert!(in_night(night, at(hour)), "{hour}");
        }
        for hour in [7, 8, 12, 22] {
            assert!(!in_night(night, at(hour)), "{hour}");
        }
        // 10-11: one hour, the first hour in it and the second not.
        assert!(in_night(Some((10, 11)), at(10)));
        assert!(!in_night(Some((10, 11)), at(11)));
        assert!(!in_night(Some((10, 11)), at(9)));
        // No night, the two hours equal, and a clock that is not set are never in the hours.
        assert!(!in_night(None, at(3)));
        assert!(!in_night(Some((5, 5)), at(5)));
        assert!(!in_night(night, None));
    }

    #[test]
    fn the_link_is_the_band_the_signal_the_days_it_held_and_the_upstream_when_it_is_up() {
        let wifi = WifiStatus {
            associated: true,
            signal_dbm: Some(-54),
            ap: None,
            freq_mhz: Some(5_180),
            joined_secs: Some(1_000),
        };
        let now = 1_000 + 12 * 86_400 + 5;
        let link = link_of(Some(&wifi), Some(9), now);
        assert_eq!(link.band, Some(Band::Ghz5));
        assert_eq!(
            (link.signal_dbm, link.linked_days, link.upstream_ms),
            (Some(-54), Some(12), Some(9))
        );
        // The first day is day zero, and 2.4 GHz is its own band; a frequency of neither is none.
        let young = WifiStatus {
            freq_mhz: Some(2_437),
            ..wifi.clone()
        };
        let link = link_of(Some(&young), None, 1_000 + 86_399);
        assert_eq!(
            (link.band, link.linked_days, link.upstream_ms),
            (Some(Band::Ghz24), Some(0), None)
        );
        let odd = WifiStatus {
            freq_mhz: Some(60_000),
            joined_secs: None,
            ..wifi.clone()
        };
        let link = link_of(Some(&odd), None, now);
        assert_eq!((link.band, link.linked_days), (None, None));
        // A link that is not up, or not known, has nothing to say.
        let down = WifiStatus {
            associated: false,
            ..wifi
        };
        assert_eq!(link_of(Some(&down), Some(9), now), Link::default());
        assert_eq!(link_of(None, Some(9), now), Link::default());
    }

    #[test]
    fn a_builds_date_is_its_day_at_utc_across_leap_days_and_the_years() {
        for (unix, date) in [
            (0, "1970-01-01"),
            (951_782_400, "2000-02-29"),
            (1_709_164_800, "2024-02-29"),
            (1_709_164_799, "2024-02-28"),
            (1_767_225_600, "2026-01-01"),
            (1_794_480_000, "2026-11-12"),
            (1_794_441_600, "2026-11-12"),
            (1_794_441_600 + 86_399, "2026-11-12"),
            (1_794_441_600 + 86_400, "2026-11-13"),
            (MIDNIGHT_UTC, "2027-01-15"),
            (4_102_444_799, "2099-12-31"),
        ] {
            assert_eq!(date_of(unix), date, "{unix}");
        }
        // The date the build was made fits the cells VERSION gives it.
        assert!(date_of(crate::BUILD_EPOCH).len() <= qb_panel::os::world::BUILT_CELLS);
    }

    /// A view of the update machinery for a board on `slot`, in `state`.
    fn update_view(slot: Option<Slot>, state: &'static str) -> UpdateView {
        UpdateView {
            slot,
            boot_files: None,
            state,
            received: 0,
            length: 0,
            trial: None,
            last: None,
        }
    }

    /// The trial of slot B that a rollback leaves for slot A.
    fn trial_of(min_left_secs: u64, waiting_for: Vec<Waiting>) -> UpdateView {
        UpdateView {
            trial: Some(crate::update::TrialView {
                build: 1_794_480_000,
                from: Slot::A,
                secs: 3,
                min_left_secs,
                waiting_for,
            }),
            ..update_view(Some(Slot::B), "trial")
        }
    }

    #[test]
    fn an_update_being_received_has_the_serving_slot_on_the_left_and_the_bytes_that_arrived() {
        let received = UpdateView {
            received: 250,
            length: 1_000,
            ..update_view(Some(Slot::B), "receiving")
        };
        let read = update_read(&received).unwrap();
        assert_eq!(
            read.phase,
            UpdatePhase::Receiving {
                received: 250,
                length: 1_000
            }
        );
        assert_eq!((read.before, read.after), (WorldSlot::B, WorldSlot::A));
        assert_eq!(read.phase.received().0, 25);
        // Nothing is under way while the machinery is idle, and the old layout has no slots to show.
        assert_eq!(update_read(&update_view(Some(Slot::A), "idle")), None);
        assert_eq!(update_read(&update_view(None, "receiving")), None);
    }

    #[test]
    fn a_trial_counts_to_its_minimum_time_then_names_what_it_waits_for_in_the_order_of_the_screen()
    {
        let counting = update_read(&trial_of(107, vec![Waiting::Wifi, Waiting::Time])).unwrap();
        assert_eq!(
            counting.phase,
            UpdatePhase::Trial {
                left_secs: 107,
                waiting_for: Some(WorldWaiting::Wifi)
            }
        );
        // The slot a rollback boots is the left box and the one that runs the right.
        assert_eq!(
            (counting.before, counting.after),
            (WorldSlot::A, WorldSlot::B)
        );
        for (waits, word) in [
            (
                vec![Waiting::Upstream, Waiting::Clock],
                Some(WorldWaiting::Clock),
            ),
            (vec![Waiting::Upstream], Some(WorldWaiting::Upstream)),
            (vec![Waiting::Time], None),
            (vec![], None),
        ] {
            let read = update_read(&trial_of(0, waits.clone())).unwrap();
            assert_eq!(
                read.phase,
                UpdatePhase::Trial {
                    left_secs: 0,
                    waiting_for: word
                },
                "{waits:?}"
            );
        }
        // A minimum time that no screen holds is held at its longest.
        let long = update_read(&trial_of(1_000_000, vec![])).unwrap();
        assert!(matches!(
            long.phase,
            UpdatePhase::Trial {
                left_secs: u16::MAX,
                ..
            }
        ));
    }

    #[test]
    fn a_finished_trial_is_shown_with_the_slots_as_they_really_are_and_has_an_identity() {
        use crate::update::{Outcome, OutcomeKind};

        // Confirmed: the new build runs, on B; the box that served before is A.
        let confirmed = UpdateView {
            last: Some(Outcome::confirmed(7)),
            ..update_view(Some(Slot::B), "idle")
        };
        let (read, identity) = finished_update(&confirmed).unwrap();
        assert_eq!(read.phase, UpdatePhase::Confirmed);
        assert_eq!((read.before, read.after), (WorldSlot::A, WorldSlot::B));
        assert_eq!(identity, (OutcomeKind::Confirmed, 7));
        assert_eq!(last_update_of(&confirmed), LastUpdate::Confirmed);
        // Rolled back: the old build runs, on A; the one that failed is B.
        let back = UpdateView {
            last: Some(Outcome::rolled_back(8, "no clock")),
            ..update_view(Some(Slot::A), "idle")
        };
        let (read, identity) = finished_update(&back).unwrap();
        assert_eq!(read.phase, UpdatePhase::RolledBack);
        assert_eq!((read.before, read.after), (WorldSlot::A, WorldSlot::B));
        assert_eq!(identity, (OutcomeKind::RolledBack, 8));
        assert_eq!(last_update_of(&back), LastUpdate::RolledBack);
        // Nothing ended for a board that has had no trial, while one is under way, and on the old layout.
        let none = update_view(Some(Slot::A), "idle");
        assert!(finished_update(&none).is_none());
        assert_eq!(last_update_of(&none), LastUpdate::None);
        let busy = UpdateView {
            last: Some(Outcome::confirmed(7)),
            ..update_view(Some(Slot::A), "receiving")
        };
        assert!(finished_update(&busy).is_none());
        let old = UpdateView {
            last: Some(Outcome::confirmed(7)),
            ..update_view(None, "idle")
        };
        assert!(finished_update(&old).is_none());
    }

    #[test]
    fn the_last_restart_is_only_what_this_boot_can_know_of_it() {
        use crate::update::Outcome;

        assert_eq!(restart_cause(&trial_of(5, vec![])), "an update");
        let confirmed = UpdateView {
            last: Some(Outcome::confirmed(7)),
            ..update_view(Some(Slot::B), "idle")
        };
        assert_eq!(restart_cause(&confirmed), "an update");
        let back = UpdateView {
            last: Some(Outcome::rolled_back(8, "no clock")),
            ..update_view(Some(Slot::A), "idle")
        };
        assert_eq!(restart_cause(&back), "an update that rolled back");
        assert!(restart_cause(&back).chars().count() <= qb_panel::os::world::RESTART_CAUSE_CELLS);
        // The board writes nothing for a restart, so a plain boot cannot say.
        assert_eq!(restart_cause(&update_view(Some(Slot::A), "idle")), "");
    }
}
