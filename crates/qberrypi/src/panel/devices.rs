//! What the panel asks of the device inventory and does to it: the control the panel is given, in the
//! way `BlockingControl` is the switch and `PendingAlerts` the alerts.
//!
//! **Wiring.** `wiring::LiveDevices` implements [`DeviceControl`] over the inventory's own lock
//! (`Mutex<Devices>`, never the resolver's), the resolver's table of who asked lately
//! (`Dns::clients`) and the lane feed's watch of one client; `wiring::Slot` hands the panel that once
//! the first service has made it, so that the panel holds its control from the start
//! (`init.rs::panel_deps`). Until then there is no inventory, and the edits are refused.
//!
//! Every read copies out and lets go: the panel's thread holds no lock between two calls. The
//! inventory is copied into the panel's own buffer, lean and bounded (`Devices::wall_copy`): the lock
//! is the one `devwatch` takes for every packet on the thread that answers DNS, so it is held for a
//! walk and a copy of the few fields the wall needs, and the wall's own ordering, naming and fitting
//! are done after it is let go. A device is named by its hardware address, which is also what the
//! panel's screens carry. Nothing a copy holds (a label, a hostname, an address) is ever logged: the
//! panel prints no copy, and the one line it logs for an edit says what kind of edit it was and
//! nothing of whom.

use std::net::Ipv4Addr;

use crate::lane::WatchedName;
use crate::watch::devices::{EditError, WallCopy};

/// A device's hardware address, as the screens carry it.
pub type Mac = [u8; 6];

/// Where the panel reads the devices from, and what it does to one.
pub trait DeviceControl: Send + Sync {
    /// Copies what the wall reads of the inventory into `out`, under the inventory's own lock, and
    /// says whether there is an inventory: `false` while there is none yet, and `out` is left as it
    /// was. `recent` says of a device's last sighting, in unix time, whether the wall still shows a
    /// device that has no label (`Devices::wall_copy`). It allocates nothing once `out` is made.
    fn wall(&self, recent: &dyn Fn(u64) -> bool, out: &mut WallCopy) -> bool;

    /// Puts in `out`, whose contents it replaces, the addresses that asked something less than
    /// `window_secs` seconds ago (at the resolver's second granularity: a window of 2 is the last
    /// second or two). Empty while nothing is known.
    fn asking(&self, window_secs: u64, out: &mut Vec<Ipv4Addr>);

    /// Lets the device's ads pass for good (`pass`) or blocks them again; what `Devices::set_ads_pass`
    /// says of it, or `Unknown` while there is no inventory.
    fn set_ads_pass(&self, mac: Mac, pass: bool) -> Result<(), EditError>;

    /// Opens the device's own gate for `secs` seconds, or shuts it with 0; what `Devices::open_gate`
    /// says of it, or `Unknown` while there is no inventory.
    fn device_gate(&self, mac: Mac, secs: u16) -> Result<(), EditError>;

    /// Keeps the names that the device at `client` asks from now on, and forgets those of the one
    /// kept before; `None` keeps nothing (`LaneFeed::watch_client`).
    fn watch_client(&self, client: Option<Ipv4Addr>);

    /// The names of the watched device, newest first, into `out`, whose contents they replace
    /// (`LaneFeed::watched_names`).
    fn watched_names(&self, out: &mut Vec<WatchedName>);
}

/// A control with no devices, for as long as nothing measures any: what the panel's own tests give it.
pub struct NoDevices;

impl DeviceControl for NoDevices {
    fn wall(&self, _recent: &dyn Fn(u64) -> bool, _out: &mut WallCopy) -> bool {
        false
    }

    fn asking(&self, _window_secs: u64, out: &mut Vec<Ipv4Addr>) {
        out.clear();
    }

    fn set_ads_pass(&self, _mac: Mac, _pass: bool) -> Result<(), EditError> {
        Err(EditError::Unknown)
    }

    fn device_gate(&self, _mac: Mac, _secs: u16) -> Result<(), EditError> {
        Err(EditError::Unknown)
    }

    fn watch_client(&self, _client: Option<Ipv4Addr>) {}

    fn watched_names(&self, out: &mut Vec<WatchedName>) {
        out.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_has_no_devices_and_refuses_every_edit() {
        let devices: &dyn DeviceControl = &NoDevices;
        assert!(!devices.wall(&|_| true, &mut WallCopy::new()));
        let mut out = vec![Ipv4Addr::LOCALHOST];
        devices.asking(2, &mut out);
        assert!(out.is_empty());
        assert_eq!(
            devices.set_ads_pass([2, 0, 0, 0, 0, 1], true),
            Err(EditError::Unknown)
        );
        assert_eq!(
            devices.device_gate([2, 0, 0, 0, 0, 1], 300),
            Err(EditError::Unknown)
        );
        devices.watch_client(Some(Ipv4Addr::new(192, 0, 2, 1)));
        let mut names = Vec::new();
        devices.watched_names(&mut names);
        assert!(names.is_empty());
    }
}
