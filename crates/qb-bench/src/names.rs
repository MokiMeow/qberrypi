//! The names a load asks for, by class: cached, blocked and forwarded.
//!
//! A name is fixed, and its query is made once and given a new ID for each send, or a pattern
//! whose `*` labels are fresh random labels each time, so that no server can have the answer:
//! `*.example.com` asks `3f9c0d2e8a1b4c7d.example.com`, then another.
//!
//! - **Cached:** a few real names, asked once each before the load ([`CACHED_NAMES`]), so that
//!   every question of the load is a cache hit.
//! - **Blocked:** names the server blocks: the names given, or the first of [`BLOCKED_CANDIDATES`]
//!   that answers as blocked. A pattern (`*.doubleclick.net`) tests a server that blocks a
//!   name's subdomains too, with a name it has never seen.
//! - **Forwarded:** fresh labels under six real zones of six operators, as `cargo xtask latency`
//!   asks them ([`FORWARDED_ZONES`]). Every one of these questions goes to the server's upstream:
//!   for qberrypi that is Quad9, so the rate is kept modest by default.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::wire::{NameError, TYPE_A, encode_query, set_id};

/// The kinds of question a load asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Cached,
    Blocked,
    Forwarded,
}

impl Class {
    /// Every class, in the order a run measures them.
    pub const ALL: [Class; 3] = [Class::Cached, Class::Blocked, Class::Forwarded];

    /// The class's name in reports and on the command line.
    pub fn word(self) -> &'static str {
        match self {
            Class::Cached => "cached",
            Class::Blocked => "blocked",
            Class::Forwarded => "forwarded",
        }
    }

    /// The class called `word`.
    pub fn from_word(word: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|class| class.word() == word)
    }
}

/// The zones of the forwarded class. Each is a zone of a different operator, none of them signed
/// and none with a wildcard, so that a fresh label under each is a real NXDOMAIN that only the
/// zone's own servers can give (see `xtask/src/latency.rs`, which checked them on 2026-11-10).
pub const FORWARDED_ZONES: [&str; 6] = [
    "google.com",
    "amazon.com",
    "microsoft.com",
    "wikipedia.org",
    "mozilla.org",
    "kernel.org",
];

/// The names of the cached class: real names, each asked once before the load.
pub const CACHED_NAMES: [&str; 8] = [
    "example.com",
    "example.org",
    "example.net",
    "iana.org",
    "wikipedia.org",
    "kernel.org",
    "mozilla.org",
    "debian.org",
];

/// Well-known ad-serving names that the usual lists hold, tried in this order until one answers
/// as blocked.
pub const BLOCKED_CANDIDATES: [&str; 3] = [
    "doubleclick.net",
    "googleadservices.com",
    "pagead2.googlesyndication.com",
];

/// The forwarded class's patterns: a fresh label under each zone in turn.
pub fn forwarded_patterns() -> Vec<String> {
    FORWARDED_ZONES
        .iter()
        .map(|zone| format!("*.{zone}"))
        .collect()
}

/// A small fast generator of random labels: SplitMix64, seeded from the keys the system gives a
/// hasher and the clock. Not for secrets: it only has to make names that no server has seen.
#[derive(Debug, Clone)]
struct Labels(u64);

impl Labels {
    fn new() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        hasher.write_u128(nanos);
        Self(hasher.finish())
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// One entry of a load's names.
#[derive(Debug, Clone)]
enum Entry {
    /// A fixed name, its query made once.
    Fixed(Vec<u8>),
    /// A name with `*` labels, made afresh for each question.
    Pattern(String),
}

/// The names a load asks, in turn, each send taking the next.
#[derive(Debug, Clone)]
pub struct Names {
    entries: Vec<Entry>,
    next: usize,
    labels: Labels,
    /// The name of a pattern, as it is filled in.
    scratch: String,
}

impl Names {
    /// The names of `list`, asked in turn as A questions: an entry with a label that is `*` is a
    /// pattern, every other is fixed. An empty list, or a name that cannot be asked, is an error
    /// that says which.
    pub fn new(list: &[String]) -> Result<Names, String> {
        if list.is_empty() {
            return Err("no names to ask".to_owned());
        }
        let mut labels = Labels::new();
        let mut entries = Vec::with_capacity(list.len());
        let mut packet = Vec::new();
        for name in list {
            let pattern = name.split('.').any(|label| label == "*");
            // A pattern is checked once with a label of the length it will have.
            let checked = if pattern {
                fill(name, &mut labels)
            } else {
                name.clone()
            };
            encode_query(&mut packet, 0, &checked, TYPE_A)
                .map_err(|error: NameError| format!("{name}: {error}"))?;
            entries.push(if pattern {
                Entry::Pattern(name.clone())
            } else {
                Entry::Fixed(packet.clone())
            });
        }
        Ok(Names {
            entries,
            next: 0,
            labels,
            scratch: String::new(),
        })
    }

    /// Writes the next question into `out` under `id`.
    pub fn write_next(&mut self, id: u16, out: &mut Vec<u8>) {
        let at = self.next;
        self.next = (self.next + 1) % self.entries.len();
        match &self.entries[at] {
            Entry::Fixed(packet) => {
                out.clear();
                out.extend_from_slice(packet);
                set_id(out, id);
            }
            Entry::Pattern(pattern) => {
                fill_into(pattern, &mut self.labels, &mut self.scratch);
                // Checked in `new` with a label of the same length.
                let _ = encode_query(out, id, &self.scratch, TYPE_A);
            }
        }
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none; never, as `new` refuses an empty list.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `pattern` with each `*` label a fresh label of 16 hex digits.
fn fill(pattern: &str, labels: &mut Labels) -> String {
    let mut out = String::new();
    fill_into(pattern, labels, &mut out);
    out
}

fn fill_into(pattern: &str, labels: &mut Labels, out: &mut String) {
    use std::fmt::Write;
    out.clear();
    for (i, label) in pattern.split('.').enumerate() {
        if i > 0 {
            out.push('.');
        }
        if label == "*" {
            let _ = write!(out, "{:016x}", labels.next());
        } else {
            out.push_str(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{HEADER_LEN, header};

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|&n| n.to_owned()).collect()
    }

    /// The name asked in `packet`, as text.
    fn asked(packet: &[u8]) -> String {
        let mut at = HEADER_LEN;
        let mut labels = Vec::new();
        while packet[at] != 0 {
            let len = usize::from(packet[at]);
            labels.push(String::from_utf8(packet[at + 1..at + 1 + len].to_vec()).unwrap());
            at += 1 + len;
        }
        labels.join(".")
    }

    #[test]
    fn fixed_names_are_asked_in_turn_under_the_id_given() {
        let mut names = Names::new(&strings(&["a.example", "b.example"])).unwrap();
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for id in 1..=3 {
            names.write_next(id, &mut out);
            assert_eq!(header(&out).unwrap().id, id);
            seen.push(asked(&out));
        }
        assert_eq!(seen, ["a.example", "b.example", "a.example"]);
    }

    #[test]
    fn a_pattern_is_a_fresh_label_each_time() {
        let mut names = Names::new(&strings(&["*.example.com"])).unwrap();
        let mut out = Vec::new();
        names.write_next(1, &mut out);
        let first = asked(&out);
        names.write_next(2, &mut out);
        let second = asked(&out);
        assert_ne!(first, second);
        for name in [&first, &second] {
            let (label, zone) = name.split_once('.').unwrap();
            assert_eq!(zone, "example.com");
            assert_eq!(label.len(), 16);
            assert!(label.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        // A `*` inside a label is no pattern.
        assert!(Names::new(&strings(&["a*b.example"])).is_ok());
    }

    #[test]
    fn a_name_that_cannot_be_asked_is_refused_with_its_reason() {
        assert_eq!(
            Names::new(&strings(&["a..example"])).unwrap_err(),
            "a..example: the name has an empty label"
        );
        assert!(Names::new(&[]).is_err());
        // The fresh label's 16 bytes are counted: with them this is over 255, with `*` it is not.
        let long = format!("*.{}", vec!["a".repeat(60); 4].join("."));
        assert_eq!(
            Names::new(&[long]).unwrap_err().split_once(": ").unwrap().1,
            "the name is longer than 255 bytes"
        );
    }

    #[test]
    fn the_classes_have_their_words_and_the_forwarded_zones_are_patterns() {
        for class in Class::ALL {
            assert_eq!(Class::from_word(class.word()), Some(class));
        }
        assert_eq!(Class::from_word("other"), None);
        assert_eq!(forwarded_patterns()[0], "*.google.com");
        assert_eq!(forwarded_patterns().len(), 6);
    }
}
