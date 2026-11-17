//! The watchdog's endpoints, under `/api/qbp/`: the alerts that the detectors raise, and the
//! devices that the device watch has heard.
//!
//! [`Api::handle`] hands a request here only after the session check, so nothing in this file
//! looks at a session: every endpoint is behind the login, and a request without one is the 401 of
//! any other path. An API that was given no [`QbpWatch`] answers 404 for all of them.
//!
//! The paths are matched in the order the rest of the API uses: an unknown path is 404, a known
//! path with another method is 405, and a query string is 404 on every path but `GET alerts`,
//! which takes `since=` and one to twenty digits and nothing else (and 400 for any other query).
//!
//! | Method and path | Answer |
//! | --- | --- |
//! | `GET alerts[?since=R]` | `{"rev", "suppressed", "alerts": [...]}`: the alerts that changed after revision `R` |
//! | `POST alerts/{id}/ack` | `{"acked": true}`, or 404 for an id that no alert has |
//! | `PUT alerts/acked` | `{"restored": n}` for `{"acked": ["<16 hex>", ...]}` |
//! | `GET devices` | `{"rev", "known_list", "devices": [...], "exempt_macs": [...], "exempt_pushed": [...], "exempt_default", "block_macs": [...], "block_pushed": [...], "own_mac"}` |
//! | `PUT devices/known` | `{"known": n}` for `{"known": ["02:00:00:00:00:01", ...], "append": false, "complete": true}` |
//! | `PUT devices/exempt` | `{"exempt": n, "rev"}` for `{"exempt": ["02:00:00:00:00:01", ...], "if_rev": r}` |
//! | `PUT devices/policy` | `{"exempt_default": b, "block": n, "rev"}` for `{"exempt_default": b, "block": ["02:00:00:00:00:01", ...], "if_rev": r}` |
//! | `PUT devices/labels` | `{"labels": n, "rev"}` for `{"labels": [["02:00:00:00:00:01", "Desk PC"], ...], "append": false, "complete": true}` |
//! | `PUT devices/gate` | `{"gate_left_secs": n, "rev"}` for `{"mac": "02:00:00:00:00:01", "secs": 300}` |
//!
//! The known list may be longer than a body can be (4 KiB, about 230 addresses), so it can be sent in
//! chunks: `append` (default false) says whether the chunk is added to the list that is being sent
//! or replaces it, and `complete` (default true) whether it is the last, which is when the list
//! takes effect, the held alerts are decided and the devices that it leaves out are raised. A long
//! list is `{append: false, complete: false}`, then `{append: true, complete: false}` as often as
//! it takes, then `{append: true, complete: true}`. The answer is the size of the set so far.
//!
//! The exempt list names the devices whose questions are held to the threat lists alone: their ads
//! are let through, and their threats are blocked and alerted on as anyone's. `PUT devices/exempt`
//! replaces the list that the desktop pushes, and leaves the configuration's `watch.exempt` alone;
//! `[]` clears it, and the answer is how many different addresses it holds. It fits one body, at
//! [`MAX_EXEMPT`] addresses, so it is never sent in chunks: more different ones than that are a
//! 400. The list lives in memory only, and the desktop sends it again after the board starts. In
//! `GET devices` `exempt_macs` is both lists together, sorted, in lower case, with the addresses
//! that have not been heard yet.
//!
//! The policy turns the exempt-by-default mode on or off: with it on, every client is held to the
//! threat lists alone but the devices on a block list, the router and the Pi, and the exempt lists
//! have no effect; with it off, the block lists have none. `PUT devices/policy` takes both keys,
//! `exempt_default` and `block`, and replaces the pushed mode, which is in force from then on, and
//! the pushed block list, which sits beside the configuration's `watch.block` as the pushed exempt
//! list sits beside `watch.exempt`; the answer is the mode and how many different addresses the
//! pushed block list holds, at most [`MAX_BLOCK`]. Until a policy is pushed the configuration's
//! `watch.exempt_default` is in force. It lives in memory only, like the exempt list. In
//! `GET devices` `exempt_default` is the mode in force and `block_macs` both block lists together,
//! as `exempt_macs` is; each device has `blocked`, whether it is on a block list, and `exempt`,
//! whether it is exempt in the mode in force: on an exempt list in the list mode, and in the
//! exempt-by-default mode on no block list and claiming neither the router's address nor the Pi's.
//!
//! The inventory has a revision, `rev`, that moves with every change of the pushed exempt list, the
//! pushed block list, the mode, the labels and the gates, whoever makes it: the desktop through
//! these endpoints, the front panel, or a gate that shuts. `PUT devices/exempt` and
//! `devices/policy` take an optional `"if_rev"`: with it, they change nothing and answer a 409
//! `{"error": ..., "rev": current}` unless `if_rev` is the revision now, so that a desktop that
//! read the lists, and was overtaken by the panel meanwhile, reads them again before it pushes.
//! Their answers, and the labels' and the gate's, give the revision after the change. A push that
//! changes nothing does not move it.
//!
//! Each device in `GET devices` also has `label` (what the desktop calls it, or `null`),
//! `gate_left_secs` (the seconds its gate stays open, or `null` while it is shut) and `fixed`
//! (the configuration's list of the mode in force names it, so that no edit of the panel changes
//! it). `exempt_pushed` and `block_pushed` are the lists the desktop pushed, as the panel has
//! edited them, without the configuration's: the ones the desktop adopts when `rev` moved. A device
//! is `exempt` while its gate is open, in either mode.
//!
//! What the desktop needs to draw the panel's wall as the panel does: each device also has
//! `label_rank` (the place of its label in the desktop's list, which is the order the wall shows
//! labelled devices in, or `null` without a label) and `always` (it is the router, or claims the
//! router's or the Pi's address, so it is never exempt and has no gate), and the top level has
//! `own_mac`, the Pi's own hardware address as text, or `null` until it is known: the Pi is no
//! device of the list, and its place on the wall is made from it.
//!
//! `PUT devices/labels` sends the desktop's names for its devices, in its order, which the panel
//! keeps: at most 64 pairs in all, each a MAC and a label of 1 to 24 printable ASCII characters,
//! chunked with `append` and `complete` as the known list is. The labels live in memory only, and
//! the desktop sends them again after the board starts.
//!
//! `PUT devices/gate` opens the gate of one device for `secs` seconds, 1 to 3600, which lets its
//! ads pass in either mode, or shuts it with 0. The device must be one that the inventory lists
//! (404 otherwise). A gate on a device that the configuration's list of the mode in force names, on
//! the router or the Pi, or past 64 open gates, is a 409 with the word `fixed`, `always` or
//! `full`, and a refusal never repeats the address. Gates live in memory only, are not pushed back
//! by the desktop and shut by themselves, also when the board restarts.
//!
//! A body is read whole before anything changes: one bad key or address, or a flag that is not
//! `true` or `false`, is a 400 and nothing is restored or replaced. No answer to a bad body repeats a piece of it. The lock of the device
//! inventory is held for a copy of its view, for `set_known`, `set_exempt` or `set_policy`, and
//! never while
//! JSON is made; the alerts that `set_known` makes are raised into the store after the lock is
//! released.
//!
//! [`Api::handle`]: super::routes::Api::handle

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde_json::{Value, json};

use super::hex;
use super::http::Reply;
use super::routes::{Error, METHOD_NOT_ALLOWED, NO_VALID_JSON, NOT_FOUND, error, ok};
use crate::watch::devices::{
    DeviceView, Devices, EditError, Label, MAX_BLOCK, MAX_EXEMPT, MAX_GATE_SECS, MAX_LABELS,
    with_devices,
};
use crate::watch::frames::Mac;
use crate::watch::store::{AlertStore, AlertView, key_hex};

/// A `since` has at most this many digits: `u64::MAX` has 20.
const SINCE_DIGITS_MAX: usize = 20;

/// A body of `PUT alerts/acked` that is not an object with `acked` a list of keys.
const BAD_ACKED: Error = (
    "bad_request",
    "\"acked\" must be a list of keys of 16 lower-case hex digits",
);
/// A body of `PUT devices/known` that is not an object with `known` a list of addresses.
const BAD_KNOWN: Error = (
    "bad_request",
    "\"known\" must be a list of Ethernet addresses",
);
/// A body of `PUT devices/exempt` that is not an object with `exempt` a list of addresses, or
/// whose list has more than [`MAX_EXEMPT`] different ones.
const BAD_EXEMPT: Error = (
    "bad_request",
    "\"exempt\" must be a list of at most 64 different Ethernet addresses",
);
/// A body of `PUT devices/policy` that is not an object with `exempt_default` true or false and
/// `block` a list of addresses, or whose list has more than [`MAX_BLOCK`] different ones.
const BAD_POLICY: Error = (
    "bad_request",
    "\"exempt_default\" must be true or false and \"block\" a list of at most 64 different Ethernet addresses",
);
/// A body of `PUT devices/labels` that is not an object with `labels` a list of at most 64 pairs
/// of an Ethernet address and a label of 1 to 24 printable ASCII characters.
const BAD_LABELS: Error = (
    "bad_request",
    "\"labels\" must be a list of at most 64 pairs of an Ethernet address and a label of 1 to 24 printable ASCII characters",
);
/// A body of `PUT devices/gate` that is not an object with `mac` an Ethernet address of a device
/// and `secs` a whole number from 0 to 3600.
const BAD_GATE: Error = (
    "bad_request",
    "\"mac\" must be an Ethernet address and \"secs\" a whole number of seconds from 0 to 3600",
);
/// A body whose `if_rev` is there and is not a whole number.
const BAD_IF_REV: Error = ("bad_request", "\"if_rev\" must be a whole revision number");
/// A push made on a revision that the inventory has left behind.
const STALE: Error = (
    "stale",
    "The lists changed since this revision; read them again",
);
/// A gate or an edit that the inventory refuses, by [`EditError`]: the word, and what it means.
const REFUSED_FIXED: Error = ("fixed", "The configuration decides this device");
const REFUSED_ALWAYS: Error = ("always", "The router and the Pi are never exempt");
const REFUSED_FULL: Error = ("full", "No room for another");
/// A body of `PUT devices/known` whose `append` or `complete` is there and is not true or false.
const BAD_FLAG: Error = (
    "bad_request",
    "\"append\" and \"complete\" must be true or false",
);
/// A query on `GET alerts` that is not `since=` and one to twenty digits that fit a `u64`.
const BAD_SINCE: Error = (
    "bad_request",
    "since must be a whole revision number, as the last answer gave it",
);

/// What the endpoints under `/api/qbp/` answer from: the store that the alerts are in and the
/// inventory that the device watch fills.
///
/// It is named for the API's side, `Qbp`, as `crate::watch::Watch`, the hook on the answer path
/// that the resolver is given, is the other thing a watch is called.
pub struct QbpWatch {
    /// The alerts, which the answer path, the analysis and the device watch raise into.
    pub alerts: Arc<AlertStore>,
    /// The devices that the device watch has heard, with the known list.
    pub devices: Arc<Mutex<Devices>>,
}

/// The endpoints, by the path after `/api/qbp/`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    /// `alerts`.
    Alerts,
    /// `alerts/{id}/ack`, with the id.
    Ack(u64),
    /// `alerts/acked`.
    Restore,
    /// `devices`.
    Devices,
    /// `devices/known`.
    Known,
    /// `devices/exempt`.
    Exempt,
    /// `devices/policy`.
    Policy,
    /// `devices/labels`.
    Labels,
    /// `devices/gate`.
    Gate,
}

impl Route {
    /// The endpoint at `rest`, the path after `/api/qbp/`. An id that is not digits only, or too
    /// large for a `u64`, is no endpoint: no alert has such an id, which is a 404 either way.
    fn at(rest: &str) -> Option<Route> {
        Some(match rest {
            "alerts" => Route::Alerts,
            "alerts/acked" => Route::Restore,
            "devices" => Route::Devices,
            "devices/known" => Route::Known,
            "devices/exempt" => Route::Exempt,
            "devices/policy" => Route::Policy,
            "devices/labels" => Route::Labels,
            "devices/gate" => Route::Gate,
            _ => {
                let id = rest.strip_prefix("alerts/")?.strip_suffix("/ack")?;
                if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                Route::Ack(id.parse().ok()?)
            }
        })
    }

    /// The one method that the endpoint answers.
    fn method(self) -> &'static str {
        match self {
            Route::Alerts | Route::Devices => "GET",
            Route::Ack(_) => "POST",
            Route::Restore
            | Route::Known
            | Route::Exempt
            | Route::Policy
            | Route::Labels
            | Route::Gate => "PUT",
        }
    }
}

/// Answers one request under `/api/qbp/`: `rest` is the path after that prefix and `query` is
/// what followed a `?` in the target, if anything did. `started` is when the API began the
/// request, for the `took` of the answer.
pub(super) fn handle(
    watch: &QbpWatch,
    method: &str,
    rest: &str,
    query: Option<&str>,
    body: &[u8],
    started: Instant,
) -> Reply {
    let Some(route) = Route::at(rest) else {
        return error(404, NOT_FOUND, started);
    };
    if method != route.method() {
        return error(405, METHOD_NOT_ALLOWED, started);
    }
    if query.is_some() && route != Route::Alerts {
        return error(404, NOT_FOUND, started);
    }
    match route {
        Route::Alerts => alerts(&watch.alerts, query, started),
        Route::Ack(id) => ack(&watch.alerts, id, started),
        Route::Restore => restore(&watch.alerts, body, started),
        Route::Devices => devices(&watch.devices, started),
        Route::Known => known(watch, body, started),
        Route::Exempt => exempt(watch, body, started),
        Route::Policy => policy(watch, body, started),
        Route::Labels => labels(watch, body, started),
        Route::Gate => gate(watch, body, started),
    }
}

/// `GET alerts[?since=R]`: the revision now, how many alerts a rate limit dropped, and the alerts
/// that changed after `R`, the oldest change first.
fn alerts(store: &AlertStore, query: Option<&str>, started: Instant) -> Reply {
    let Some(since) = since_wanted(query) else {
        return error(400, BAD_SINCE, started);
    };
    let list = store.list(since);
    let alerts: Vec<Value> = list.alerts.iter().map(alert_json).collect();
    ok(
        json!({ "rev": list.rev, "suppressed": list.suppressed, "alerts": alerts }),
        started,
    )
}

/// The revision a `GET alerts` asks after: 0 with no query, and for exactly `since=` and one to
/// [`SINCE_DIGITS_MAX`] ASCII digits that fit a `u64`, that number. `None`, a 400, for anything
/// else.
fn since_wanted(query: Option<&str>) -> Option<u64> {
    let Some(query) = query else {
        return Some(0);
    };
    let digits = query.strip_prefix("since=")?;
    if digits.is_empty()
        || digits.len() > SINCE_DIGITS_MAX
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// One alert as JSON: its key is the 16 hex digits that `PUT alerts/acked` takes back.
fn alert_json(alert: &AlertView) -> Value {
    json!({
        "id": alert.id,
        "key": key_hex(alert.key),
        "rev": alert.rev,
        "kind": alert.kind,
        "severity": alert.severity,
        "client": alert.client.map(|client| client.to_string()),
        "subject": alert.subject,
        "first_unix": alert.first_unix,
        "last_unix": alert.last_unix,
        "count": alert.count,
        "acked": alert.acked,
    })
}

/// `POST alerts/{id}/ack`: acknowledges the alert, which is 404 when there is none with `id`.
fn ack(store: &AlertStore, id: u64, started: Instant) -> Reply {
    if store.ack(id) {
        ok(json!({ "acked": true }), started)
    } else {
        error(404, NOT_FOUND, started)
    }
}

/// `PUT alerts/acked`: adds the keys of the body to the store's acknowledged ones, and answers how
/// many different keys it kept.
fn restore(store: &AlertStore, body: &[u8], started: Instant) -> Reply {
    let texts = match strings(body, "acked", BAD_ACKED) {
        Ok(texts) => texts,
        Err(refusal) => return error(400, refusal, started),
    };
    let mut keys = Vec::with_capacity(texts.len());
    for text in &texts {
        let Some(bytes) = hex::decode::<8>(text) else {
            return error(400, BAD_ACKED, started);
        };
        keys.push(u64::from_be_bytes(bytes));
    }
    ok(json!({ "restored": store.restore_acked(&keys) }), started)
}

/// `GET devices`: the revision, whether a known list has come, every device, the one seen last
/// first, the MACs of both exempt lists and of the pushed one, the mode in force, the MACs of
/// both block lists and of the pushed one, and the Pi's own hardware address (`own_mac`, or `null`).
///
/// The inventory is locked for the copy of its view, and the JSON is made after.
fn devices(devices: &Mutex<Devices>, started: Instant) -> Reply {
    let view = lock(devices).view();
    let list: Vec<Value> = view.devices.iter().map(device_json).collect();
    let exempt: Vec<String> = view.exempt_macs.iter().map(ToString::to_string).collect();
    let block: Vec<String> = view.block_macs.iter().map(ToString::to_string).collect();
    let texts = |macs: &[Mac]| -> Vec<String> { macs.iter().map(ToString::to_string).collect() };
    ok(
        json!({
            "rev": view.rev,
            "known_list": view.known_list.as_str(),
            "devices": list,
            "exempt_macs": exempt,
            "exempt_pushed": texts(&view.exempt_pushed),
            "exempt_default": view.exempt_default,
            "block_macs": block,
            "block_pushed": texts(&view.block_pushed),
            "own_mac": view.own_mac.map(|mac| mac.to_string()),
        }),
        started,
    )
}

/// One device as JSON.
fn device_json(device: &DeviceView) -> Value {
    let ipv4: Vec<String> = device.ipv4.iter().map(ToString::to_string).collect();
    json!({
        "mac": device.mac.to_string(),
        "ipv4": ipv4,
        "hostname": device.hostname,
        "first_unix": device.first_unix,
        "last_unix": device.last_unix,
        "known": device.known,
        "exempt": device.exempt,
        "blocked": device.blocked,
        "label": device.label.as_deref(),
        "gate_left_secs": device.gate_left_secs,
        "fixed": device.fixed,
        "label_rank": device.label_rank,
        "always": device.always,
    })
}

/// `PUT devices/known`: takes a chunk of the known list, the addresses of the body, as
/// [`Devices::load_known`] says with the flags `append` (default false) and `complete` (default
/// true), and answers how many different addresses the set holds now. The addresses and the flags
/// are all read before the inventory is touched, and the alerts that a complete list makes are
/// raised after its lock is released.
fn known(watch: &QbpWatch, body: &[u8], started: Instant) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return error(400, NO_VALID_JSON, started),
    };
    let texts = match string_list(&value, "known", BAD_KNOWN) {
        Ok(texts) => texts,
        Err(refusal) => return error(400, refusal, started),
    };
    let (Some(append), Some(complete)) = (
        flag(&value, "append", false),
        flag(&value, "complete", true),
    ) else {
        return error(400, BAD_FLAG, started);
    };
    let mut macs = Vec::with_capacity(texts.len());
    for text in &texts {
        let Ok(mac) = text.parse::<Mac>() else {
            return error(400, BAD_KNOWN, started);
        };
        macs.push(mac);
    }
    let kept = with_devices(&watch.devices, &*watch.alerts, |devices| {
        devices.load_known(macs, append, complete)
    });
    ok(json!({ "known": kept }), started)
}

/// `PUT devices/exempt`: replaces the pushed exempt list with the addresses of the body, each once,
/// and answers how many different ones it holds now and the revision after. They are all read
/// before the inventory is touched: one that is not an Ethernet address, or more than
/// [`MAX_EXEMPT`] different ones, is a 400 and the list stays as it was. With `if_rev` the list is
/// replaced only if that is the revision now; if it is not, nothing changes and the answer is a 409
/// with the revision that is.
fn exempt(watch: &QbpWatch, body: &[u8], started: Instant) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return error(400, NO_VALID_JSON, started),
    };
    let texts = match string_list(&value, "exempt", BAD_EXEMPT) {
        Ok(texts) => texts,
        Err(refusal) => return error(400, refusal, started),
    };
    let mut macs = BTreeSet::new();
    for text in &texts {
        let Ok(mac) = text.parse::<Mac>() else {
            return error(400, BAD_EXEMPT, started);
        };
        macs.insert(mac);
        if macs.len() > MAX_EXEMPT {
            return error(400, BAD_EXEMPT, started);
        }
    }
    let Some(if_rev) = if_rev(&value) else {
        return error(400, BAD_IF_REV, started);
    };
    let pushed = with_devices(&watch.devices, &*watch.alerts, |devices| {
        devices.check_rev(if_rev)?;
        let kept = devices.set_exempt(macs.into_iter().collect());
        Ok((kept, devices.rev()))
    });
    match pushed {
        Ok((kept, rev)) => ok(json!({ "exempt": kept, "rev": rev }), started),
        Err(current) => stale(current, started),
    }
}

/// `PUT devices/policy`: replaces the pushed mode with `exempt_default` and the pushed block list
/// with the addresses of `block`, each once, and answers the mode, how many different addresses
/// the list holds now and the revision after. Both keys are required, and all is read before the
/// inventory is touched: a mode that is not true or false, an address that is not an Ethernet
/// address, or more than [`MAX_BLOCK`] different ones, is a 400 and the policy stays as it was.
/// `if_rev` is as for [`exempt`].
fn policy(watch: &QbpWatch, body: &[u8], started: Instant) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return error(400, NO_VALID_JSON, started),
    };
    let Some(exempt_default) = value.get("exempt_default").and_then(Value::as_bool) else {
        return error(400, BAD_POLICY, started);
    };
    let texts = match string_list(&value, "block", BAD_POLICY) {
        Ok(texts) => texts,
        Err(refusal) => return error(400, refusal, started),
    };
    let mut macs = BTreeSet::new();
    for text in &texts {
        let Ok(mac) = text.parse::<Mac>() else {
            return error(400, BAD_POLICY, started);
        };
        macs.insert(mac);
        if macs.len() > MAX_BLOCK {
            return error(400, BAD_POLICY, started);
        }
    }
    let Some(if_rev) = if_rev(&value) else {
        return error(400, BAD_IF_REV, started);
    };
    let pushed = with_devices(&watch.devices, &*watch.alerts, |devices| {
        devices.check_rev(if_rev)?;
        let kept = devices.set_policy(exempt_default, macs.into_iter().collect());
        Ok((kept, devices.rev()))
    });
    match pushed {
        Ok((kept, rev)) => ok(
            json!({ "exempt_default": exempt_default, "block": kept, "rev": rev }),
            started,
        ),
        Err(current) => stale(current, started),
    }
}

/// `PUT devices/labels`: takes a chunk of the desktop's labels, as [`Devices::load_labels`] says
/// with the flags `append` (default false) and `complete` (default true), and answers how many
/// labels the set holds now and the revision after. The pairs and the flags are all read before
/// the inventory is touched: more than [`MAX_LABELS`] pairs, an address that is no device's, a
/// label that is not 1 to 24 printable ASCII characters, or a pair that is not two strings is a
/// 400, and no answer repeats a piece of the body.
fn labels(watch: &QbpWatch, body: &[u8], started: Instant) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return error(400, NO_VALID_JSON, started),
    };
    let Some(Value::Array(items)) = value.get("labels") else {
        return error(400, BAD_LABELS, started);
    };
    if items.len() > MAX_LABELS {
        return error(400, BAD_LABELS, started);
    }
    let (Some(append), Some(complete)) = (
        flag(&value, "append", false),
        flag(&value, "complete", true),
    ) else {
        return error(400, BAD_FLAG, started);
    };
    let mut pairs = Vec::with_capacity(items.len());
    for item in items {
        let Value::Array(pair) = item else {
            return error(400, BAD_LABELS, started);
        };
        let [Value::String(mac), Value::String(label)] = pair.as_slice() else {
            return error(400, BAD_LABELS, started);
        };
        let (Ok(mac), Some(label)) = (mac.parse::<Mac>(), Label::new(label)) else {
            return error(400, BAD_LABELS, started);
        };
        if !mac.can_be_a_device() {
            return error(400, BAD_LABELS, started);
        }
        pairs.push((mac, label));
    }
    let (kept, rev) = with_devices(&watch.devices, &*watch.alerts, |devices| {
        (devices.load_labels(pairs, append, complete), devices.rev())
    });
    ok(json!({ "labels": kept, "rev": rev }), started)
}

/// `PUT devices/gate`: opens the gate of the device `mac` for `secs` seconds, or shuts it with 0,
/// and answers the seconds and the revision after. A body that is not a MAC of a device and a whole
/// number of seconds from 0 to 3600 is a 400, a device that the inventory does not list a 404, and
/// an edit that the inventory refuses (the configuration decides the device, it is the router or
/// the Pi, or the gates are full) a 409 with a word for it and without the address.
fn gate(watch: &QbpWatch, body: &[u8], started: Instant) -> Reply {
    let value = match serde_json::from_slice::<Value>(body) {
        Ok(value) => value,
        Err(_) => return error(400, NO_VALID_JSON, started),
    };
    let mac = value
        .get("mac")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Mac>().ok())
        .filter(|mac| mac.can_be_a_device());
    let secs = value
        .get("secs")
        .and_then(Value::as_u64)
        .and_then(|secs| u16::try_from(secs).ok())
        .filter(|&secs| secs <= MAX_GATE_SECS);
    let (Some(mac), Some(secs)) = (mac, secs) else {
        return error(400, BAD_GATE, started);
    };
    let done = with_devices(&watch.devices, &*watch.alerts, |devices| {
        devices.open_gate(mac, secs).map(|()| devices.rev())
    });
    match done {
        Ok(rev) => ok(json!({ "gate_left_secs": secs, "rev": rev }), started),
        Err(EditError::Unknown) => error(404, NOT_FOUND, started),
        Err(EditError::Fixed) => error(409, REFUSED_FIXED, started),
        Err(EditError::Always) => error(409, REFUSED_ALWAYS, started),
        Err(EditError::Full) => error(409, REFUSED_FULL, started),
        Err(EditError::Invalid) => error(400, BAD_GATE, started),
    }
}

/// The 409 for a push made on a revision that is not the one now: Pi-hole's error object, and the
/// revision that is, which is what the desktop reads the lists again on.
fn stale(current: u64, started: Instant) -> Reply {
    let (key, message) = STALE;
    let body = json!({
        "error": { "key": key, "message": message, "hint": null },
        "rev": current,
        "took": started.elapsed().as_secs_f64(),
    });
    Reply {
        status: 409,
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

/// The `if_rev` of a push: `Some(None)` when the body has none, the revision when it has one, and
/// `None` when it has one that is not a whole number.
fn if_rev(body: &Value) -> Option<Option<u64>> {
    match body.get("if_rev") {
        None => Some(None),
        Some(value) => value.as_u64().map(Some),
    }
}

/// The boolean field `field` of the JSON object `body`: `default` when it is not there, and `None`
/// when it is there and is not `true` or `false`.
fn flag(body: &Value, field: &str, default: bool) -> Option<bool> {
    match body.get(field) {
        None => Some(default),
        Some(value) => value.as_bool(),
    }
}

/// The texts in the list that the field `field` of the JSON object `body` holds. A body that is not
/// JSON is [`NO_VALID_JSON`], and an object without the field, a field that is no list, or a list
/// with an item that is no string is `wrong_shape`. Other fields are ignored.
fn strings(body: &[u8], field: &str, wrong_shape: Error) -> Result<Vec<String>, Error> {
    let Ok(body) = serde_json::from_slice::<Value>(body) else {
        return Err(NO_VALID_JSON);
    };
    string_list(&body, field, wrong_shape)
}

/// The texts in the list that the field `field` of the parsed JSON `body` holds, as [`strings`]
/// reads them.
fn string_list(body: &Value, field: &str, wrong_shape: Error) -> Result<Vec<String>, Error> {
    let Some(Value::Array(items)) = body.get(field) else {
        return Err(wrong_shape);
    };
    items
        .iter()
        .map(|item| item.as_str().map(str::to_owned).ok_or(wrong_shape))
        .collect()
}

/// The inventory, locked. A lock that a panic poisoned still guards a whole inventory, which keeps
/// its bounds whatever a call left half done, so it is taken as it is. It is never held across an
/// `.await`: nothing here waits.
fn lock(devices: &Mutex<Devices>) -> MutexGuard<'_, Devices> {
    devices.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use qb_dns::{Counts, HistoryBucket, LogEntry};

    use super::*;
    use crate::alerts::{Alert, AlertSink, Severity};
    use crate::api::http::Request;
    use crate::api::password::PasswordHash;
    use crate::api::routes::{Api, Facts};
    use crate::api::seams::{Client, ServerStats};
    use crate::watch::frames::{Arp, DHCP_REQUEST, Dhcp};
    use crate::watch::store::key_of;

    /// The address that the tests log in from.
    const ME: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 7);
    /// The Unix time that the clock of the store and of the inventory reads.
    const UNIX: u64 = 1_800_000_000;

    /// A `Facts` with nothing to tell: these tests ask for nothing but the endpoints under
    /// `/api/qbp/`.
    struct NoFacts;

    impl Facts for NoFacts {
        fn last_24h(&self) -> Counts {
            Counts::default()
        }
        fn history(&self) -> Vec<HistoryBucket> {
            Vec::new()
        }
        fn upstreams(&self) -> Vec<ServerStats> {
            Vec::new()
        }
        fn clients(&self) -> (usize, usize) {
            (0, 0)
        }
        fn client_list(&self) -> Vec<Client> {
            Vec::new()
        }
        fn recent(&self, _n: usize) -> Vec<LogEntry> {
            Vec::new()
        }
        fn list(&self) -> (Option<usize>, Option<u64>) {
            (None, None)
        }
        fn paused_left_secs(&self) -> Option<u64> {
            None
        }
        fn pause_for(&self, _secs: u64) {}
        fn resume(&self) {}
    }

    /// The clock of the store and the inventory: (monotonic seconds, unix seconds), both still.
    fn clock() -> Arc<dyn Fn() -> (u64, u64) + Send + Sync> {
        Arc::new(|| (100, UNIX))
    }

    /// An API with a watch over an empty store and an empty inventory, and a session on it.
    struct Setup {
        api: Api,
        watch: Arc<QbpWatch>,
        sid: String,
    }

    impl Setup {
        fn new() -> Self {
            let store = Arc::new(AlertStore::new(clock()));
            let devices = Devices::new(None, Ipv4Addr::new(192, 0, 2, 1), clock());
            let watch = Arc::new(QbpWatch {
                alerts: store,
                devices: Arc::new(Mutex::new(devices)),
            });
            let api = plain_api().with_watch(watch.clone());
            let sid = login(&api);
            Setup { api, watch, sid }
        }

        /// What the API answers to `method` `target` with `body`, with the session.
        fn send(&self, method: &str, target: &str, body: &str) -> (u16, Value) {
            ask(&self.api, method, target, Some(&self.sid), body.as_bytes())
        }

        /// Raises an alert of `kind` and severity high about `client` and `subject`.
        fn raise(&self, kind: &'static str, client: Option<IpAddr>, subject: &str) {
            self.watch.alerts.raise(Alert {
                kind,
                severity: Severity::High,
                client,
                subject: subject.to_owned(),
            });
        }

        /// Lets the inventory hear a DHCP request of `mac`, which gives `host` as its name.
        fn hear(&self, mac: Mac, host: &str) {
            with_devices(&self.watch.devices, &*self.watch.alerts, |devices| {
                devices.on_dhcp(&Dhcp {
                    msg_type: Some(DHCP_REQUEST),
                    chaddr: mac,
                    ciaddr: Ipv4Addr::UNSPECIFIED,
                    requested_ip: Some(Ipv4Addr::new(192, 0, 2, 40)),
                    hostname: Some(host.to_owned()),
                });
            });
        }

        /// The alerts that the store holds, as the endpoint lists them.
        fn listed(&self) -> Vec<Value> {
            let (status, body) = self.send("GET", "/api/qbp/alerts", "");
            assert_eq!(status, 200);
            body["alerts"].as_array().unwrap().clone()
        }
    }

    /// An API with no watch, and the password `right`.
    fn plain_api() -> Api {
        Api::new(
            PasswordHash::new(b"right", [0; 16]),
            Arc::new(NoFacts),
            Arc::new(|| 1_000),
            Arc::new(|| Some([7; 16])),
        )
    }

    /// Logs in from `ME` and returns the sid.
    fn login(api: &Api) -> String {
        let (status, body) = ask(api, "POST", "/api/auth", None, br#"{"password":"right"}"#);
        assert_eq!(status, 200);
        body["session"]["sid"].as_str().unwrap().to_owned()
    }

    /// The status and the JSON body of the API's answer to a request.
    fn ask(api: &Api, method: &str, target: &str, sid: Option<&str>, body: &[u8]) -> (u16, Value) {
        let reply = api.handle(
            &Request {
                method: method.into(),
                target: target.into(),
                sid: sid.map(Into::into),
                body: body.to_vec(),
                close: false,
            },
            ME,
        );
        (
            reply.status,
            serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
        )
    }

    /// A client of the documentation range.
    fn client(host: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, host))
    }

    /// The address `02:00:00:00:00:nn`.
    fn mac(n: u8) -> Mac {
        Mac([2, 0, 0, 0, 0, n])
    }

    /// The error key of an answer that is Pi-hole's error object.
    fn error_key(body: &Value) -> &str {
        body["error"]["key"].as_str().unwrap()
    }

    #[test]
    fn the_alerts_are_listed_oldest_change_first_with_their_key_and_client() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        setup.raise("new_device", None, "02:00:00:00:00:09");

        let (status, body) = setup.send("GET", "/api/qbp/alerts", "");

        assert_eq!(status, 200);
        assert_eq!((&body["rev"], &body["suppressed"]), (&json!(2), &json!(0)));
        assert!(body["took"].is_number());
        let key = key_hex(key_of("dga", Some(client(7)), "random NXDOMAIN names"));
        assert_eq!(
            body["alerts"],
            json!([
                {
                    "id": 1, "key": key, "rev": 1, "kind": "dga", "severity": "high",
                    "client": "192.0.2.7", "subject": "random NXDOMAIN names",
                    "first_unix": UNIX, "last_unix": UNIX, "count": 1, "acked": false,
                },
                {
                    "id": 2, "key": key_hex(key_of("new_device", None, "02:00:00:00:00:09")),
                    "rev": 2, "kind": "new_device", "severity": "high", "client": null,
                    "subject": "02:00:00:00:00:09", "first_unix": UNIX, "last_unix": UNIX,
                    "count": 1, "acked": false,
                },
            ])
        );
        assert_eq!(key.len(), 16);
    }

    #[test]
    fn since_lists_what_changed_after_a_revision_and_a_repeat_counts_as_a_change() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        setup.raise("burst", Some(client(8)), "question rate");

        let after = |since: &str| {
            let (status, body) = setup.send("GET", &format!("/api/qbp/alerts?since={since}"), "");
            assert_eq!(status, 200, "since={since}");
            let kinds: Vec<String> = body["alerts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["kind"].as_str().unwrap().to_owned())
                .collect();
            (body["rev"].as_u64().unwrap(), kinds)
        };
        assert_eq!(after("0"), (2, vec!["dga".to_owned(), "burst".to_owned()]));
        assert_eq!(after("1"), (2, vec!["burst".to_owned()]));
        assert_eq!(after("2"), (2, Vec::new()));
        assert_eq!(after("18446744073709551615"), (2, Vec::new()));
        // The first alert raised again merges and moves to the end.
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        assert_eq!(after("2"), (3, vec!["dga".to_owned()]));
        assert_eq!(setup.listed()[1]["count"], json!(2));
    }

    #[test]
    fn a_since_that_is_not_one_to_twenty_digits_of_a_u64_is_a_400_and_nothing_else_is() {
        let setup = Setup::new();
        for query in [
            "since=",
            "since=-1",
            "since=1.5",
            "since=abc",
            "since=1 ",
            "since=+1",
            "since=١٢",
            "since=99999999999999999999",
            "since=000000000000000000001",
            "since=1&since=2",
            "since=1&x=2",
            "Since=1",
            "x=1",
            "",
        ] {
            let (status, body) = setup.send("GET", &format!("/api/qbp/alerts?{query}"), "");
            assert_eq!(status, 400, "?{query}");
            assert_eq!(error_key(&body), "bad_request", "?{query}");
        }
        // Leading zeros are digits, if they are not too many.
        let (status, _) = setup.send("GET", "/api/qbp/alerts?since=00000000000000000001", "");
        assert_eq!(status, 200);
    }

    #[test]
    fn an_ack_marks_the_alert_and_bumps_its_revision_and_an_unknown_id_is_a_404() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        setup.raise("burst", Some(client(8)), "question rate");

        let (status, body) = setup.send("POST", "/api/qbp/alerts/2/ack", "");

        assert_eq!((status, &body["acked"]), (200, &json!(true)));
        assert!(body["took"].is_number());
        let listed = setup.listed();
        assert_eq!(
            (listed[0]["acked"].clone(), listed[1]["acked"].clone()),
            (json!(false), json!(true))
        );
        assert_eq!(listed[1]["rev"], json!(3));
        // An alert that is acknowledged already is still there to acknowledge.
        assert_eq!(setup.send("POST", "/api/qbp/alerts/2/ack", "").0, 200);
        assert_eq!(setup.listed()[1]["rev"], json!(3));
        // No alert has the id 3, and an id that is not plain digits is no endpoint at all.
        for id in [
            "3",
            "0",
            "99999999999999999999999",
            "",
            "-1",
            "+1",
            "x",
            "1.0",
            "0x1",
            "acked",
            "٢",
        ] {
            let (status, body) = setup.send("POST", &format!("/api/qbp/alerts/{id}/ack"), "");
            assert_eq!(status, 404, "id {id:?}");
            assert_eq!(error_key(&body), "not_found", "id {id:?}");
        }
        assert_eq!(setup.listed()[0]["acked"], json!(false));
    }

    #[test]
    fn restored_keys_acknowledge_the_alerts_there_and_the_ones_raised_later() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        let there = key_hex(key_of("dga", Some(client(7)), "random NXDOMAIN names"));
        let later = key_hex(key_of("burst", Some(client(8)), "question rate"));
        let body = format!(r#"{{"acked": ["{there}", "{later}", "{there}"], "other": 1}}"#);

        let (status, answer) = setup.send("PUT", "/api/qbp/alerts/acked", &body);

        assert_eq!((status, &answer["restored"]), (200, &json!(2)));
        assert!(answer["took"].is_number());
        assert_eq!(setup.listed()[0]["acked"], json!(true));
        setup.raise("burst", Some(client(8)), "question rate");
        assert_eq!(setup.listed()[1]["acked"], json!(true));
        // An empty list restores nothing, and is fine.
        let (status, answer) = setup.send("PUT", "/api/qbp/alerts/acked", r#"{"acked": []}"#);
        assert_eq!((status, &answer["restored"]), (200, &json!(0)));
    }

    #[test]
    fn a_bad_body_of_the_acked_keys_restores_nothing() {
        let setup = Setup::new();
        let good = key_hex(key_of("dga", Some(client(7)), "random NXDOMAIN names"));
        let mixed = format!(r#"{{"acked": ["{good}", "not a key"]}}"#);
        let bad: [(&str, &str); 14] = [
            ("", "bad_request"),
            ("acked", "bad_request"),
            (r#"{"acked": ["#, "bad_request"),
            ("[]", "bad_request"),
            ("{}", "bad_request"),
            (r#"{"acked": null}"#, "bad_request"),
            (r#"{"acked": "0123456789abcdef"}"#, "bad_request"),
            (r#"{"acked": [1]}"#, "bad_request"),
            (r#"{"acked": ["0123456789abcdef", null]}"#, "bad_request"),
            (r#"{"acked": ["0123456789ABCDEF"]}"#, "bad_request"),
            (r#"{"acked": ["0123456789abcde"]}"#, "bad_request"),
            (r#"{"acked": ["0123456789abcdef0"]}"#, "bad_request"),
            (r#"{"acked": ["0123456789abcdeg"]}"#, "bad_request"),
            (&mixed, "bad_request"),
        ];
        for (body, key) in bad {
            let (status, answer) = setup.send("PUT", "/api/qbp/alerts/acked", body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), key, "{body}");
            // The refusal does not say what was wrong with the key.
            assert!(!answer.to_string().contains("not a key"), "{body}");
        }
        // The good key of the mixed body was not kept: its alert starts unacknowledged.
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        assert_eq!(setup.listed()[0]["acked"], json!(false));
    }

    #[test]
    fn the_devices_come_with_their_names_and_whether_a_known_list_has_come() {
        let setup = Setup::new();
        let (status, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(status, 200);
        assert_eq!(
            (&body["known_list"], &body["devices"], &body["exempt_macs"]),
            (&json!("waiting"), &json!([]), &json!([]))
        );
        assert_eq!(
            (&body["exempt_default"], &body["block_macs"]),
            (&json!(false), &json!([]))
        );
        assert_eq!(
            (&body["rev"], &body["exempt_pushed"], &body["block_pushed"]),
            (&json!(0), &json!([]), &json!([]))
        );
        assert!(body["took"].is_number());
        // The Pi's own address is not known yet: the field is there, and says so.
        assert_eq!(body["own_mac"], json!(null));
        assert!(body.as_object().unwrap().contains_key("own_mac"));

        setup.hear(mac(5), "laptop");
        let (_, body) = setup.send("GET", "/api/qbp/devices", "");

        assert_eq!(
            body["devices"],
            json!([{
                "mac": "02:00:00:00:00:05", "ipv4": ["192.0.2.40"], "hostname": "laptop",
                "first_unix": UNIX, "last_unix": UNIX, "known": false, "exempt": false,
                "blocked": false, "label": null, "gate_left_secs": null, "fixed": false,
                "label_rank": null, "always": false,
            }])
        );
    }

    #[test]
    fn the_devices_tell_the_pis_own_address_and_each_ones_label_place_and_whether_it_is_always() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        // The router, which asks by ARP for its own address and so is the one that claims it first; and
        // the Pi's own address, once it is known.
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.on_arp(&Arp {
                op: 1,
                sender_mac: mac(0xa1),
                sender_ip: Ipv4Addr::new(192, 0, 2, 1),
                target_ip: Ipv4Addr::new(192, 0, 2, 1),
            });
            devices.set_own_mac(mac(0xee));
        });
        // The desktop's order: the phone first, then the laptop; the router has no label.
        let (status, _) = put_labels(
            &setup,
            r#"{"labels": [["02:00:00:00:00:06", "Phone"], ["02:00:00:00:00:05", "Desk PC"]]}"#,
        );
        assert_eq!(status, 200);

        let body = get_devices(&setup);
        assert_eq!(body["own_mac"], json!("02:00:00:00:00:ee"));
        let of = |n: u8| -> (Value, Value, Value) {
            let device = body["devices"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["mac"] == json!(mac_text(n)))
                .unwrap_or_else(|| panic!("device {n}"));
            (
                device["label"].clone(),
                device["label_rank"].clone(),
                device["always"].clone(),
            )
        };
        assert_eq!(of(6), (json!("Phone"), json!(0), json!(false)));
        assert_eq!(of(5), (json!("Desk PC"), json!(1), json!(false)));
        assert_eq!(of(0xa1), (json!(null), json!(null), json!(true)));
        // Every field it had is still there, with the same names.
        let phone = body["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["mac"] == json!(mac_text(6)))
            .unwrap();
        let mut keys: Vec<&str> = phone
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "always",
                "blocked",
                "exempt",
                "first_unix",
                "fixed",
                "gate_left_secs",
                "hostname",
                "ipv4",
                "known",
                "label",
                "label_rank",
                "last_unix",
                "mac"
            ]
        );
    }

    /// Sends `body` to `PUT devices/exempt`, and gives the status and the answer.
    fn put_exempt(setup: &Setup, body: &str) -> (u16, Value) {
        setup.send("PUT", "/api/qbp/devices/exempt", body)
    }

    /// The `exempt_macs` that `GET devices` tells, and each device's MAC with its `exempt`.
    fn exempt_state(setup: &Setup) -> (Value, Vec<(String, bool)>) {
        let (status, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(status, 200);
        let devices = body["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["mac"].as_str().unwrap().to_owned(),
                    d["exempt"].as_bool().unwrap(),
                )
            })
            .collect();
        (body["exempt_macs"].clone(), devices)
    }

    #[test]
    fn an_exempt_list_replaces_the_pushed_one_and_leaves_the_configurations_alone() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.set_exempt_config(vec![mac(9)]);
        });

        let (status, body) = put_exempt(
            &setup,
            r#"{"exempt": ["02:00:00:00:00:05", "02-00-00-00-00-05", "02:00:00:00:00:AA"]}"#,
        );

        // Two different addresses: the first two spell the same one.
        assert_eq!((status, &body["exempt"]), (200, &json!(2)));
        assert!(body["took"].is_number());
        let (macs, devices) = exempt_state(&setup);
        // Both lists, sorted, in lower case, the ones not heard yet included.
        assert_eq!(
            macs,
            json!([
                "02:00:00:00:00:05",
                "02:00:00:00:00:09",
                "02:00:00:00:00:aa"
            ])
        );
        assert_eq!(devices, [(mac_text(6), false), (mac_text(5), true)]);
        // An empty list clears the pushed one; the configuration's stays.
        let (status, body) = put_exempt(&setup, r#"{"exempt": [], "other": 1}"#);
        assert_eq!((status, &body["exempt"]), (200, &json!(0)));
        let (macs, devices) = exempt_state(&setup);
        assert_eq!(macs, json!(["02:00:00:00:00:09"]));
        assert!(devices.iter().all(|(_, exempt)| !exempt));
        // Exemption raises nothing, and is no part of the known list.
        assert!(setup.listed().is_empty());
        assert_eq!(list_state(&setup), json!("waiting"));
    }

    #[test]
    fn an_exempt_list_of_64_different_addresses_is_the_most() {
        let setup = Setup::new();
        let list = |n: u8| -> Vec<String> { (0..n).map(mac_text).collect() };
        // 64 different ones, each twice: 64.
        let twice: Vec<String> = list(64).into_iter().chain(list(64)).collect();
        let (status, body) = put_exempt(&setup, &json!({ "exempt": twice }).to_string());
        assert_eq!((status, &body["exempt"]), (200, &json!(64)));
        // One more is a 400, and the list that was there stays.
        let (status, body) = put_exempt(&setup, &json!({ "exempt": list(65) }).to_string());
        assert_eq!((status, error_key(&body)), (400, "bad_request"));
        assert!(!body.to_string().contains("02:00"), "{body}");
        assert_eq!(exempt_state(&setup).0.as_array().unwrap().len(), 64);
    }

    #[test]
    fn a_bad_exempt_list_changes_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        put_exempt(&setup, r#"{"exempt": ["02:00:00:00:00:05"]}"#);
        let bad = [
            "",
            "02:00:00:00:00:05",
            r#"{"exempt": ["#,
            "[]",
            "{}",
            r#"{"known": []}"#,
            r#"{"exempt": null}"#,
            r#"{"exempt": "02:00:00:00:00:05"}"#,
            r#"{"exempt": [5]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", null]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", "02:00:00:00:00:0"]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", "02:00:00:00:00:0g"]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", "02:00-00:00:00:06"]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", "020000000006"]}"#,
            r#"{"exempt": ["02:00:00:00:00:05", " 02:00:00:00:00:06"]}"#,
        ];
        for body in bad {
            let (status, answer) = put_exempt(&setup, body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), "bad_request", "{body}");
            // The refusal does not repeat an address.
            assert!(!answer.to_string().contains("02:00"), "{body}");
        }
        let (macs, devices) = exempt_state(&setup);
        assert_eq!(macs, json!(["02:00:00:00:00:05"]));
        assert_eq!(devices, [(mac_text(5), true)]);
    }

    /// Sends `body` to `PUT devices/policy`, and gives the status and the answer.
    fn put_policy(setup: &Setup, body: &str) -> (u16, Value) {
        setup.send("PUT", "/api/qbp/devices/policy", body)
    }

    /// What `GET devices` tells of the policy: `exempt_default`, `block_macs`, and each device's
    /// MAC with its `exempt` and `blocked`.
    fn policy_state(setup: &Setup) -> (Value, Value, Vec<(String, bool, bool)>) {
        let (status, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(status, 200);
        let devices = body["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["mac"].as_str().unwrap().to_owned(),
                    d["exempt"].as_bool().unwrap(),
                    d["blocked"].as_bool().unwrap(),
                )
            })
            .collect();
        (
            body["exempt_default"].clone(),
            body["block_macs"].clone(),
            devices,
        )
    }

    #[test]
    fn a_policy_turns_exempt_by_default_on_and_replaces_the_pushed_block_list() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.set_block_config(vec![mac(9)]);
            devices.set_exempt_config(vec![mac(5)]);
        });
        // The list mode: the exempt list says who is exempt, the block list only who is on it.
        let (on, block, devices) = policy_state(&setup);
        assert_eq!((on, block), (json!(false), json!(["02:00:00:00:00:09"])));
        assert_eq!(
            devices,
            [(mac_text(6), false, false), (mac_text(5), true, false)]
        );

        let (status, body) = put_policy(
            &setup,
            r#"{"exempt_default": true, "block": ["02:00:00:00:00:06", "02-00-00-00-00-06", "02:00:00:00:00:AA"], "other": 1}"#,
        );

        assert_eq!(
            (status, &body["exempt_default"], &body["block"]),
            (200, &json!(true), &json!(2))
        );
        assert!(body["took"].is_number());
        let (on, block, devices) = policy_state(&setup);
        assert_eq!(on, json!(true));
        // Both lists, sorted, in lower case, the ones not heard yet included.
        assert_eq!(
            block,
            json!([
                "02:00:00:00:00:06",
                "02:00:00:00:00:09",
                "02:00:00:00:00:aa"
            ])
        );
        // Every device is exempt but the blocked one; the exempt list is kept, and has no say.
        assert_eq!(
            devices,
            [(mac_text(6), false, true), (mac_text(5), true, false)]
        );
        let (_, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(body["exempt_macs"], json!(["02:00:00:00:00:05"]));
        // Off again, with an empty list: the exempt list rules, and the configuration's block
        // list stays.
        let (status, body) = put_policy(&setup, r#"{"exempt_default": false, "block": []}"#);
        assert_eq!(
            (status, &body["exempt_default"], &body["block"]),
            (200, &json!(false), &json!(0))
        );
        let (on, block, devices) = policy_state(&setup);
        assert_eq!((on, block), (json!(false), json!(["02:00:00:00:00:09"])));
        assert_eq!(
            devices,
            [(mac_text(6), false, false), (mac_text(5), true, false)]
        );
        // A policy raises nothing, and is no part of the known list.
        assert!(setup.listed().is_empty());
        assert_eq!(list_state(&setup), json!("waiting"));
    }

    #[test]
    fn a_policy_with_64_different_blocked_addresses_is_the_most() {
        let setup = Setup::new();
        let list = |n: u8| -> Vec<String> { (0..n).map(mac_text).collect() };
        let twice: Vec<String> = list(64).into_iter().chain(list(64)).collect();
        let (status, body) = put_policy(
            &setup,
            &json!({ "exempt_default": true, "block": twice }).to_string(),
        );
        assert_eq!((status, &body["block"]), (200, &json!(64)));
        let (status, body) = put_policy(
            &setup,
            &json!({ "exempt_default": false, "block": list(65) }).to_string(),
        );
        assert_eq!((status, error_key(&body)), (400, "bad_request"));
        assert!(!body.to_string().contains("02:00"), "{body}");
        // The policy that was there stays, mode and list.
        let (on, block, _) = policy_state(&setup);
        assert_eq!(on, json!(true));
        assert_eq!(block.as_array().unwrap().len(), 64);
    }

    #[test]
    fn a_bad_policy_changes_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        put_policy(
            &setup,
            r#"{"exempt_default": true, "block": ["02:00:00:00:00:05"]}"#,
        );
        let bad = [
            "",
            "true",
            r#"{"exempt_default": "#,
            "[]",
            "{}",
            r#"{"block": []}"#,
            r#"{"exempt_default": false}"#,
            r#"{"exempt_default": null, "block": []}"#,
            r#"{"exempt_default": "false", "block": []}"#,
            r#"{"exempt_default": 0, "block": []}"#,
            r#"{"exempt_default": false, "block": null}"#,
            r#"{"exempt_default": false, "block": "02:00:00:00:00:05"}"#,
            r#"{"exempt_default": false, "block": [5]}"#,
            r#"{"exempt_default": false, "block": ["02:00:00:00:00:05", null]}"#,
            r#"{"exempt_default": false, "block": ["02:00:00:00:00:05", "02:00:00:00:00:0g"]}"#,
            r#"{"exempt_default": false, "block": ["02:00:00:00:00:05", "020000000006"]}"#,
            r#"{"exempt_default": false, "block": ["02:00:00:00:00:05", " 02:00:00:00:00:06"]}"#,
            r#"{"exempt_default": false, "exempt": []}"#,
        ];
        for body in bad {
            let (status, answer) = put_policy(&setup, body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), "bad_request", "{body}");
            // The refusal does not repeat an address.
            assert!(!answer.to_string().contains("02:00"), "{body}");
        }
        let (on, block, devices) = policy_state(&setup);
        assert_eq!((on, block), (json!(true), json!(["02:00:00:00:00:05"])));
        assert_eq!(devices, [(mac_text(5), false, true)]);
    }

    #[test]
    fn a_known_list_replaces_the_old_one_and_raises_the_devices_that_are_not_on_it() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");

        let (status, body) = setup.send(
            "PUT",
            "/api/qbp/devices/known",
            r#"{"known": ["02:00:00:00:00:05", "02-00-00-00-00-05", "02:00:00:00:00:AA"]}"#,
        );

        // Two different addresses: the first two spell the same one.
        assert_eq!((status, &body["known"]), (200, &json!(2)));
        assert!(body["took"].is_number());
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(devices["known_list"], json!("received"));
        let known: Vec<(&str, bool)> = devices["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["mac"].as_str().unwrap(), d["known"].as_bool().unwrap()))
            .collect();
        assert_eq!(
            known,
            [("02:00:00:00:00:06", false), ("02:00:00:00:00:05", true)]
        );
        // The device that is not on the list is a new device now.
        let alerts = setup.listed();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["kind"], json!("new_device"));
        assert_eq!(alerts[0]["subject"], json!("02:00:00:00:00:06"));
        // A second list replaces the first.
        let (_, body) = setup.send("PUT", "/api/qbp/devices/known", r#"{"known": []}"#);
        assert_eq!(body["known"], json!(0));
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert!(
            devices["devices"]
                .as_array()
                .unwrap()
                .iter()
                .all(|d| d["known"] == json!(false))
        );
    }

    /// The MAC `02:00:00:00:00:<last>` as text.
    fn mac_text(last: u8) -> String {
        mac(last).to_string()
    }

    /// Sends one chunk of a known list: the `macs`, with the flags `append` and `complete`.
    fn chunk(setup: &Setup, macs: &[u8], append: bool, complete: bool) -> Value {
        let known: Vec<String> = macs.iter().map(|&n| mac_text(n)).collect();
        let body = json!({ "known": known, "append": append, "complete": complete });
        let (status, answer) = setup.send("PUT", "/api/qbp/devices/known", &body.to_string());
        assert_eq!(status, 200, "{answer}");
        answer
    }

    /// The `known_list` state that the API tells.
    fn list_state(setup: &Setup) -> Value {
        setup.send("GET", "/api/qbp/devices", "").1["known_list"].clone()
    }

    #[test]
    fn a_known_list_in_three_chunks_releases_the_holds_only_at_the_last() {
        let setup = Setup::new();
        for n in 5..=9 {
            setup.hear(mac(n), "device");
        }
        // The first chunk replaces, the next ones add; each says how big the set is so far.
        assert_eq!(chunk(&setup, &[5, 6], false, false)["known"], json!(2));
        assert_eq!(list_state(&setup), json!("waiting"));
        assert!(setup.listed().is_empty());
        // The devices of an earlier chunk are heard again between the chunks: nothing is raised.
        setup.hear(mac(5), "device");
        setup.hear(mac(6), "device");
        assert_eq!(chunk(&setup, &[7], true, false)["known"], json!(3));
        assert_eq!(list_state(&setup), json!("waiting"));
        assert!(setup.listed().is_empty());
        setup.hear(mac(7), "device");
        assert!(setup.listed().is_empty());
        // The last chunk completes the list: the two devices that no chunk named are raised.
        assert_eq!(chunk(&setup, &[8], true, true)["known"], json!(4));
        assert_eq!(list_state(&setup), json!("received"));
        let mut subjects: Vec<String> = setup
            .listed()
            .iter()
            .map(|alert| alert["subject"].as_str().unwrap().to_owned())
            .collect();
        subjects.sort();
        assert_eq!(subjects, [mac_text(9)]);
    }

    #[test]
    fn a_chunked_list_that_replaces_a_list_does_not_raise_the_devices_of_later_chunks() {
        let setup = Setup::new();
        for n in 5..=7 {
            setup.hear(mac(n), "device");
        }
        chunk(&setup, &[5, 6, 7], false, true);
        assert!(setup.listed().is_empty());
        // The list is sent again in two chunks; the second one has a device that the first has not.
        chunk(&setup, &[5], false, false);
        for n in 5..=7 {
            setup.hear(mac(n), "device");
        }
        assert!(setup.listed().is_empty());
        let answer = chunk(&setup, &[6], true, true);
        assert_eq!(answer["known"], json!(2));
        // Now the device that the whole list leaves out is new.
        let alerts = setup.listed();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["subject"], json!(mac_text(7)));
    }

    #[test]
    fn a_chunk_that_adds_is_added_to_the_list_that_there_is_and_the_cap_still_holds() {
        let setup = Setup::new();
        setup.hear(mac(5), "device");
        // No list is being sent: an adding chunk that is complete is a list with these added.
        chunk(&setup, &[1], false, true);
        assert_eq!(chunk(&setup, &[2], true, true)["known"], json!(2));
        // The cap of 1024 holds across chunks.
        let many = |from: u32, to: u32| -> Vec<String> {
            (from..to)
                .map(|n| Mac([2, 0, 0, 0, (n >> 8) as u8, n as u8]).to_string())
                .collect()
        };
        let body = |known: Vec<String>, append: bool| {
            json!({ "known": known, "append": append, "complete": false }).to_string()
        };
        let (_, answer) = setup.send("PUT", "/api/qbp/devices/known", &body(many(0, 600), false));
        assert_eq!(answer["known"], json!(600));
        let (_, answer) = setup.send(
            "PUT",
            "/api/qbp/devices/known",
            &body(many(300, 1300), true),
        );
        assert_eq!(answer["known"], json!(1024));
    }

    #[test]
    fn flags_that_are_not_booleans_are_a_400_and_change_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        for flags in [
            r#""append": "yes""#,
            r#""append": 1"#,
            r#""append": null"#,
            r#""complete": "false""#,
            r#""complete": 0"#,
            r#""complete": null"#,
            r#""append": [], "complete": true"#,
        ] {
            let body = format!(r#"{{"known": [], {flags}}}"#);
            let (status, answer) = setup.send("PUT", "/api/qbp/devices/known", &body);
            assert_eq!(status, 400, "{flags}");
            assert_eq!(error_key(&answer), "bad_request", "{flags}");
        }
        assert_eq!(list_state(&setup), json!("waiting"));
        assert!(setup.listed().is_empty());
        // The flags may be left out, and then it is a whole list that replaces: as before.
        let (status, answer) = setup.send("PUT", "/api/qbp/devices/known", r#"{"known": []}"#);
        assert_eq!((status, &answer["known"]), (200, &json!(0)));
        assert_eq!(list_state(&setup), json!("received"));
    }

    #[test]
    fn a_bad_known_list_changes_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        let bad = [
            "",
            "02:00:00:00:00:05",
            r#"{"known": ["#,
            "[]",
            "{}",
            r#"{"known": null}"#,
            r#"{"known": "02:00:00:00:00:05"}"#,
            r#"{"known": [5]}"#,
            r#"{"known": ["02:00:00:00:00:05", null]}"#,
            r#"{"known": ["02:00:00:00:00:05", "02:00:00:00:00:0"]}"#,
            r#"{"known": ["02:00:00:00:00:05", "02:00:00:00:00:0g"]}"#,
            r#"{"known": ["02:00:00:00:00:05", "02:00-00:00:00:06"]}"#,
            r#"{"known": ["02:00:00:00:00:05", "020000000006"]}"#,
            r#"{"known": ["02:00:00:00:00:05", " 02:00:00:00:00:06"]}"#,
        ];
        for body in bad {
            let (status, answer) = setup.send("PUT", "/api/qbp/devices/known", body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), "bad_request", "{body}");
            // The refusal does not repeat an address.
            assert!(!answer.to_string().contains("02:00"), "{body}");
        }
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(devices["known_list"], json!("waiting"));
        assert_eq!(devices["devices"][0]["known"], json!(false));
        assert!(setup.listed().is_empty());
    }

    /// Sends `body` to `PUT devices/labels`, and gives the status and the answer.
    fn put_labels(setup: &Setup, body: &str) -> (u16, Value) {
        setup.send("PUT", "/api/qbp/devices/labels", body)
    }

    /// Sends `body` to `PUT devices/gate`, and gives the status and the answer.
    fn put_gate(setup: &Setup, body: &str) -> (u16, Value) {
        setup.send("PUT", "/api/qbp/devices/gate", body)
    }

    /// The body of `GET devices`.
    fn get_devices(setup: &Setup) -> Value {
        let (status, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(status, 200);
        body
    }

    /// What `GET devices` tells of the device `n`: its label, its gate's seconds, `exempt`, `fixed`.
    fn device_facts(setup: &Setup, n: u8) -> (Value, Value, Value, Value) {
        let body = get_devices(setup);
        let device = body["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["mac"] == json!(mac_text(n)))
            .unwrap()
            .clone();
        (
            device["label"].clone(),
            device["gate_left_secs"].clone(),
            device["exempt"].clone(),
            device["fixed"].clone(),
        )
    }

    #[test]
    fn labels_come_in_the_desktops_order_and_show_on_the_devices_they_name() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        let (status, body) = put_labels(
            &setup,
            r#"{"labels": [["02:00:00:00:00:06", "Phone"], ["02-00-00-00-00-05", "Desk PC"], ["02:00:00:00:00:AA", "Not heard"], ["02:00:00:00:00:06", "Again"]], "other": 1}"#,
        );
        // Three different addresses: the first of a repeat stays. The labels moved the revision.
        assert_eq!(
            (status, &body["labels"], &body["rev"]),
            (200, &json!(3), &json!(1))
        );
        assert!(body["took"].is_number());
        assert_eq!(device_facts(&setup, 5).0, json!("Desk PC"));
        assert_eq!(device_facts(&setup, 6).0, json!("Phone"));
        assert_eq!(get_devices(&setup)["rev"], json!(1));
        // The same list again changes nothing, and the revision stays.
        let (_, body) = put_labels(
            &setup,
            r#"{"labels": [["02:00:00:00:00:06", "Phone"], ["02:00:00:00:00:05", "Desk PC"], ["02:00:00:00:00:aa", "Not heard"]]}"#,
        );
        assert_eq!((&body["labels"], &body["rev"]), (&json!(3), &json!(1)));
        // The next whole list replaces them.
        let (_, body) = put_labels(&setup, r#"{"labels": []}"#);
        assert_eq!((&body["labels"], &body["rev"]), (&json!(0), &json!(2)));
        assert_eq!(device_facts(&setup, 5).0, json!(null));
        // A label is not an exemption, and raises nothing.
        assert!(setup.listed().is_empty());
    }

    #[test]
    fn labels_in_chunks_take_effect_with_the_last_one() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        let (_, body) = put_labels(
            &setup,
            r#"{"labels": [["02:00:00:00:00:05", "Desk PC"]], "append": false, "complete": false}"#,
        );
        assert_eq!((&body["labels"], &body["rev"]), (&json!(1), &json!(0)));
        assert_eq!(device_facts(&setup, 5).0, json!(null));
        let (_, body) = put_labels(
            &setup,
            r#"{"labels": [["02:00:00:00:00:06", "Phone"]], "append": true, "complete": true}"#,
        );
        assert_eq!((&body["labels"], &body["rev"]), (&json!(2), &json!(1)));
        assert_eq!(device_facts(&setup, 5).0, json!("Desk PC"));
        assert_eq!(device_facts(&setup, 6).0, json!("Phone"));
    }

    #[test]
    fn a_bad_list_of_labels_is_a_400_and_changes_nothing_and_repeats_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        put_labels(&setup, r#"{"labels": [["02:00:00:00:00:05", "Desk PC"]]}"#);
        let sixty_five: Vec<Value> = (0..65).map(|n| json!([mac_text(n + 1), "x"])).collect();
        let sixty_five = json!({ "labels": sixty_five }).to_string();
        let long = format!(
            r#"{{"labels": [["02:00:00:00:00:06", "{}"]]}}"#,
            "x".repeat(25)
        );
        let bad = [
            "",
            "[]",
            "{}",
            r#"{"labels": ["#,
            r#"{"labels": null}"#,
            r#"{"labels": "02:00:00:00:00:06"}"#,
            r#"{"labels": [5]}"#,
            r#"{"labels": ["02:00:00:00:00:06"]}"#,
            r#"{"labels": [["02:00:00:00:00:06"]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", "Phone", "more"]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", 5]]}"#,
            r#"{"labels": [[5, "Phone"]]}"#,
            r#"{"labels": [["02:00:00:00:00:0g", "Phone"]]}"#,
            r#"{"labels": [["020000000006", "Phone"]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", ""]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", "caf\u00e9"]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", "tab\there"]]}"#,
            r#"{"labels": [["01:00:00:00:00:06", "Phone"]]}"#,
            r#"{"labels": [["00:00:00:00:00:00", "Phone"]]}"#,
            r#"{"labels": [["02:00:00:00:00:06", "Phone"], ["02:00:00:00:00:07", "bad\u0007"]]}"#,
            r#"{"labels": [], "append": "yes"}"#,
            r#"{"labels": [], "complete": 0}"#,
            &long,
            &sixty_five,
        ];
        for body in bad {
            let (status, answer) = put_labels(&setup, body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), "bad_request", "{body}");
            // The refusal repeats neither an address nor a label.
            let text = answer.to_string();
            assert!(!text.contains("02:00") && !text.contains("Phone"), "{body}");
        }
        // The labels there were stay, and the revision with them.
        assert_eq!(device_facts(&setup, 5).0, json!("Desk PC"));
        assert_eq!(get_devices(&setup)["rev"], json!(1));
        // 64 pairs and a label of 24 characters are the most.
        let sixty_four: Vec<Value> = (0..64)
            .map(|n| json!([mac_text(n + 1), "x".repeat(24)]))
            .collect();
        let (status, body) = put_labels(&setup, &json!({ "labels": sixty_four }).to_string());
        assert_eq!((status, &body["labels"]), (200, &json!(64)));
    }

    #[test]
    fn a_gate_opens_and_shows_on_the_device_and_shuts_with_zero() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        assert_eq!(
            device_facts(&setup, 5),
            (json!(null), json!(null), json!(false), json!(false))
        );

        let (status, body) = put_gate(&setup, r#"{"mac": "02:00:00:00:00:05", "secs": 300}"#);
        assert_eq!(
            (status, &body["gate_left_secs"], &body["rev"]),
            (200, &json!(300), &json!(1))
        );
        assert!(body["took"].is_number());
        // The device is exempt for now; no list names it.
        assert_eq!(
            device_facts(&setup, 5),
            (json!(null), json!(300), json!(true), json!(false))
        );
        let devices = get_devices(&setup);
        assert_eq!(
            (
                &devices["exempt_macs"],
                &devices["exempt_pushed"],
                &devices["rev"]
            ),
            (&json!([]), &json!([]), &json!(1))
        );

        // An hour is the most, and a gate that is open takes a new time.
        let (status, body) = put_gate(&setup, r#"{"mac": "02-00-00-00-00-05", "secs": 3600}"#);
        assert_eq!(
            (status, &body["gate_left_secs"], &body["rev"]),
            (200, &json!(3600), &json!(2))
        );
        assert_eq!(device_facts(&setup, 5).1, json!(3600));

        // 0 shuts it, and shutting a gate that is shut is fine and moves nothing.
        let (status, body) = put_gate(&setup, r#"{"mac": "02:00:00:00:00:05", "secs": 0}"#);
        assert_eq!(
            (status, &body["gate_left_secs"], &body["rev"]),
            (200, &json!(0), &json!(3))
        );
        assert_eq!(
            device_facts(&setup, 5),
            (json!(null), json!(null), json!(false), json!(false))
        );
        let (status, body) = put_gate(&setup, r#"{"mac": "02:00:00:00:00:05", "secs": 0}"#);
        assert_eq!((status, &body["rev"]), (200, &json!(3)));
        assert!(setup.listed().is_empty());
    }

    #[test]
    fn a_gate_that_the_inventory_refuses_is_a_409_with_a_word_and_never_the_address() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        setup.hear(mac(6), "phone");
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.set_exempt_config(vec![mac(5)]);
            // The first message that claims the router's address fixes the router's MAC.
            devices.on_arp(&Arp {
                op: 1,
                sender_mac: mac(9),
                sender_ip: Ipv4Addr::new(192, 0, 2, 1),
                target_ip: Ipv4Addr::new(192, 0, 2, 1),
            });
            devices.set_own_mac(mac(0xee));
        });
        for (n, word) in [(5, "fixed"), (9, "always"), (0xee, "always")] {
            let body = format!(r#"{{"mac": "{}", "secs": 60}}"#, mac_text(n));
            let (status, answer) = put_gate(&setup, &body);
            assert_eq!(status, 409, "{n}");
            assert_eq!(error_key(&answer), word, "{n}");
            assert!(!answer.to_string().contains("02:00"), "{answer}");
            // Shutting is refused the same way.
            let body = format!(r#"{{"mac": "{}", "secs": 0}}"#, mac_text(n));
            assert_eq!(put_gate(&setup, &body).0, 409, "{n}");
        }
        // A device that the inventory does not list is a 404, to open and to shut.
        for secs in [60, 0] {
            let body = format!(r#"{{"mac": "02:00:00:00:00:77", "secs": {secs}}}"#);
            let (status, answer) = put_gate(&setup, &body);
            assert_eq!((status, error_key(&answer)), (404, "not_found"), "{secs}");
        }
        assert_eq!(get_devices(&setup)["rev"], json!(0));

        // 64 gates open; the 65th is a 409, and a gate that is open may take a new time.
        for n in 0..65u8 {
            with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
                devices.on_arp(&Arp {
                    op: 1,
                    sender_mac: Mac([2, 0, 0, 0, 1, n]),
                    sender_ip: Ipv4Addr::new(192, 0, 2, 100 + n),
                    target_ip: Ipv4Addr::new(192, 0, 2, 1),
                });
            });
        }
        let gate_of = |n: u8| format!(r#"{{"mac": "02:00:00:00:01:{n:02x}", "secs": 60}}"#);
        for n in 0..64 {
            assert_eq!(put_gate(&setup, &gate_of(n)).0, 200, "{n}");
        }
        let (status, answer) = put_gate(&setup, &gate_of(64));
        assert_eq!((status, error_key(&answer)), (409, "full"));
        assert_eq!(put_gate(&setup, &gate_of(3)).0, 200);
    }

    #[test]
    fn a_bad_gate_body_is_a_400_and_changes_nothing_and_repeats_nothing() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        let bad = [
            "",
            "[]",
            "{}",
            r#"{"mac": "02:00:00:00:00:05"}"#,
            r#"{"secs": 60}"#,
            r#"{"mac": null, "secs": 60}"#,
            r#"{"mac": 5, "secs": 60}"#,
            r#"{"mac": "02:00:00:00:00:0g", "secs": 60}"#,
            r#"{"mac": "020000000005", "secs": 60}"#,
            r#"{"mac": "01:00:00:00:00:05", "secs": 60}"#,
            r#"{"mac": "00:00:00:00:00:00", "secs": 60}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": null}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": "60"}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": -1}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": 60.5}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": 60.0}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": 3601}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": 65536}"#,
            r#"{"mac": "02:00:00:00:00:05", "secs": 18446744073709551616}"#,
        ];
        for body in bad {
            let (status, answer) = put_gate(&setup, body);
            assert_eq!(status, 400, "{body}");
            assert_eq!(error_key(&answer), "bad_request", "{body}");
            assert!(!answer.to_string().contains("02:00"), "{body}");
        }
        assert_eq!(device_facts(&setup, 5).1, json!(null));
        assert_eq!(get_devices(&setup)["rev"], json!(0));
    }

    #[test]
    fn if_rev_makes_a_push_conditional_and_a_stale_one_is_a_409_with_the_revision_now() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        let (status, body) = put_exempt(&setup, r#"{"exempt": ["02:00:00:00:00:05"]}"#);
        assert_eq!(
            (status, &body["exempt"], &body["rev"]),
            (200, &json!(1), &json!(1))
        );

        // A push on an old revision changes nothing and says which revision it is.
        let (status, body) = put_exempt(&setup, r#"{"exempt": [], "if_rev": 0}"#);
        assert_eq!(status, 409);
        assert_eq!((&body["rev"], error_key(&body)), (&json!(1), "stale"));
        assert!(body["took"].is_number());
        assert_eq!(exempt_state(&setup).0, json!(["02:00:00:00:00:05"]));
        // On the revision now it goes through, and says the revision after.
        let (status, body) = put_exempt(&setup, r#"{"exempt": [], "if_rev": 1}"#);
        assert_eq!(
            (status, &body["exempt"], &body["rev"]),
            (200, &json!(0), &json!(2))
        );
        // A push that changes nothing, on the revision now, moves nothing.
        let (_, body) = put_exempt(&setup, r#"{"exempt": [], "if_rev": 2}"#);
        assert_eq!(body["rev"], json!(2));

        // The policy is conditional the same way, and changes the revision with its mode.
        let (status, body) = put_policy(
            &setup,
            r#"{"exempt_default": true, "block": [], "if_rev": 1}"#,
        );
        assert_eq!(
            (status, &body["rev"], error_key(&body)),
            (409, &json!(2), "stale")
        );
        assert_eq!(policy_state(&setup).0, json!(false));
        let (status, body) = put_policy(
            &setup,
            r#"{"exempt_default": true, "block": [], "if_rev": 2}"#,
        );
        assert_eq!(
            (status, &body["block"], &body["rev"]),
            (200, &json!(0), &json!(3))
        );

        // An edit of the panel moves it, and so does a gate: the desktop's push is stale.
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.set_ads_pass(mac(5), false).unwrap();
        });
        let (status, body) = put_exempt(&setup, r#"{"exempt": [], "if_rev": 3}"#);
        assert_eq!((status, &body["rev"]), (409, &json!(4)));
        assert_eq!(
            put_gate(&setup, r#"{"mac": "02:00:00:00:00:05", "secs": 60}"#).1["rev"],
            json!(5)
        );
        let (status, body) = put_policy(
            &setup,
            r#"{"exempt_default": true, "block": [], "if_rev": 4}"#,
        );
        assert_eq!((status, &body["rev"]), (409, &json!(5)));
        // The list the panel made is what `GET devices` tells as pushed.
        let devices = get_devices(&setup);
        assert_eq!(
            (&devices["rev"], &devices["block_pushed"]),
            (&json!(5), &json!(["02:00:00:00:00:05"]))
        );
    }

    #[test]
    fn an_if_rev_that_is_not_a_whole_number_is_a_400_and_changes_nothing() {
        let setup = Setup::new();
        for if_rev in ["\"x\"", "-1", "1.5", "null", "[]", "18446744073709551616"] {
            let body = format!(r#"{{"exempt": ["02:00:00:00:00:05"], "if_rev": {if_rev}}}"#);
            let (status, answer) = put_exempt(&setup, &body);
            assert_eq!(
                (status, error_key(&answer)),
                (400, "bad_request"),
                "{if_rev}"
            );
            let body = format!(r#"{{"exempt_default": true, "block": [], "if_rev": {if_rev}}}"#);
            let (status, answer) = put_policy(&setup, &body);
            assert_eq!(
                (status, error_key(&answer)),
                (400, "bad_request"),
                "{if_rev}"
            );
        }
        let devices = get_devices(&setup);
        assert_eq!(
            (&devices["rev"], &devices["exempt_macs"]),
            (&json!(0), &json!([]))
        );
        assert_eq!(devices["exempt_default"], json!(false));
    }

    #[test]
    fn a_known_path_with_another_method_is_a_405_and_changes_nothing() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        setup.hear(mac(5), "laptop");
        let paths = [
            ("GET", "/api/qbp/alerts"),
            ("POST", "/api/qbp/alerts/1/ack"),
            ("PUT", "/api/qbp/alerts/acked"),
            ("GET", "/api/qbp/devices"),
            ("PUT", "/api/qbp/devices/known"),
            ("PUT", "/api/qbp/devices/exempt"),
            ("PUT", "/api/qbp/devices/policy"),
            ("PUT", "/api/qbp/devices/labels"),
            ("PUT", "/api/qbp/devices/gate"),
        ];
        for (right, path) in paths {
            for method in [
                "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "get",
            ] {
                if method == right {
                    continue;
                }
                let body = r#"{"acked": [], "known": [], "exempt": [], "exempt_default": true, "block": [], "labels": [], "mac": "02:00:00:00:00:05", "secs": 60}"#;
                let (status, body) = setup.send(method, path, body);
                assert_eq!(status, 405, "{method} {path}");
                assert_eq!(error_key(&body), "method_not_allowed", "{method} {path}");
            }
        }
        // A method that is wrong is refused before a query is looked at.
        assert_eq!(setup.send("POST", "/api/qbp/alerts?since=x", "").0, 405);
        assert_eq!(setup.listed()[0]["acked"], json!(false));
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(devices["known_list"], json!("waiting"));
        assert_eq!(devices["exempt_macs"], json!([]));
        assert_eq!(devices["exempt_default"], json!(false));
    }

    #[test]
    fn other_paths_and_queries_on_the_other_paths_are_a_404() {
        let setup = Setup::new();
        let paths = [
            ("GET", "/api/qbp/"),
            ("GET", "/api/qbp"),
            ("GET", "/api/qbp/nope"),
            ("GET", "/api/qbp/alerts/"),
            ("GET", "/api/qbp/Alerts"),
            ("GET", "/api/qbp/alerts/1"),
            ("POST", "/api/qbp/alerts/1/ack/"),
            ("POST", "/api/qbp/alerts/1/ack/x"),
            ("POST", "/api/qbp/alerts/1/acks"),
            ("POST", "/api/qbp//alerts/1/ack"),
            ("PUT", "/api/qbp/alerts/acked/ack"),
            ("GET", "/api/qbp/devices/"),
            ("GET", "/api/qbp/devices/known/"),
            ("GET", "/api/qbp/devices/other"),
            ("PUT", "/api/qbp/devices/exempt/"),
            ("PUT", "/api/qbp/devices/Exempt"),
            ("PUT", "/api/qbp/exempt"),
            ("PUT", "/api/qbp/devices/policy/"),
            ("PUT", "/api/qbp/devices/Policy"),
            ("PUT", "/api/qbp/policy"),
            ("PUT", "/api/qbp/devices/labels/"),
            ("PUT", "/api/qbp/devices/Labels"),
            ("PUT", "/api/qbp/labels"),
            ("PUT", "/api/qbp/devices/gate/"),
            ("PUT", "/api/qbp/devices/Gate"),
            ("PUT", "/api/qbp/gate"),
            ("DELETE", "/api/qbp/nope"),
            // A query on any path but `GET alerts` is no such path.
            ("GET", "/api/qbp/devices?x=1"),
            ("GET", "/api/qbp/devices?"),
            ("PUT", "/api/qbp/devices/known?x=1"),
            ("PUT", "/api/qbp/devices/exempt?x=1"),
            ("PUT", "/api/qbp/devices/policy?x=1"),
            ("PUT", "/api/qbp/devices/labels?x=1"),
            ("PUT", "/api/qbp/devices/gate?x=1"),
            ("PUT", "/api/qbp/alerts/acked?since=0"),
            ("POST", "/api/qbp/alerts/1/ack?x=1"),
        ];
        for (method, path) in paths {
            let body = r#"{"acked": [], "known": [], "exempt": [], "exempt_default": true, "block": [], "labels": [], "mac": "02:00:00:00:00:05", "secs": 60}"#;
            let (status, body) = setup.send(method, path, body);
            assert_eq!(status, 404, "{method} {path}");
            assert_eq!(error_key(&body), "not_found", "{method} {path}");
        }
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(devices["known_list"], json!("waiting"));
        assert_eq!(devices["exempt_macs"], json!([]));
        assert_eq!(devices["exempt_default"], json!(false));
    }

    #[test]
    fn without_a_session_every_path_is_a_401_and_nothing_changes_or_shows() {
        let setup = Setup::new();
        setup.raise("dga", Some(client(7)), "random NXDOMAIN names");
        setup.hear(mac(5), "laptop");
        let body = br#"{"acked": ["0123456789abcdef"], "known": ["02:00:00:00:00:05"], "exempt": ["02:00:00:00:00:05"], "exempt_default": true, "block": ["02:00:00:00:00:05"], "labels": [["02:00:00:00:00:05", "Desk PC"]], "mac": "02:00:00:00:00:05", "secs": 60}"#;
        let requests = [
            ("GET", "/api/qbp/alerts"),
            ("GET", "/api/qbp/alerts?since=0"),
            ("POST", "/api/qbp/alerts/1/ack"),
            ("PUT", "/api/qbp/alerts/acked"),
            ("GET", "/api/qbp/devices"),
            ("PUT", "/api/qbp/devices/known"),
            ("PUT", "/api/qbp/devices/exempt"),
            ("PUT", "/api/qbp/devices/policy"),
            ("PUT", "/api/qbp/devices/labels"),
            ("PUT", "/api/qbp/devices/gate"),
            ("GET", "/api/qbp/nope"),
            ("GET", "/api/qbp/"),
            ("DELETE", "/api/qbp/alerts"),
        ];
        for (method, path) in requests {
            for sid in [None, Some("not-a-session"), Some("")] {
                let (status, answer) = ask(&setup.api, method, path, sid, body);
                assert_eq!(status, 401, "{method} {path} {sid:?}");
                assert_eq!(
                    error_key(&answer),
                    "unauthorized",
                    "{method} {path} {sid:?}"
                );
                assert!(answer.get("alerts").is_none() && answer.get("devices").is_none());
            }
        }
        // Nothing was acknowledged, restored or replaced.
        assert_eq!(setup.listed()[0]["acked"], json!(false));
        let (_, devices) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(devices["known_list"], json!("waiting"));
        assert_eq!(devices["exempt_macs"], json!([]));
        assert_eq!(
            (&devices["exempt_default"], &devices["block_macs"]),
            (&json!(false), &json!([]))
        );
        // No label, no gate and no revision moved.
        assert_eq!(devices["rev"], json!(0));
        assert_eq!(devices["devices"][0]["label"], json!(null));
        assert_eq!(devices["devices"][0]["gate_left_secs"], json!(null));
    }

    #[test]
    fn an_api_with_no_watch_answers_404_under_qbp_and_401_without_a_session() {
        let api = plain_api();
        let sid = login(&api);
        for (method, path) in [
            ("GET", "/api/qbp/alerts"),
            ("GET", "/api/qbp/devices"),
            ("PUT", "/api/qbp/devices/known"),
            ("PUT", "/api/qbp/devices/exempt"),
            ("PUT", "/api/qbp/devices/policy"),
            ("PUT", "/api/qbp/devices/labels"),
            ("PUT", "/api/qbp/devices/gate"),
            ("GET", "/api/qbp/"),
        ] {
            let (status, body) = ask(&api, method, path, Some(&sid), b"{}");
            assert_eq!(status, 404, "{method} {path}");
            assert_eq!(error_key(&body), "not_found");
            let (status, _) = ask(&api, method, path, None, b"{}");
            assert_eq!(status, 401, "{method} {path}");
        }
        // The rest of the API is the same with or without a watch.
        assert_eq!(ask(&api, "GET", "/api/auth", Some(&sid), b"").0, 200);
    }

    #[test]
    fn the_rest_of_the_api_does_not_change_with_a_watch() {
        let setup = Setup::new();
        assert_eq!(setup.send("GET", "/api/auth", "").0, 200);
        assert_eq!(setup.send("GET", "/api/stats/summary", "").0, 200);
        // `/api/qbp` is not a path of the watchdog and not of anything else.
        assert_eq!(setup.send("GET", "/api/qbpx/alerts", "").0, 404);
        assert_eq!(setup.send("GET", "/api/v1/qbp/alerts", "").0, 404);
    }

    #[test]
    fn a_poisoned_inventory_lock_still_answers() {
        let setup = Setup::new();
        setup.hear(mac(5), "laptop");
        let devices = setup.watch.devices.clone();
        let poisoned = std::thread::spawn(move || {
            let _held = devices.lock().unwrap();
            panic!("a holder of the lock panicked");
        })
        .join();
        assert!(poisoned.is_err());
        assert!(setup.watch.devices.is_poisoned());

        let (status, body) = setup.send("GET", "/api/qbp/devices", "");
        assert_eq!(status, 200);
        assert_eq!(body["devices"].as_array().unwrap().len(), 1);
        let (status, body) = setup.send("PUT", "/api/qbp/devices/known", r#"{"known": []}"#);
        assert_eq!((status, &body["known"]), (200, &json!(0)));
    }

    #[test]
    fn the_store_is_never_called_while_the_inventory_is_locked() {
        use std::sync::OnceLock;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // A store whose clock, which every raise reads, tries the lock of the inventory.
        let cell: Arc<OnceLock<Arc<Mutex<Devices>>>> = Arc::default();
        let (reads, locked) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let probe: Arc<dyn Fn() -> (u64, u64) + Send + Sync> = {
            let (cell, reads, locked) = (cell.clone(), reads.clone(), locked.clone());
            Arc::new(move || {
                reads.fetch_add(1, Ordering::Relaxed);
                if let Some(devices) = cell.get()
                    && matches!(devices.try_lock(), Err(std::sync::TryLockError::WouldBlock))
                {
                    locked.fetch_add(1, Ordering::Relaxed);
                }
                (100, UNIX)
            })
        };
        let store = Arc::new(AlertStore::new(probe));
        let devices = Arc::new(Mutex::new(Devices::new(
            None,
            Ipv4Addr::new(192, 0, 2, 1),
            clock(),
        )));
        cell.set(devices.clone()).ok().unwrap();
        let watch = Arc::new(QbpWatch {
            alerts: store,
            devices,
        });
        let api = plain_api().with_watch(watch.clone());
        let sid = login(&api);
        let setup = Setup { api, watch, sid };

        // Three devices are heard and none is listed: the list raises three alerts.
        for n in 20..23 {
            setup.hear(mac(n), "phone");
        }
        let (status, _) = setup.send("PUT", "/api/qbp/devices/known", r#"{"known": []}"#);
        assert_eq!(status, 200);
        assert_eq!(setup.listed().len(), 3);
        assert!(reads.load(Ordering::Relaxed) >= 3);
        assert_eq!(locked.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_device_that_is_not_on_the_list_is_one_alert_however_often_the_list_is_sent() {
        let setup = Setup::new();
        let arp = Arp {
            op: 1,
            sender_mac: mac(9),
            sender_ip: Ipv4Addr::new(192, 0, 2, 41),
            target_ip: Ipv4Addr::new(192, 0, 2, 1),
        };
        with_devices(&setup.watch.devices, &*setup.watch.alerts, |devices| {
            devices.on_arp(&arp);
        });
        for _ in 0..3 {
            let (status, _) = setup.send("PUT", "/api/qbp/devices/known", r#"{"known": []}"#);
            assert_eq!(status, 200);
        }
        let alerts = setup.listed();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0]["count"], json!(1));
    }
}
