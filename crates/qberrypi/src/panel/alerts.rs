//! The alerts the panel takes over its screen for: what it reads of them, and what it does to them.
//!
//! The panel never owns an alert. It asks a [`PendingAlerts`] for the one to show and how many wait,
//! once a second, and tells it which one a tap acknowledged. The behaviour watch keeps the real store;
//! the shapes here mirror the ones that watch shares (`Severity { Info, Warn, High }`, a `kind`, an
//! optional `client`, a `subject`, how many times and since when), plus the `id` that the
//! acknowledgement names.
//!
//! **Wiring.** `watch::store::AlertStore` implements [`PendingAlerts`], and `wiring::Slot` hands
//! the panel the store once the first service has made it, so that the panel holds its source of
//! alerts from the start (`init.rs::panel_deps`):
//!
//! - `top()` is the newest unacknowledged alert of the highest severity, mapped to a
//!   [`PanelAlert`] whose `id` is the alert's `AlertView.id`;
//! - `waiting_count()` is how many alerts are not acknowledged;
//! - `ack(id)` is the store's `ack(id)`;
//! - the store's `Severity` has the same three variants, so a `From` impl is enough.
//!
//! An alert's subject is a DNS name, or a device's hardware address. Like the lane's names it reaches
//! the physical screen and nothing else: [`PanelAlert`] implements no `Debug`, and nothing in the
//! panel logs it.

use std::net::IpAddr;

/// How much an alert matters; the panel shows `High` on a white card and the rest on a blue one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Worth knowing.
    Info,
    /// Worth a look.
    Warn,
    /// Worth interrupting for.
    High,
}

impl From<crate::alerts::Severity> for Severity {
    fn from(severity: crate::alerts::Severity) -> Self {
        match severity {
            crate::alerts::Severity::Info => Self::Info,
            crate::alerts::Severity::Warn => Self::Warn,
            crate::alerts::Severity::High => Self::High,
        }
    }
}

/// One alert, as the panel reads it from the store.
///
/// It has no `Debug`: the subject is a name or an address.
#[derive(Clone)]
pub struct PanelAlert {
    /// What the store calls it, and what an acknowledgement names.
    pub id: u64,
    /// What sort of alert it is: `threat`, `rebind`, `dga`, `tunnel`, `beacon`, `burst`,
    /// `new_device`, `arp_spoof`, or another that the card spells out of the kind itself.
    pub kind: String,
    /// How much it matters.
    pub severity: Severity,
    /// The device it concerns, if it names one.
    pub client: Option<IpAddr>,
    /// What it is about: the name that was asked for, or for a device alert the device's hardware
    /// address as the store keeps it (`02:00:00:00:00:01`).
    pub subject: String,
    /// How many times it was raised.
    pub count: u64,
    /// The Unix time when it was first raised.
    pub first_unix: u64,
    /// The Unix time when it was last raised.
    pub last_unix: u64,
}

/// Where the panel gets the alert to show, and where it says that one was seen.
pub trait PendingAlerts: Send + Sync {
    /// The alert to show now: unacknowledged, of the highest severity there is, the newest of
    /// those. `None` when there is none.
    fn top(&self) -> Option<PanelAlert>;

    /// How many alerts are waiting: not acknowledged.
    fn waiting_count(&self) -> u32;

    /// Marks the alert `id` as seen, so that `top` stops giving it. An `id` that is not known is
    /// not an error.
    fn ack(&self, id: u64);
}

/// A source with no alert, ever: what the panel's own tests give it.
pub struct NoAlerts;

impl PendingAlerts for NoAlerts {
    fn top(&self) -> Option<PanelAlert> {
        None
    }

    fn waiting_count(&self) -> u32 {
        0
    }

    fn ack(&self, _id: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_has_no_alert_and_ignores_an_ack() {
        let alerts: &dyn PendingAlerts = &NoAlerts;
        assert!(alerts.top().is_none());
        assert_eq!(alerts.waiting_count(), 0);
        alerts.ack(7);
        assert!(alerts.top().is_none());
    }
}
