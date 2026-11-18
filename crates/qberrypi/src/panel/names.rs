//! What the panel asks of the owner's names and does to them: the control the panel is given, in
//! the way `DeviceControl` is the devices' and `PendingAlerts` the alerts'.
//!
//! **Wiring.** `wiring::LiveNames` implements [`NamesControl`] over the owner's list
//! ([`crate::names::OwnerList`], which the API edits too) and the resolver; `wiring::Slot` hands the
//! panel that once the first service has made it (`Slot::fill_names`), so that the panel holds its
//! control from the start. Until then nothing is on any list, the top of the day is empty, and an
//! edit is refused with [`NamesError::Unavailable`].
//!
//! Every read copies out and lets go. `top_today` and `class` take the resolver's lock for a copy
//! of at most eight names or for one lookup that allocates nothing; the rest is the owner's list's
//! own lock. The panel's thread holds no lock between two calls. A name that a read gives back is
//! the house's: [`TopName`] and [`OwnerEntry`] print no name, and nothing the panel does with them
//! is logged.
//!
//! The edits are the panel's: `set` carries the wall time of the tap as the entry's day, as the API's
//! push never does, and moves the list's revision as every edit does, so qnet adopts it.

use crate::names::{NamesError, OwnerEntry, OwnerHold};
pub use qb_dns::{OwnerSide, TopName};

/// Which list of the house blocks a name, as the NAME screen says it. A threat list wins over an ads
/// list, and the configuration's own `deny` entries are told from the ads lists.
///
/// The owner's own entries are not a class: they are [`NamesControl::owner_hold`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameClass {
    /// No list blocks it (the owner's entries aside).
    None,
    /// An ads or trackers list blocks it.
    Ads,
    /// The configuration's `dns.deny` blocks it.
    Deny,
    /// A threat list blocks it; no entry of the owner can let it through.
    Threat,
}

/// Where the panel reads the owner's names from, and what it does to them.
pub trait NamesControl: Send + Sync {
    /// The names blocked most often today, most often first, at most eight, with how often
    /// (`Stats::top_today`): a copy under the resolver's lock. Empty after midnight until the first
    /// block of the new day, and while there is no resolver.
    fn top_today(&self) -> Vec<TopName>;

    /// How many entries the owner's list holds, on both sides.
    fn owner_len(&self) -> usize;

    /// The newest `n` entries of the owner's list, the newest first.
    fn owner_newest(&self, n: usize) -> Vec<OwnerEntry>;

    /// What the owner's list says of `name` as the resolver would decide it: which side covers it
    /// (the entry or a parent), whether the name itself is an entry (only then can the owner take
    /// it back), and the day set from the panel. `None` when no entry covers it.
    fn owner_hold(&self, name: &str) -> Option<OwnerHold>;

    /// Which list blocks `name`, from the block set and the configuration's `deny`; the owner's
    /// entries and a pause are not asked. One lookup under the resolver's lock.
    fn class(&self, name: &str) -> NameClass;

    /// Puts `name` on the owner's list `verdict`, with the day of the tap; the entry the name had on
    /// the other side goes. The panel never allows a name on a threat list
    /// ([`NamesError::Threat`]). Moves the revision unless the name is on that side already.
    fn set(&self, name: &str, verdict: OwnerSide) -> Result<(), NamesError>;

    /// Takes the entry of exactly `name` off the list, whichever side it is on: `REFUSE AGAIN` and
    /// `ANSWER AGAIN`. Whether there was one.
    fn forget(&self, name: &str) -> Result<bool, NamesError>;
}

/// A control with no names, for as long as nothing measures any: what the panel's own tests give it.
pub struct NoNames;

impl NamesControl for NoNames {
    fn top_today(&self) -> Vec<TopName> {
        Vec::new()
    }

    fn owner_len(&self) -> usize {
        0
    }

    fn owner_newest(&self, _n: usize) -> Vec<OwnerEntry> {
        Vec::new()
    }

    fn owner_hold(&self, _name: &str) -> Option<OwnerHold> {
        None
    }

    fn class(&self, _name: &str) -> NameClass {
        NameClass::None
    }

    fn set(&self, _name: &str, _verdict: OwnerSide) -> Result<(), NamesError> {
        Err(NamesError::Unavailable)
    }

    fn forget(&self, _name: &str) -> Result<bool, NamesError> {
        Err(NamesError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_has_no_names_and_refuses_every_edit() {
        let names: &dyn NamesControl = &NoNames;
        assert!(names.top_today().is_empty());
        assert_eq!(names.owner_len(), 0);
        assert!(names.owner_newest(3).is_empty());
        assert_eq!(names.owner_hold("a.example.org"), None);
        assert_eq!(names.class("a.example.org"), NameClass::None);
        assert_eq!(
            names.set("a.example.org", OwnerSide::Refuse),
            Err(NamesError::Unavailable)
        );
        assert_eq!(names.forget("a.example.org"), Err(NamesError::Unavailable));
    }
}
