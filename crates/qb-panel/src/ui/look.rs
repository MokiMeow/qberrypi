//! The panel's palette and the numbers the OS screens share, from the round 8 mockup (`common.js`,
//! `screens.js` and the "Numbers for building the wall" table of its README).
//!
//! Coordinates are screen pixels on the 480 by 320 frame, y pointing down. The Drain keeps its own
//! numbers in `drain::look` and re-exports the palette from here.

use crate::geometry::Rgb;

// Colours: the round 5 palette, and nothing else.

/// The ground of the screen.
pub const GROUND: Rgb = [0x00, 0x00, 0x00];
/// The accent: refused tiles, the funnel's first shade, the key, the gate.
pub const BLUE: Rgb = [0x4a, 0x72, 0xff];
/// The light blue.
pub const BLUE_LT: Rgb = [0xa4, 0xb6, 0xff];
/// The mid blue.
pub const BLUE_MD: Rgb = [0x31, 0x52, 0xc4];
/// The funnel's second shade.
pub const BLUE_DK: Rgb = [0x1f, 0x33, 0x83];
/// The fill of a plain tile.
pub const BLUE_DEEP: Rgb = [0x12, 0x1b, 0x44];
/// The ink on the lit key.
pub const BLUE_NIGHT: Rgb = [0x0c, 0x12, 0x30];
/// Values, outlines and the paused key.
pub const WHITE: Rgb = [0xff, 0xff, 0xff];
/// The brightest grey: a plain tile's letter, the clock and the paused key's label.
pub const G1: Rgb = [0xd0, 0xd0, 0xd0];
/// The `REFUSED` label.
pub const G2: Rgb = [0x8e, 0x8e, 0x8e];
/// The third grey.
pub const G3: Rgb = [0x5e, 0x5e, 0x5e];
/// The rails the lane runs between.
pub const G4: Rgb = [0x2e, 0x2e, 0x2e];
/// The darkest grey.
pub const G5: Rgb = [0x18, 0x18, 0x18];

// Text is set glyph by glyph, as the mockup's `tx` does.

/// JetBrains Mono's advance, as a share of the size.
pub const ADVANCE_EM: f32 = 0.6;

// The lane (spec 3.1): the status bar on every screen but LIVE and the whole-system states.

/// The width of a lane cell: one letter of the lane's 24 px bold.
pub const LANE_CELL: f32 = 24.0;
/// The size of the lane's letters, in bold.
pub const LANE_PX: f32 = 24.0;
/// The lane's letters' baseline.
pub const LANE_BASELINE: f32 = 45.0;
/// The first cell of the system's two words on HOME, and of the section's word elsewhere.
pub const LANE_WORD_CELL: u8 = 5;
/// The first cell of the clock, which takes five.
pub const LANE_CLOCK_CELL: u8 = 15;
/// The top rail's top.
pub const RAIL_TOP_Y: u16 = 4;
/// The bottom rail's top.
pub const RAIL_BOTTOM_Y: u16 = 64;
/// The thickness of both rails, and of the lit rail.
pub const RAIL_H: u16 = 4;
/// The width of the lit rail under `HOME`.
pub const LIT_HOME_W: u16 = 96;
/// The left end of the lit rail under a section's word.
pub const LIT_WORD_X: u16 = 120;
/// The count tile's left edge: a white tile for the alerts that wait.
pub const COUNT_TILE_X: u16 = 313;
/// The count tile's top.
pub const COUNT_TILE_Y: u16 = 16;
/// The count tile's width.
pub const COUNT_TILE_W: u16 = 22;
/// The count tile's height.
pub const COUNT_TILE_H: u16 = 40;
/// The count tile's digit's centre.
pub const COUNT_TILE_DIGIT_X: f32 = 324.0;
/// The count tile's digit's baseline.
pub const COUNT_TILE_DIGIT_BASELINE: f32 = 44.0;

/// The lane's tap targets: a press above this row is on the lane.
pub const LANE_TARGET_Y: u16 = 72;
/// The lane's tap targets: `HOME` is every press left of this column.
pub const LANE_HOME_X: u16 = 108;
/// The lane's tap targets: the clock, which opens SYSTEM, is every press from this column on.
pub const LANE_CLOCK_X: u16 = 348;

// HOME (spec 3.2, `homes.js`): six tiles, three across and two down.

/// How many tiles HOME has.
pub const HOME_TILES: usize = 6;
/// How many tiles stand in a row of HOME.
pub const HOME_COLUMNS: usize = 3;
/// The left edge of HOME's first tile.
pub const HOME_TILE_X0: u16 = 8;
/// The top of HOME's first row.
pub const HOME_TILE_Y0: u16 = 80;
/// The distance between two tiles' left edges.
pub const HOME_TILE_PITCH_X: u16 = 157;
/// The distance between two rows' tops.
pub const HOME_TILE_PITCH_Y: u16 = 116;
/// A tile's width.
pub const HOME_TILE_W: u16 = 150;
/// A tile's height.
pub const HOME_TILE_H: u16 = 108;
/// A tap target of HOME's tiles starts at this row (the tile's top less a margin); `wallHit`'s 76.
pub const HOME_HIT_Y0: u16 = 76;
/// A tap target of HOME's tiles starts at this column (the first tile's left less a margin); `wallHit`'s 4.
pub const HOME_HIT_X0: u16 = 4;

// The slab and the floor (spec 3.5, `screens.js`).

/// The slab's left edge.
pub const SLAB_X: u16 = 8;
/// The slab's top.
pub const SLAB_Y: u16 = 80;
/// The slab's width.
pub const SLAB_W: u16 = 464;
/// The slab's height.
pub const SLAB_H: u16 = 152;

/// The floor starts at this row, for the taps: a press from here down is on a key.
pub const FLOOR_Y: u16 = 236;
/// The floor's left key is every press left of this column; the right key the rest.
pub const FLOOR_SPLIT_X: u16 = 240;
/// The floor's keys' top.
pub const KEY_Y: u16 = 240;
/// The floor's keys' height.
pub const KEY_H: u16 = 64;
/// A floor key's width when there are two.
pub const KEY_W2: u16 = 228;
/// The right key's left edge when there are two.
pub const KEY_X2: u16 = 244;
/// A floor key's left edge when there are four, the first; the others follow at `FOUR_KEYS_PITCH`.
pub const FOUR_KEYS_X0: u16 = 8;
/// The distance between four keys' left edges.
pub const FOUR_KEYS_PITCH: u16 = 118;
/// A floor key's width when there are four.
pub const KEY_W4: u16 = 110;

// DEVICES (spec 3.4, `devicesScreen`): twelve places, four across and three down.

/// How many places the wall of devices has.
pub const WALL_PLACES: usize = 12;
/// How many places stand in a row of the wall.
pub const WALL_COLUMNS: usize = 4;
/// The left edge of the wall's first place.
pub const WALL_X0: u16 = 8;
/// The top of the wall's first row.
pub const WALL_Y0: u16 = 80;
/// The distance between two places' left edges.
pub const WALL_PITCH_X: u16 = 118;
/// The distance between two rows' tops.
pub const WALL_PITCH_Y: u16 = 77;
/// A place's width.
pub const WALL_W: u16 = 110;
/// A place's height.
pub const WALL_H: u16 = 70;
/// A tap target of the wall starts at this column (the first place's left less a margin); the mockup's 4.
pub const WALL_HIT_X0: u16 = 4;
/// A tap target of the wall starts at this row (the first row's top less a margin); the mockup's 76.
pub const WALL_HIT_Y0: u16 = 76;
