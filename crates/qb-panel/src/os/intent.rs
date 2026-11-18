//! What the panel asks the system to do: an `Intent` (spec §4). qb-panel returns intents and
//! qberrypi carries them out; nothing here does anything.
//!
//! `is_strong` is the one list of spec §7.1: the actions that ask for the PIN when the lock is on.
//!
//! An intent can hold a device's address, a name the owner typed or PIN digits, so its `Debug` says
//! what kind of intent it is and never what is in it: nothing a log line could print is private.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::os::world::Mac;

/// What the main gate or a device's gate is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMove {
    /// Open for this many seconds: 300, 1,800 or 3,600.
    Open(u16),
    /// Shut now.
    Shut,
}

/// What the owner decides for one name (spec §3.7).
///
/// The spec says `Allow | Refuse | Clear`; a `Clear` is two things that are not alike, the one that
/// loosens and the one that tightens, and §7.1 asks the PIN for only the first, so they are two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `ALLOW NAME`: answer it though the lists refuse it. Strong.
    Allow,
    /// `REFUSE NAME`: refuse it though the lists answer it. Never asks.
    Refuse,
    /// `REFUSE AGAIN`: take back the owner's allow, so the lists decide again. Stricter; never asks.
    ClearAllow,
    /// `ANSWER AGAIN`: take back the owner's refusal, so the lists decide again. It loosens: strong.
    ClearRefuse,
}

/// One digit of the PIN, 0 to 9, as a key of the pad gives it. A PIN digit is a secret wherever it goes: its `Debug`
/// prints none, so no log line that prints a `Tap` or a `Change` can leak one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Digit(u8);

impl Digit {
    /// The digit; a value over 9 is not a digit and is kept as 9.
    pub const fn new(digit: u8) -> Self {
        Self(if digit > 9 { 9 } else { digit })
    }

    /// The digit's value, 0 to 9.
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl fmt::Debug for Digit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Digit(*)")
    }
}

/// Four PIN digits, each 0 to 9, as the pad gives them to the one that verifies or keeps them (spec §7.3).
///
/// They are held in a `Zeroizing<[u8; 4]>` from the pad to the verify: they are wiped when the value is dropped.
/// `Debug` prints no digit, and two sets are compared in constant time, so that no use of `==` on them leaks
/// how many digits matched.
#[derive(Clone)]
pub struct PinDigits(Zeroizing<[u8; 4]>);

impl PinDigits {
    /// The digits as typed; a value over 9 is not a digit and is kept as 9.
    pub fn new(digits: [u8; 4]) -> Self {
        Self(Zeroizing::new(digits.map(|digit| digit.min(9))))
    }

    /// The digits, borrowed: nothing is copied out of the wiped holder.
    pub fn digits(&self) -> &[u8; 4] {
        &self.0
    }

    /// The digits in their wiped holder, for `Lock::verify(Zeroizing<[u8; 4]>)`.
    pub fn into_zeroizing(self) -> Zeroizing<[u8; 4]> {
        self.0
    }

    /// Puts `digit` at place `at`; a place past the fourth is ignored. The pad types with it.
    pub(crate) fn put(&mut self, at: usize, digit: Digit) {
        if let Some(place) = self.0.get_mut(at) {
            *place = digit.value();
        }
    }

    /// Wipes every digit to 0.
    pub(crate) fn wipe(&mut self) {
        self.0.zeroize();
    }
}

impl PartialEq for PinDigits {
    /// Every digit is looked at whatever the first ones are.
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0_u8, |differ, (a, b)| differ | (a ^ b))
            == 0
    }
}

impl Eq for PinDigits {}

impl fmt::Debug for PinDigits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PinDigits(****)")
    }
}

/// What the system must do because of a tap.
#[derive(Clone, PartialEq, Eq)]
pub enum Intent {
    /// The main gate: open for a while, or shut.
    Gate(GateMove),
    /// One device's ads: `pass: true` lets them pass for good (`ALLOW ADS`, strong), `false` blocks them again.
    DeviceAds {
        /// The device.
        mac: Mac,
        /// Whether its ads are to pass.
        pass: bool,
    },
    /// One device's own gate, for `secs` seconds; 0 shuts it.
    DeviceGate {
        /// The device.
        mac: Mac,
        /// How long it stays open; 0 shuts it.
        secs: u16,
    },
    /// Dismiss the alert with this store id: it was seen.
    Ack(u64),
    /// One name's verdict.
    Name {
        /// The name.
        name: String,
        /// What the owner decided.
        verdict: Verdict,
    },
    /// The night hours, as hours of the day.
    NightHours {
        /// When the dark begins.
        from: u8,
        /// When it ends.
        to: u8,
    },
    /// Sleep now: the night clock, until a touch.
    SleepNow,
    /// Join Wi-Fi again.
    Rejoin,
    /// Restart the board. Strong.
    Restart,
    /// Set the PIN, which turns the lock on. Strong: with a PIN already set, it is a new one.
    SetPin(PinDigits),
    /// Turn the lock off. Strong.
    ClearPin,
    /// The four digits typed on the PIN pad, to be checked against the PIN: the answer goes back to the pad through
    /// `nav::verified`. Never asked for itself: it is how the asking is answered.
    Verify(PinDigits),
}

impl Intent {
    /// Whether the action asks for the PIN when the lock is on (spec §7.1): `ALLOW ADS` on a device,
    /// `ALLOW NAME` and `ANSWER AGAIN`, `RESTART`, `TURN OFF` the lock and a new PIN.
    ///
    /// Never asked: whatever makes the house stricter (`BLOCK ADS`, `REFUSE NAME` and `REFUSE AGAIN`, shutting
    /// a gate), dismissing an alert, opening the main gate or a device's gate, night hours, `SLEEP NOW`
    /// and `REJOIN`.
    pub const fn is_strong(&self) -> bool {
        match self {
            Self::DeviceAds { pass, .. } => *pass,
            Self::Name { verdict, .. } => {
                matches!(verdict, Verdict::Allow | Verdict::ClearRefuse)
            }
            Self::Restart | Self::SetPin(_) | Self::ClearPin => true,
            Self::Gate(_)
            | Self::DeviceGate { .. }
            | Self::Ack(_)
            | Self::NightHours { .. }
            | Self::SleepNow
            | Self::Rejoin
            | Self::Verify(_) => false,
        }
    }
}

impl fmt::Debug for Intent {
    /// The kind of intent and what is no one's secret (seconds, hours, an alert's id, a verdict); never a
    /// device, a name or a digit.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gate(gate_move) => f.debug_tuple("Gate").field(gate_move).finish(),
            Self::DeviceAds { pass, .. } => f
                .debug_struct("DeviceAds")
                .field("pass", pass)
                .finish_non_exhaustive(),
            Self::DeviceGate { secs, .. } => f
                .debug_struct("DeviceGate")
                .field("secs", secs)
                .finish_non_exhaustive(),
            Self::Ack(id) => f.debug_tuple("Ack").field(id).finish(),
            Self::Name { verdict, .. } => f
                .debug_struct("Name")
                .field("verdict", verdict)
                .finish_non_exhaustive(),
            Self::NightHours { from, to } => f
                .debug_struct("NightHours")
                .field("from", from)
                .field("to", to)
                .finish(),
            Self::SleepNow => f.write_str("SleepNow"),
            Self::Rejoin => f.write_str("Rejoin"),
            Self::Restart => f.write_str("Restart"),
            Self::SetPin(_) => f.write_str("SetPin(..)"),
            Self::ClearPin => f.write_str("ClearPin"),
            Self::Verify(_) => f.write_str("Verify(..)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Mac = [2, 0, 0, 0, 0, 12];

    /// Every intent there is, with the answer §7.1 gives for it.
    fn every_intent() -> Vec<(Intent, bool)> {
        let name = |verdict| Intent::Name {
            name: "ads.example.org".into(),
            verdict,
        };
        vec![
            // Strong: they loosen the house, or are the lock itself.
            (
                Intent::DeviceAds {
                    mac: MAC,
                    pass: true,
                },
                true,
            ),
            (name(Verdict::Allow), true),
            (name(Verdict::ClearRefuse), true),
            (Intent::Restart, true),
            (Intent::ClearPin, true),
            (Intent::SetPin(PinDigits::new([1, 2, 3, 4])), true),
            // Never asked: stricter, temporary or only a reading.
            (
                Intent::DeviceAds {
                    mac: MAC,
                    pass: false,
                },
                false,
            ),
            (name(Verdict::Refuse), false),
            (name(Verdict::ClearAllow), false),
            (Intent::Gate(GateMove::Open(300)), false),
            (Intent::Gate(GateMove::Open(1800)), false),
            (Intent::Gate(GateMove::Open(3600)), false),
            (Intent::Gate(GateMove::Shut), false),
            (
                Intent::DeviceGate {
                    mac: MAC,
                    secs: 300,
                },
                false,
            ),
            (Intent::DeviceGate { mac: MAC, secs: 0 }, false),
            (Intent::Ack(7), false),
            (Intent::NightHours { from: 23, to: 7 }, false),
            (Intent::SleepNow, false),
            (Intent::Rejoin, false),
            // The check of typed digits is never asked for: it is how the asking is answered.
            (Intent::Verify(PinDigits::new([1, 2, 3, 4])), false),
        ]
    }

    #[test]
    fn strong_is_exactly_the_list_of_spec_7_1() {
        for (intent, strong) in every_intent() {
            assert_eq!(intent.is_strong(), strong, "{intent:?}");
        }
    }

    #[test]
    fn the_strong_ones_are_the_four_of_the_spec() {
        // ALLOW ADS; ALLOW NAME and ANSWER AGAIN; RESTART; TURN OFF and a new PIN.
        let strong: Vec<String> = every_intent()
            .into_iter()
            .filter(|(intent, _)| intent.is_strong())
            .map(|(intent, _)| format!("{intent:?}"))
            .collect();
        assert_eq!(strong.len(), 6, "{strong:?}");
        for word in ["DeviceAds", "Name", "Restart", "ClearPin", "SetPin"] {
            assert!(strong.iter().any(|said| said.starts_with(word)), "{word}");
        }
    }

    #[test]
    fn debug_never_says_a_device_a_name_or_a_digit() {
        let intents = [
            Intent::DeviceAds {
                mac: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                pass: true,
            },
            Intent::DeviceGate {
                mac: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                secs: 1800,
            },
            Intent::Name {
                name: "secret-tracker.example.org".into(),
                verdict: Verdict::Allow,
            },
            Intent::SetPin(PinDigits::new([4, 9, 7, 3])),
            Intent::Verify(PinDigits::new([4, 9, 7, 3])),
        ];
        for intent in intents {
            let said = format!("{intent:?}");
            for secret in [
                "secret", "example", "170", "187", "255", "[", "4973", "4, 9",
            ] {
                assert!(!said.contains(secret), "{said} holds {secret}");
            }
        }
        assert_eq!(
            format!("{:?}", PinDigits::new([1, 2, 3, 4])),
            "PinDigits(****)"
        );
        // What is no secret is said.
        assert_eq!(
            format!("{:?}", Intent::Gate(GateMove::Open(1800))),
            "Gate(Open(1800))"
        );
        assert_eq!(format!("{:?}", Intent::Ack(9)), "Ack(9)");
    }

    #[test]
    fn digits_are_kept_as_digits() {
        assert_eq!(PinDigits::new([1, 2, 3, 4]).digits(), &[1, 2, 3, 4]);
        assert_eq!(PinDigits::new([0, 10, 255, 9]).digits(), &[0, 9, 9, 9]);
    }

    #[test]
    fn digits_compare_by_value_and_leave_in_a_wiped_holder() {
        assert_eq!(PinDigits::new([1, 2, 3, 4]), PinDigits::new([1, 2, 3, 4]));
        for other in [[1, 2, 3, 5], [0, 2, 3, 4], [4, 3, 2, 1], [1, 2, 4, 3]] {
            assert_ne!(PinDigits::new([1, 2, 3, 4]), PinDigits::new(other));
        }
        let wiped = PinDigits::new([5, 6, 7, 8]).into_zeroizing();
        assert_eq!(*wiped, [5, 6, 7, 8]);
        // A single digit is kept as a digit and prints none.
        assert_eq!(Digit::new(200).value(), 9);
        assert_eq!(format!("{:?}", Digit::new(4)), "Digit(*)");
    }
}
