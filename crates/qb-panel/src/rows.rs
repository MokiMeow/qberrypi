//! Which rows of a frame differ from another: the measure of what a screen
//! write costs, because the screen's driver sends one span from the first
//! touched row to the last.

use std::ops::Range;

/// The rows in which `new` differs from `old`, as the first and one past the last, or `None` if it does not.
///
/// The rows between the two may be equal. Slices of different lengths differ in all of `new`'s rows.
/// `old` may be memory that something else writes to while it is read: the range that comes back
/// is then one of rows that existed, never empty or turned round.
///
/// # Panics
///
/// If `stride`, the bytes in a row, is zero.
pub fn changed_rows(old: &[u8], new: &[u8], stride: usize) -> Option<Range<usize>> {
    if old.len() != new.len() {
        return Some(0..new.len().div_ceil(stride));
    }
    let mut rows = old.chunks(stride).zip(new.chunks(stride));
    let first = rows.clone().position(|(was, is)| was != is)?;
    // A last row that differs, as there is a first, unless `old` has changed in between.
    let last = rows
        .rposition(|(was, is)| was != is)
        .map_or(first, |last| last.max(first));
    Some(first..last + 1)
}

/// How many unchanged rows between two changed ones still make one band: a write of its own costs a window
/// to set and a start, which are worth about this many rows of pixels.
pub const BAND_GAP: usize = 8;

/// The rows in which `new` differs from `old`, as bands: runs of rows from the first to one past the last
/// that differs, a run going on over up to `gap` rows that are equal. A write of each band
/// sends fewer rows than the one span of `changed_rows` when two things far apart change together (a lamp
/// and the clock), and the same rows when one thing does.
///
/// None (an empty list) when they do not differ. Slices of different lengths differ in all of `new`'s rows, as one band.
///
/// # Panics
///
/// If `stride`, the bytes in a row, is zero.
pub fn changed_bands(old: &[u8], new: &[u8], stride: usize, gap: usize) -> Vec<Range<usize>> {
    if old.len() != new.len() {
        let all = 0..new.len().div_ceil(stride);
        return vec![all];
    }
    let mut bands: Vec<Range<usize>> = vec![];
    for (row, (was, is)) in old.chunks(stride).zip(new.chunks(stride)).enumerate() {
        if was == is {
            continue;
        }
        match bands.last_mut() {
            Some(band) if row <= band.end + gap => band.end = row + 1,
            _ => bands.push(row..row + 1),
        }
    }
    bands
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::geometry::{HEIGHT, WIDTH};

    /// How many bytes a frame holds: two for each pixel.
    const FRAME_BYTES: usize = WIDTH * HEIGHT * 2;

    /// How many bytes a row holds.
    const ROW_BYTES: usize = WIDTH * 2;

    /// A list of one band of rows.
    fn one(rows: Range<usize>) -> Vec<Range<usize>> {
        vec![rows]
    }

    #[test]
    fn equal_frames_have_no_rows_that_changed() {
        let frame = vec![0x5a; FRAME_BYTES];
        assert_eq!(changed_rows(&frame, &frame, ROW_BYTES), None);
        assert_eq!(changed_rows(&[], &[], ROW_BYTES), None);
    }

    #[test]
    fn the_rows_that_changed_run_from_the_first_to_the_last_that_differ() {
        let old = vec![0; FRAME_BYTES];
        let with = |bytes: &[usize]| {
            let mut new = old.clone();
            for &at in bytes {
                new[at] = 1;
            }
            changed_rows(&old, &new, ROW_BYTES)
        };
        // One pixel of row 10.
        assert_eq!(with(&[10 * ROW_BYTES + 400]), Some(10..11));
        // Rows 3 and 200, and nothing between them.
        assert_eq!(with(&[3 * ROW_BYTES, 200 * ROW_BYTES + 959]), Some(3..201));
        // The first byte and the last byte of the frame.
        assert_eq!(with(&[0]), Some(0..1));
        assert_eq!(with(&[FRAME_BYTES - 1]), Some(319..320));
        assert_eq!(with(&[0, FRAME_BYTES - 1]), Some(0..320));
    }

    #[test]
    fn rows_of_different_lengths_all_count_as_changed() {
        assert_eq!(changed_rows(&[0; 960], &[0; 1920], ROW_BYTES), Some(0..2));
        assert_eq!(changed_rows(&[0; 1920], &[0; 960], ROW_BYTES), Some(0..1));
        assert_eq!(changed_rows(&[0; 960], &[0; 1000], ROW_BYTES), Some(0..2));
        assert_eq!(changed_rows(&[], &[0; 10], ROW_BYTES), Some(0..1));
    }

    #[test]
    fn bands_split_where_the_changes_are_far_apart_and_join_where_they_are_near() {
        let old = vec![0; FRAME_BYTES];
        let with = |rows: &[usize], gap: usize| {
            let mut new = old.clone();
            for &row in rows {
                new[row * ROW_BYTES + 7] = 1;
            }
            changed_bands(&old, &new, ROW_BYTES, gap)
        };
        assert_eq!(with(&[], 8), Vec::<Range<usize>>::new());
        assert_eq!(with(&[10], 8), one(10..11));
        // Rows 10 and 20 have 9 equal rows between them: two bands with a gap of 8, one with 9.
        assert_eq!(with(&[10, 20], 8), [10..11, 20..21]);
        assert_eq!(with(&[10, 20], 9), one(10..21));
        // A lamp, then the clock far above and a countdown far below: three small writes, not one big one.
        assert_eq!(
            with(&[96, 100, 22, 30, 240, 246], 8),
            [22..31, 96..101, 240..247]
        );
        // The whole span of `changed_rows` is the first band's start to the last band's end.
        let rows = [96, 100, 22, 30, 240, 246];
        let mut new = old.clone();
        for &row in &rows {
            new[row * ROW_BYTES] = 1;
        }
        assert_eq!(changed_rows(&old, &new, ROW_BYTES), Some(22..247));
        // A gap as long as the frame is the one span.
        assert_eq!(with(&rows, usize::MAX / 2), one(22..247));
        assert_eq!(with(&[0, 319], 8), [0..1, 319..320]);
    }

    #[test]
    fn bands_of_frames_of_different_lengths_are_all_of_new() {
        assert_eq!(
            changed_bands(&[0; 960], &[0; 1920], ROW_BYTES, BAND_GAP),
            one(0..2)
        );
        assert_eq!(changed_bands(&[], &[0; 10], ROW_BYTES, BAND_GAP), one(0..1));
    }
}
