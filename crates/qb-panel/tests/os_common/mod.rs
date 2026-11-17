//! What the golden tests share: the metric that compares two frames, the PNG reading and writing,
//! and the flag that writes the references.
//!
//! The comparison has three parts: the mean over the frame, the share of outlier pixels and the
//! worst tile. `drain_golden.rs` and `os_golden.rs` both use it, so a limit means the same in both.

#![allow(dead_code)] // each test binary uses some of these

pub mod traffic;

use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};

use qb_panel::{HEIGHT, WIDTH};

/// A rectangle of the frame: left, top, width, height.
pub type Rect = (usize, usize, usize, usize);

/// The side of a tile of the third part of the metric, in pixels.
const TILE: usize = 16;
/// A pixel is an outlier if a channel of it differs by more than this.
const OUTLIER_STEP: u8 = 48;
/// The tiles in a row of the frame.
const TILES_ACROSS: usize = WIDTH / TILE;
/// The pixels of a frame, as the divisor of a share or a mean.
const FRAME_PIXELS: f64 = (WIDTH * HEIGHT) as f64;
/// The pixels of a tile, as the divisor of a mean.
const TILE_PIXELS: f64 = (TILE * TILE) as f64;
/// The colour channels of a pixel that are compared: R, G and B.
const CHANNELS: f64 = 3.0;

// The tiles cover the frame exactly: 600 of them.
const _: () = assert!(WIDTH.is_multiple_of(TILE) && HEIGHT.is_multiple_of(TILE));

/// How far two frames are apart.
pub struct Diff {
    /// The mean of |a − b| over the R, G and B bytes of all pixels.
    pub mean_abs: f64,
    /// The share of pixels with any channel differing by more than 48.
    pub outliers: f64,
    /// The largest `mean_abs` of one of the 600 tiles of 16 × 16.
    pub worst_tile: f64,
}

/// Compares two 480 × 320 RGBA frames; the alpha is ignored.
pub fn diff(a: &[u8], b: &[u8]) -> Diff {
    let (a, b) = (a.as_chunks::<4>().0, b.as_chunks::<4>().0);
    assert_eq!(
        (a.len(), b.len()),
        (WIDTH * HEIGHT, WIDTH * HEIGHT),
        "two frames of 480 × 320"
    );
    // The sum of |a − b| in each tile; a whole frame of the largest steps stays within `u32`.
    let mut tiles = vec![0_u32; TILES_ACROSS * (HEIGHT / TILE)];
    let mut outliers = 0_u32;
    for (at, (p, q)) in a.iter().zip(b).enumerate() {
        let steps = [
            p[0].abs_diff(q[0]),
            p[1].abs_diff(q[1]),
            p[2].abs_diff(q[2]),
        ];
        let (x, y) = (at % WIDTH, at / WIDTH);
        tiles[y / TILE * TILES_ACROSS + x / TILE] +=
            steps.iter().copied().map(u32::from).sum::<u32>();
        outliers += u32::from(steps.iter().any(|&step| step > OUTLIER_STEP));
    }
    let worst = tiles.iter().copied().max().unwrap_or(0);
    Diff {
        mean_abs: f64::from(tiles.iter().sum::<u32>()) / (FRAME_PIXELS * CHANNELS),
        outliers: f64::from(outliers) / FRAME_PIXELS,
        worst_tile: f64::from(worst) / (TILE_PIXELS * CHANNELS),
    }
}

/// Whether a difference is within the three limits of the golden test.
pub fn close(d: &Diff) -> bool {
    d.mean_abs <= 0.40 && d.outliers <= 0.0003 && d.worst_tile <= 8.0
}

/// How far two frames are apart inside some rectangles of the frame: the three parts of the metric,
/// over the pixels of the rectangles only. A tile is of the frame's grid, and counts only the pixels of
/// the rectangles that fall in it.
pub fn region_diff(a: &[u8], b: &[u8], rects: &[Rect]) -> Diff {
    let inside = |x: usize, y: usize| {
        rects
            .iter()
            .any(|&(l, t, w, h)| (l..l + w).contains(&x) && (t..t + h).contains(&y))
    };
    let mut tiles = vec![0_u32; TILES_ACROSS * (HEIGHT / TILE)];
    let (mut total, mut outliers, mut pixels) = (0_u64, 0_u64, 0_u64);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if !inside(x, y) {
                continue;
            }
            let at = (y * WIDTH + x) * 4;
            let (p, q) = (&a[at..at + 3], &b[at..at + 3]);
            let steps = [
                p[0].abs_diff(q[0]),
                p[1].abs_diff(q[1]),
                p[2].abs_diff(q[2]),
            ];
            let sum = steps.iter().copied().map(u32::from).sum::<u32>();
            tiles[y / TILE * TILES_ACROSS + x / TILE] += sum;
            total += u64::from(sum);
            outliers += u64::from(steps.iter().any(|&step| step > OUTLIER_STEP));
            pixels += 1;
        }
    }
    let pixels = pixels.max(1) as f64;
    Diff {
        mean_abs: total as f64 / (pixels * CHANNELS),
        outliers: outliers as f64 / pixels,
        worst_tile: f64::from(tiles.iter().copied().max().unwrap_or(0)) / (TILE_PIXELS * CHANNELS),
    }
}

/// An 8-bit RGB or RGBA PNG of any size, as RGBA, with its width and height.
pub fn read_png_any(path: impl AsRef<Path>) -> (usize, usize, Vec<u8>) {
    let path = path.as_ref();
    let png = fs::read(path).unwrap_or_else(|e| panic!("{} is read: {e}", path.display()));
    let mut reader = png::Decoder::new(Cursor::new(png))
        .read_info()
        .expect("the file is a PNG");
    let mut raw = vec![0; reader.output_buffer_size().expect("the PNG fits in memory")];
    let frame = reader.next_frame(&mut raw).expect("the PNG decodes");
    let (width, height) = (
        usize::try_from(frame.width).expect("the width fits"),
        usize::try_from(frame.height).expect("the height fits"),
    );
    assert_eq!(
        frame.bit_depth,
        png::BitDepth::Eight,
        "{} is 8-bit",
        path.display()
    );
    let raw = &raw[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgb => raw
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, u8::MAX])
            .collect(),
        png::ColorType::Rgba => raw.to_vec(),
        other => panic!("{} is {other:?}, not RGB or RGBA", path.display()),
    };
    (width, height, rgba)
}

/// An 8-bit RGB PNG as an RGBA frame of 480 × 320.
pub fn read_png(path: impl AsRef<Path>) -> Vec<u8> {
    let path = path.as_ref();
    let (width, height, rgba) = read_png_any(path);
    assert_eq!(
        (width, height),
        (WIDTH, HEIGHT),
        "{} has the frame's size",
        path.display()
    );
    rgba
}

/// Writes an RGBA frame as an 8-bit RGB PNG.
pub fn write_png(path: &Path, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, _]| [r, g, b])
        .collect();
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        u32::try_from(WIDTH)?,
        u32::try_from(HEIGHT)?,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgb)?;
    writer.finish()?;
    Ok(())
}

/// Whether this run is to write the references: `QBP_BLESS` is set to anything.
pub fn blessing() -> bool {
    env::var_os("QBP_BLESS").is_some()
}

/// The mockup's directory, if `QBP_MOCKUP_DIR` names it.
pub fn mockup_dir() -> Option<PathBuf> {
    let dir = env::var_os("QBP_MOCKUP_DIR").map(PathBuf::from);
    if dir.is_none() {
        eprintln!("QBP_MOCKUP_DIR is not set: nothing to compare with");
    }
    dir
}

/// A still of the mockup, a 2x copy of a 480 x 320 frame, as that frame: every second pixel.
pub fn mockup_frame(path: &Path) -> Vec<u8> {
    let (width, height, big) = read_png_any(path);
    assert_eq!(
        (width, height),
        (2 * WIDTH, 2 * HEIGHT),
        "{} is a 2x still",
        path.display()
    );
    let mut frame = vec![0; WIDTH * HEIGHT * 4];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let from = (2 * y * width + 2 * x) * 4;
            let to = (y * WIDTH + x) * 4;
            frame[to..to + 4].copy_from_slice(&big[from..from + 4]);
        }
    }
    frame
}
