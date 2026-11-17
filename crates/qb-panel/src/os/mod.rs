//! The panel as a small OS (spec `2026-11-12-qberrypi-panel-os-design.md`): what the screens read
//! (`world`), where the panel is and what a tap does (`nav`, `intent`), the screens themselves
//! (`screens`) and the mockup's house as a world for tests and the preview (`fixture`). Pure, like the
//! rest of the crate: the caller fills a `World`, and the screens draw from it.

pub mod fixture;
pub mod intent;
pub mod nav;
pub mod screens;
pub mod world;

pub use intent::{GateMove, Intent, PinDigits, Verdict};
pub use nav::{Change, IDLE_MS, Nav, Screen, Section, Tap, Whole, decide, idle, refresh, tap};
pub use world::World;
