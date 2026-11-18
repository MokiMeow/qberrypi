//! The owner's own names: the ones they allow and the ones they refuse, set from the front panel
//! or the API and kept in memory only.
//!
//! An [`OwnerNames`] is two small sorted lists, each at most [`OWNER_NAMES_MAX`] names. A name
//! matches an entry when it is the entry or a subdomain of it, as a `deny` or `allow` entry of the
//! configuration does. The [`Resolver`](crate::Resolver) holds one beside its block set, so a
//! refresh of the lists keeps it, and consults it for a query only when it is not empty: with
//! both lists empty the cost of a query is the two checks of [`OwnerNames::is_empty`].
//!
//! The names are the owner's own, so nothing here prints one: `Debug` says how many there are,
//! and an error carries no text.

use crate::name::{MAX_NAME_LEN, normalize};

/// The most names either list of an [`OwnerNames`] holds.
pub const OWNER_NAMES_MAX: usize = 128;

/// Why a set of owner names was not built. It says nothing of the entry at fault: a caller must
/// not repeat the owner's names in a log line or an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerNamesError {
    /// An entry is not a valid domain name (see [`normalize`](crate::name::normalize)).
    Invalid,
    /// A list holds more than [`OWNER_NAMES_MAX`] distinct names.
    TooMany,
}

/// Which of the two lists holds a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerSide {
    /// The owner lets the name through even though an ads list or a `deny` entry blocks it. A
    /// threat list still blocks it.
    Allow,
    /// The owner refuses the name, for every client.
    Refuse,
}

/// The names the owner allows and the names the owner refuses.
///
/// Build it before taking the lock on the resolver, then hand it to
/// [`Resolver::set_owner_names`](crate::Resolver::set_owner_names), which swaps it in.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OwnerNames {
    /// Names let through, normalised, in bytewise order, without duplicates.
    allow: Vec<Box<str>>,
    /// Names refused, normalised, in bytewise order, without duplicates.
    refuse: Vec<Box<str>>,
}

impl core::fmt::Debug for OwnerNames {
    /// The two lengths only: the names are the owner's.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OwnerNames")
            .field("allow", &self.allow.len())
            .field("refuse", &self.refuse.len())
            .finish()
    }
}

impl OwnerNames {
    /// The names of `allow` and of `refuse`, each normalised (lower case, one trailing dot
    /// dropped), sorted and without duplicates. An entry that is not a valid name, or a list of
    /// more than [`OWNER_NAMES_MAX`] distinct names, is an error and builds nothing.
    pub fn new<A, R>(allow: A, refuse: R) -> Result<Self, OwnerNamesError>
    where
        A: IntoIterator,
        A::Item: AsRef<str>,
        R: IntoIterator,
        R::Item: AsRef<str>,
    {
        Ok(Self {
            allow: sorted(allow)?,
            refuse: sorted(refuse)?,
        })
    }

    /// Whether both lists are empty: two length checks, which is all a query costs then.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.refuse.is_empty()
    }

    /// The names let through, in bytewise order.
    pub fn allow_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.allow.iter().map(|name| &**name)
    }

    /// The names refused, in bytewise order.
    pub fn refuse_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.refuse.iter().map(|name| &**name)
    }

    /// Which list covers `name`, the entry itself or a parent of it: the refused list first, as
    /// a query is decided, then the allowed one. `name` may be spelled as the log holds it or as
    /// a client wrote it, as for [`Resolver::lists`](crate::Resolver::lists); a name that is not
    /// a valid one is on neither. Allocates nothing.
    pub fn side(&self, name: &str) -> Option<OwnerSide> {
        let mut buf = [0; MAX_NAME_LEN];
        let name = normalize(name.as_bytes(), &mut buf)?;
        if self.refuses(name) {
            Some(OwnerSide::Refuse)
        } else if self.allows(name) {
            Some(OwnerSide::Allow)
        } else {
            None
        }
    }

    /// Whether `name`, already normalised, or a parent of it is refused. A binary search for the
    /// name and for each parent: at most four for an ordinary name.
    pub(crate) fn refuses(&self, name: &str) -> bool {
        covers(&self.refuse, name)
    }

    /// Whether `name`, already normalised, or a parent of it is allowed.
    pub(crate) fn allows(&self, name: &str) -> bool {
        covers(&self.allow, name)
    }
}

/// The normalised, sorted and deduplicated names of `entries`.
fn sorted<I>(entries: I) -> Result<Vec<Box<str>>, OwnerNamesError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut buf = [0; MAX_NAME_LEN];
    let mut names: Vec<Box<str>> = Vec::new();
    for entry in entries {
        let name =
            normalize(entry.as_ref().as_bytes(), &mut buf).ok_or(OwnerNamesError::Invalid)?;
        names.push(name.into());
    }
    names.sort_unstable();
    names.dedup();
    if names.len() > OWNER_NAMES_MAX {
        return Err(OwnerNamesError::TooMany);
    }
    names.shrink_to_fit();
    Ok(names)
}

/// Whether `name` or one of its parents, one label shorter at a time, is in `list`.
fn covers(list: &[Box<str>], name: &str) -> bool {
    if list.is_empty() {
        return false;
    }
    let mut covering = name;
    loop {
        if list
            .binary_search_by(|entry| entry.as_bytes().cmp(covering.as_bytes()))
            .is_ok()
        {
            return true;
        }
        match covering.split_once('.') {
            Some((_, parent)) => covering = parent,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(allow: &[&str], refuse: &[&str]) -> OwnerNames {
        OwnerNames::new(allow, refuse).unwrap()
    }

    #[test]
    fn names_are_normalised_sorted_and_stored_once() {
        let n = names(&["B.Example.ORG.", "a.example.org", "b.example.org"], &[]);
        assert_eq!(
            n.allow_names().collect::<Vec<_>>(),
            ["a.example.org", "b.example.org"]
        );
        assert_eq!(n.refuse_names().len(), 0);
        assert!(!n.is_empty());
        assert!(OwnerNames::default().is_empty());
        assert!(names(&[], &[]).is_empty());
    }

    #[test]
    fn a_name_matches_itself_and_its_parents_at_label_boundaries() {
        let n = names(&["ok.example.org"], &["ads.example.net", "test"]);
        for (name, want) in [
            ("ads.example.net", Some(OwnerSide::Refuse)),
            ("x.y.ads.example.net", Some(OwnerSide::Refuse)),
            ("ADS.Example.NET.", Some(OwnerSide::Refuse)),
            ("a.test", Some(OwnerSide::Refuse)),
            ("test", Some(OwnerSide::Refuse)),
            ("ok.example.org", Some(OwnerSide::Allow)),
            ("deep.ok.example.org", Some(OwnerSide::Allow)),
            // A suffix of the text that is not a parent.
            ("notads.example.net", None),
            ("contest", None),
            ("example.net", None),
            ("example.org", None),
            ("xok.example.org", None),
            // Not a valid name: on neither list.
            ("", None),
            ("a b.test", None),
        ] {
            assert_eq!(n.side(name), want, "{name:?}");
        }
    }

    #[test]
    fn the_refused_list_is_asked_first() {
        let n = names(&["example.org"], &["ads.example.org"]);
        assert_eq!(n.side("ads.example.org"), Some(OwnerSide::Refuse));
        assert_eq!(n.side("x.example.org"), Some(OwnerSide::Allow));
    }

    #[test]
    fn bad_entries_and_long_lists_build_nothing() {
        for bad in ["", "not a name", "*.example.org", "192.0.2.1", "a..b"] {
            assert_eq!(
                OwnerNames::new([bad], [] as [&str; 0]),
                Err(OwnerNamesError::Invalid),
                "{bad:?}"
            );
            assert_eq!(
                OwnerNames::new([] as [&str; 0], ["fine.example.org", bad]),
                Err(OwnerNamesError::Invalid),
                "{bad:?}"
            );
        }
        let many = |n: usize| {
            (0..n)
                .map(|i| format!("n{i}.example.org"))
                .collect::<Vec<_>>()
        };
        let full = OwnerNames::new(many(OWNER_NAMES_MAX), many(OWNER_NAMES_MAX)).unwrap();
        assert_eq!(
            (full.allow_names().len(), full.refuse_names().len()),
            (OWNER_NAMES_MAX, OWNER_NAMES_MAX)
        );
        assert_eq!(
            OwnerNames::new(many(OWNER_NAMES_MAX + 1), [] as [&str; 0]),
            Err(OwnerNamesError::TooMany)
        );
        assert_eq!(
            OwnerNames::new([] as [&str; 0], many(OWNER_NAMES_MAX + 1)),
            Err(OwnerNamesError::TooMany)
        );
        // Duplicates are one name: 129 entries of 128 names fit.
        let mut with_copy = many(OWNER_NAMES_MAX);
        with_copy.push("N0.example.org.".into());
        assert!(OwnerNames::new(with_copy, [] as [&str; 0]).is_ok());
    }

    #[test]
    fn a_full_list_finds_every_name_and_nothing_else() {
        let list: Vec<String> = (0..OWNER_NAMES_MAX)
            .map(|i| format!("n{i}.example.org"))
            .collect();
        let n = OwnerNames::new([] as [&str; 0], &list).unwrap();
        for name in &list {
            assert!(n.refuses(name), "{name}");
            assert!(n.refuses(&format!("sub.{name}")), "{name}");
        }
        assert!(!n.refuses("n128.example.org") && !n.refuses("example.org"));
        assert!(!n.allows("n0.example.org"));
    }

    #[test]
    fn debug_shows_no_name() {
        let n = names(
            &["secret.example.org"],
            &["private.example.net", "x.example.net"],
        );
        let shown = format!("{n:?}");
        assert!(!shown.contains("example"), "{shown}");
        assert!(shown.contains('1') && shown.contains('2'), "{shown}");
    }
}
