//! The strings the panel shows, made from its values. An unknown value is a
//! dash, never a zero, and no string can outgrow its place.

use crate::geometry::Weight;
use crate::state::TimeOfDay;
use crate::text::has_glyph;

/// An unknown value: an em dash alone, with no unit.
pub const DASH: &str = "\u{2014}";

/// A counter in at most 7 characters: `10,818`, then `1.9M` and `4.2B` (cut, not rounded).
pub fn count(n: u32) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{},{:03}", n / 1_000, n % 1_000),
        1_000_000..1_000_000_000 => format!("{}.{}M", n / 1_000_000, n / 100_000 % 10),
        _ => format!("{}.{}B", n / 1_000_000_000, n / 100_000_000 % 10),
    }
}

/// A size in a few characters, for a tile's figure: `912`, `1.2K`, `212K`, `1.2M` (cut, not
/// rounded), the way the mockup writes the lists' `212K` names.
pub fn short_count(n: u32) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{}.{}K", n / 1_000, n / 100 % 10),
        10_000..1_000_000 => format!("{}K", n / 1_000),
        1_000_000..1_000_000_000 => format!("{}.{}M", n / 1_000_000, n / 100_000 % 10),
        _ => format!("{}.{}B", n / 1_000_000_000, n / 100_000_000 % 10),
    }
}

/// Seconds as `m:ss`, from `0:00` to `99:59`: an hour's gate says `60:00`. `countdown` is the Drain's key and
/// stops at an hour; this one is for the gate's own lengths, 5, 30 and 60 minutes, and runs to `99:59`.
pub fn mss(secs: u16) -> String {
    let secs = secs.min(99 * 60 + 59);
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// The longest gate there is, in seconds: an hour. Nothing the panel opens is longer, and `countdown`
/// stops there.
pub const GATE_LONGEST_SECS: u16 = 3_600;

/// What is left of a pause as `m:ss` for the Drain's key, never more than `60:00`: the gate opens for 5, 30
/// or 60 minutes (the key is 80 px wide, and `60:00` in 24 px bold is 72).
pub fn countdown(left_secs: u16) -> String {
    mss(left_secs.min(GATE_LONGEST_SECS))
}

/// How many steps the paused gate refills in.
const GATE_FILL_STEPS: u32 = 19;

/// The most steps the refill shows: the last one is the gate full, and a pause that is still running is never that.
const GATE_FILL_MAX: u32 = GATE_FILL_STEPS - 1;

/// How full the paused gate is, as steps of 12 pixels from the floor, 0 to 18, when `left_secs` of a pause of
/// `total_secs` are left.
///
/// The gate is hollow when the pause starts and refills from the floor as it runs out:
/// `floor((total - left) * 19 / total)`, against the pause's own length, so that 5, 30 and 60 minutes all
/// refill from empty. It is 0 for a full pause or more (a pause longer than its total counts as full) and
/// stops at 18 for none left. A total of 0 is taken as 1 s.
pub fn gate_fill(left_secs: u16, total_secs: u16) -> u8 {
    let total = u32::from(total_secs).max(1);
    let gone = total.saturating_sub(u32::from(left_secs));
    let steps = (u64::from(gone) * u64::from(GATE_FILL_STEPS) / u64::from(total))
        .min(u64::from(GATE_FILL_MAX));
    u8::try_from(steps).unwrap_or(u8::MAX)
}

/// A DNS name in `cells` cells, as the lane shows it.
///
/// It is lower-case, with one trailing dot off, and every character that is
/// not printable ASCII is a `?`. A name longer than `cells` is cut so that a
/// `~` is the last cell. Only the first `cells + 1` characters are read, so a
/// name of any length costs the same.
pub fn fit_name(name: &str, cells: usize) -> String {
    let name = name.strip_suffix('.').unwrap_or(name);
    cut(
        name.chars().map(|c| printable(c.to_ascii_lowercase())),
        cells,
    )
}

/// The character itself if it is printable ASCII and not a space, else `?`.
fn printable(c: char) -> char {
    if ('!'..='~').contains(&c) { c } else { '?' }
}

/// A label or a line of words in at most `cells` cells: its case and its spaces are kept, every control
/// character and every character the font lacks is a `?`, and a text that is longer is cut so that a
/// `~` is the last cell. Leading and trailing spaces go. For what the owner typed (a device's label)
/// and what the panel's own code says; a DNS name is `fit_name` or `fit_site`.
pub fn fit_text(text: &str, cells: usize) -> String {
    let shown = |c: char| {
        c == ' '
            || c.is_ascii_graphic()
            || (!c.is_ascii() && !c.is_control() && has_glyph(Weight::Bold, c))
    };
    cut(
        text.trim().chars().map(|c| if shown(c) { c } else { '?' }),
        cells,
    )
}

/// The time as the clock says it: `10:21`, or `--:--` while the clock is not set.
pub fn clock(time: Option<TimeOfDay>) -> String {
    match time {
        Some(TimeOfDay { hour, minute }) if hour < 24 && minute < 60 => {
            format!("{hour:02}:{minute:02}")
        }
        _ => "--:--".to_string(),
    }
}

/// The first `cells` characters, or fewer with a `~` last if there are more.
fn cut(chars: impl Iterator<Item = char>, cells: usize) -> String {
    let mut kept: Vec<char> = chars.take(cells.saturating_add(1)).collect();
    if kept.len() > cells {
        kept.truncate(cells.saturating_sub(1));
        if cells > 0 {
            kept.push('~');
        }
    }
    kept.into_iter().collect()
}

/// How many characters of a name `fit_site` and `wrap_name` look at, counted from its end. A DNS name is at
/// most 253, so a longer one is not a name, and the end of it is what says whose it is.
const READ_CHARS: usize = 256;

/// The last `limit` characters of a name, with the one trailing dot off, in the lane's alphabet:
/// lower-case, and a `?` for every character that is not printable ASCII.
fn tail_of(name: &str, limit: usize) -> Vec<char> {
    let name = name.strip_suffix('.').unwrap_or(name);
    let mut chars: Vec<char> = name
        .chars()
        .rev()
        .take(limit)
        .map(|c| printable(c.to_ascii_lowercase()))
        .collect();
    chars.reverse();
    chars
}

/// How many labels make the site of a name, whose labels are `labels`: three when the name ends in a
/// two-letter suffix under a short second level (`example.co.uk`), else two.
fn site_labels(labels: &[&[char]]) -> usize {
    match labels {
        [.., second, last] if labels.len() >= 3 && last.len() == 2 && second.len() <= 3 => 3,
        _ => 2,
    }
}

/// A name in at most `cells` cells, with its site kept whole where it can be.
///
/// The end of a DNS name says whose it is, so host labels go first, whole, from the left, until the
/// name fits or only the site is left: its last two labels, or three under a suffix like `co.uk`.
/// A site that is still too long is cut at its end, with a `~` last. The name is first made as
/// `fit_name` makes it: lower-case, with one trailing dot off and a `?` for every character that is
/// not printable ASCII. Only its last 256 characters are read, so a name of any length costs the same.
pub fn fit_site(name: &str, cells: usize) -> String {
    let chars = tail_of(name, READ_CHARS);
    let labels: Vec<&[char]> = chars.split(|&c| c == '.').collect();
    // The length of the labels from `first` on, joined by dots.
    let mut joined = chars.len();
    let mut first = 0;
    loop {
        let rest = labels.get(first..).unwrap_or_default();
        let Some(leading) = rest.first() else { break };
        if joined <= cells || rest.len() <= site_labels(rest) {
            break;
        }
        joined = joined.saturating_sub(leading.len() + 1);
        first += 1;
    }
    let kept: Vec<char> = labels.get(first..).unwrap_or_default().join(&'.');
    cut(kept.into_iter(), cells)
}

/// A subject on one or two lines of `cells` cells, broken after a dot.
///
/// A name that fits is one line, and the second is empty. A longer one, to twice the room, breaks
/// after the last dot at or before the end of the first line that leaves room on the second, or at
/// the end of the first line when there is no such dot. Past twice the room the start goes and a `~`
/// leads. The name is made as `fit_name` makes it; only its last `2 * cells + 1` characters are read.
pub fn wrap_name(name: &str, cells: usize) -> [String; 2] {
    if cells == 0 {
        return [String::new(), String::new()];
    }
    let whole = cells.saturating_mul(2);
    let mut chars = tail_of(name, whole.saturating_add(1));
    if chars.len() <= cells {
        return [chars.into_iter().collect(), String::new()];
    }
    if chars.len() > whole {
        // One character too many was read: the name is longer than the two lines.
        let from = chars.len() - (whole - 1);
        chars = std::iter::once('~')
            .chain(chars.into_iter().skip(from))
            .collect();
    }
    // The first break after a dot that leaves the second line no more than `cells`, looking back from the end of the first line.
    let least = chars.len() - cells;
    let at = (least..=cells)
        .rev()
        .find(|&at| chars.get(at.wrapping_sub(1)) == Some(&'.'))
        .unwrap_or(cells);
    let (head, tail) = chars.split_at(at.min(chars.len()));
    [head.iter().collect(), tail.iter().collect()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_get_separators_then_a_unit() {
        for (n, s) in [
            (0, "0"),
            (999, "999"),
            (1000, "1,000"),
            (4780, "4,780"),
            (999_999, "999,999"),
            (1_000_000, "1.0M"),
            (1_949_999, "1.9M"),
            (999_999_999, "999.9M"),
            (1_000_000_000, "1.0B"),
            (u32::MAX, "4.2B"),
        ] {
            assert_eq!(count(n), s);
        }
    }

    #[test]
    fn a_count_never_outgrows_seven_characters() {
        // Every decade boundary of a `u32` and its two neighbours, and the largest value.
        let mut edges = vec![u32::MAX];
        for power in 0..=9 {
            let decade = 10_u32.pow(power);
            edges.extend([decade - 1, decade, decade + 1]);
        }
        for n in edges {
            assert!(count(n).chars().count() <= 7, "{n}: {}", count(n));
        }
    }

    #[test]
    fn a_countdown_is_minutes_and_seconds_and_stops_at_sixty() {
        for (s, t) in [
            (299, "4:59"),
            (300, "5:00"),
            (277, "4:37"),
            (60, "1:00"),
            (1, "0:01"),
            (0, "0:00"),
            (599, "9:59"),
            (600, "10:00"),
            (1_800, "30:00"),
            (3_599, "59:59"),
            (3_600, "60:00"),
            (3_601, "60:00"),
            (u16::MAX, "60:00"),
        ] {
            assert_eq!(countdown(s), t);
        }
    }

    /// The fill steps of a pause of `total` seconds, for every second left from `total` down to 0.
    fn fills(total: u16) -> Vec<u8> {
        (0..=total)
            .rev()
            .map(|left| gate_fill(left, total))
            .collect()
    }

    #[test]
    fn gate_fill_steps_for_a_five_minute_pause() {
        // A full pause is an empty gate, and each step is a nineteenth of it.
        assert_eq!(gate_fill(300, 300), 0);
        assert_eq!(
            gate_fill(u16::MAX, 300),
            0,
            "more than a pause is none of it gone"
        );
        assert_eq!(gate_fill(299, 300), 0);
        assert_eq!(gate_fill(285, 300), 0, "15 s gone is 0.95 of a step");
        assert_eq!(gate_fill(284, 300), 1);
        assert_eq!(gate_fill(277, 300), 1, "the mockup's 4:37");
        assert_eq!(gate_fill(150, 300), 9);
        assert_eq!(gate_fill(1, 300), 18);
        assert_eq!(
            gate_fill(0, 300),
            18,
            "the last step is as full as it shows"
        );
        // Never more than 18, never backwards: the steps only go up as the time runs out.
        let mut last = 0;
        for fill in fills(300) {
            assert!(fill >= last && fill <= 18, "{fill}");
            last = fill;
        }
        assert_eq!(last, 18);
        assert_eq!(
            fills(300).iter().filter(|&&f| f == 0).count(),
            16,
            "left 285..=300 is the first step"
        );
    }

    #[test]
    fn gate_fill_counts_against_the_pauses_own_total_so_every_length_refills_from_empty() {
        for total in [300_u16, 1_800, 3_600] {
            let fills = fills(total);
            assert_eq!(fills[0], 0, "{total} s: it starts empty");
            assert_eq!(*fills.last().expect("a run"), 18, "{total} s: it ends full");
            assert!(
                fills.windows(2).all(|pair| pair[0] <= pair[1]),
                "{total} s: only up"
            );
            // Every step from 0 to 18 is seen, and a step is a nineteenth of the pause.
            for step in 0..=18_u8 {
                assert!(fills.contains(&step), "{total} s: step {step} is seen");
            }
            assert_eq!(gate_fill(total, total), 0);
            assert_eq!(gate_fill(total / 2, total), 9, "{total} s: halfway");
            assert_eq!(gate_fill(1, total), 18);
            // The first step comes after a nineteenth of the pause has gone, rounded up.
            let first_step = u32::from(total).div_ceil(19);
            assert_eq!(
                gate_fill(
                    u16::try_from(u32::from(total) - first_step + 1).unwrap_or(0),
                    total
                ),
                0
            );
            assert_eq!(
                gate_fill(
                    u16::try_from(u32::from(total) - first_step).unwrap_or(0),
                    total
                ),
                1,
                "{total} s"
            );
        }
        // The same time left is a different fill under a different total: an hour with 5 minutes
        // left is nearly full, five minutes with 5 minutes left is empty.
        assert_eq!(gate_fill(300, 300), 0);
        assert_eq!(gate_fill(300, 1_800), 15);
        assert_eq!(gate_fill(300, 3_600), 17);
        // A pause that runs longer than its total (set by the API) counts as full, a total of 0 as 1 s.
        assert_eq!(gate_fill(3_600, 300), 0);
        assert_eq!(gate_fill(5, 0), 0);
        assert_eq!(gate_fill(0, 0), 18);
        assert_eq!(gate_fill(u16::MAX, u16::MAX), 0);
        assert_eq!(gate_fill(0, u16::MAX), 18);
    }

    #[test]
    fn fit_site_drops_host_labels_then_cuts() {
        // The mockup's examples, at the 15 cells a refused name stops in.
        assert_eq!(
            fit_site("pagead2.googlesyndication.com", 15),
            "googlesyndicat~"
        );
        assert_eq!(fit_site("rubiconproject.com", 15), "rubiconproject~");
        assert_eq!(fit_site("doubleclick.net", 15), "doubleclick.net");
        assert_eq!(fit_site("scorecardresearch.com", 15), "scorecardresea~");
        // Host labels go whole, from the left, and the site is what is read.
        assert_eq!(
            fit_site("a.very.long.tracker.example.org", 20),
            "tracker.example.org"
        );
        assert_eq!(
            fit_site("a.very.long.tracker.example.org", 15),
            "example.org"
        );
        assert_eq!(fit_site("one.example.org", 20), "one.example.org");
        assert_eq!(fit_site("one.example.org", 15), "one.example.org");
        assert_eq!(fit_site("one.example.org", 11), "example.org");
        // A name that fits is left alone, one cell over it is not.
        assert_eq!(fit_site("abcdefghij.example.c", 20), "abcdefghij.example.c");
        assert_eq!(
            fit_site("abcdefghij.example.co", 20),
            "example.co",
            "21 cells: the host goes"
        );
        // Under a two-letter suffix with a short second level the site is three labels.
        assert_eq!(fit_site("a.b.example.co.uk", 13), "example.co.uk");
        assert_eq!(fit_site("a.b.example.co.uk", 15), "b.example.co.uk");
        assert_eq!(fit_site("a.b.example.co.uk", 20), "a.b.example.co.uk");
        assert_eq!(fit_site("a.b.c.example.co.uk", 15), "c.example.co.uk");
        assert_eq!(
            fit_site("a.b.example.co.uk", 10),
            "example.c~",
            "the site is cut at its end"
        );
        assert_eq!(fit_site("a.b.shop.co.uk", 8), "shop.co~");
        // A suffix of more than two letters, or a second level of four or more under one of two, is
        // not three labels: only the last two are the site.
        assert_eq!(fit_site("a.b.example.com", 11), "example.com");
        assert_eq!(fit_site("a.b.example.de", 10), "example.de");
        assert_eq!(
            fit_site("a.b.exam.co.uk", 5),
            "exam~",
            "exam.co.uk is the site, and is cut"
        );
        assert_eq!(fit_site("a.b.c.d.example.com.au", 14), "example.com.au");
        // The name is made as `fit_name` makes it first: lower-case, one trailing dot off, `?` for the odd.
        assert_eq!(fit_site("WWW.Example.ORG.", 11), "example.org");
        assert_eq!(
            fit_site("b\u{fc}cher.example.org", 20),
            "b?cher.example.org"
        );
        assert_eq!(fit_site("a\u{7}b c\u{1F4A9}d", 20), "a?b?c?d");
        assert_eq!(fit_site("a.b..", 20), "a.b.", "a second trailing dot stays");
        // Names with no dots, or only dots, have no host labels to drop.
        assert_eq!(fit_site("localhost", 15), "localhost");
        assert_eq!(
            fit_site(&"a".repeat(253), 15),
            format!("{}~", "a".repeat(14))
        );
        assert_eq!(fit_site("..", 15), ".", "one trailing dot goes");
        assert_eq!(fit_site("", 15), "");
        assert_eq!(
            fit_site(".", 15),
            "",
            "a lone dot is the root: nothing to show"
        );
        // The extremes of the cell count.
        assert_eq!(fit_site("abc.example", 0), "");
        assert_eq!(fit_site("abc.example", 1), "~");
        assert_eq!(fit_site("a", 1), "a");
    }

    #[test]
    fn fit_site_never_exceeds_its_cells_and_costs_a_bounded_amount() {
        // A DNS name at its longest, in labels of every length, in odd alphabets, and at every width.
        let mut names = vec![
            "a".repeat(253),
            "a.".repeat(126) + "b",
            format!(
                "{}.{}.{}.{}",
                "x".repeat(63),
                "y".repeat(63),
                "z".repeat(63),
                "w".repeat(61)
            ),
            format!("{}.co.uk", "q".repeat(250)),
            "\u{1F4A9}".repeat(253),
            format!("{}.example.co.uk", "h.".repeat(120)),
            ".".repeat(300),
            "a.b.c.example.co.uk".to_string(),
            "co.uk".to_string(),
            "x.co.uk".to_string(),
            "x.y.co.uk".to_string(),
        ];
        // Names with no site worth the name at all, and a megabyte of them.
        names.push("é".repeat(1 << 20));
        names.push(format!("{}.example.org", "m".repeat(1 << 20)));
        for cells in [0, 1, 2, 5, 14, 15, 16, 20, 26] {
            for name in &names {
                let fitted = fit_site(name, cells);
                assert!(
                    fitted.chars().count() <= cells,
                    "{cells} cells: {}",
                    fitted.len()
                );
                assert!(fitted.chars().all(|c| ('!'..='~').contains(&c)));
            }
        }
        // The end of a name is what is read, even when the name is longer than any name can be.
        assert_eq!(
            fit_site(&format!("{}.example.org", "m".repeat(1 << 20)), 15),
            "example.org"
        );
        // Every prefix of a long, dotted name, at both widths.
        let long = "ab.cde.fg.hijk.example.co.uk";
        for len in 0..=long.len() {
            for cells in [15, 20] {
                assert!(fit_site(&long[..len], cells).chars().count() <= cells);
            }
        }
    }

    #[test]
    fn wrap_name_breaks_after_a_dot_and_costs_a_bounded_amount() {
        let wrap = |name: &str| wrap_name(name, 26);
        assert_eq!(
            wrap("secure-login-verify.top"),
            ["secure-login-verify.top", ""]
        );
        assert_eq!(
            wrap("secure-login-verify.example.org"),
            ["secure-login-verify.", "example.org"]
        );
        // A name of any length costs the same, and keeps its end.
        let huge = format!("{}.example.org", "x".repeat(1 << 20));
        let [first, second] = wrap(&huge);
        assert_eq!(first, format!("~{}", "x".repeat(25)));
        assert_eq!(second, format!("{}.example.org", "x".repeat(14)));
        // 52 cells is two full lines; 53 loses its start.
        assert_eq!(wrap(&"y".repeat(52)), ["y".repeat(26), "y".repeat(26)]);
        assert_eq!(
            wrap(&"y".repeat(53)),
            [format!("~{}", "y".repeat(25)), "y".repeat(26)]
        );
        assert_eq!(wrap("a.b..").concat(), "a.b.", "one trailing dot goes");
        assert_eq!(wrap("W\u{fc}.EXAMPLE.org"), ["w?.example.org", ""]);
        assert_eq!(wrap_name("abc", 0), [String::new(), String::new()]);
        assert_eq!(wrap_name("abc", 1), ["~", "c"]);
    }

    #[test]
    fn fit_text_keeps_case_and_spaces_and_cuts_with_a_tilde() {
        assert_eq!(fit_text("Living room TV", 24), "Living room TV");
        assert_eq!(
            fit_text("  Sam's phone  ", 24),
            "Sam's phone",
            "the ends are trimmed"
        );
        assert_eq!(fit_text("Sam's phone", 8), "Sam's p~");
        assert_eq!(fit_text("abc", 3), "abc");
        assert_eq!(fit_text("abcd", 3), "ab~");
        assert_eq!(fit_text("abc", 0), "");
        assert_eq!(fit_text("", 5), "");
        // A control character is a `?`; a letter the font has stays, one it lacks is a `?`.
        assert_eq!(fit_text("a\tb\nc", 24), "a?b?c");
        assert_eq!(fit_text("Caf\u{e9}", 24), "Caf\u{e9}");
        assert_eq!(fit_text("a\u{1f600}b", 24), "a?b");
        // Any length costs the same cells.
        assert_eq!(fit_text(&"x".repeat(1 << 16), 24).chars().count(), 24);
    }

    #[test]
    fn the_clock_says_the_time_or_dashes() {
        let at = |hour, minute| clock(Some(TimeOfDay { hour, minute }));
        assert_eq!(at(10, 21), "10:21");
        assert_eq!(at(0, 5), "00:05");
        assert_eq!(at(23, 59), "23:59");
        assert_eq!(clock(None), "--:--");
        assert_eq!(at(24, 0), "--:--");
        assert_eq!(at(1, 60), "--:--");
    }

    #[test]
    fn a_tiles_figure_is_short_and_a_gates_time_goes_to_an_hour() {
        for (n, s) in [
            (0, "0"),
            (912, "912"),
            (1_000, "1.0K"),
            (1_234, "1.2K"),
            (9_999, "9.9K"),
            (10_000, "10K"),
            (212_000, "212K"),
            (999_999, "999K"),
            (1_000_000, "1.0M"),
            (1_250_000, "1.2M"),
            (u32::MAX, "4.2B"),
        ] {
            assert_eq!(short_count(n), s);
        }
        for (secs, s) in [
            (0, "0:00"),
            (59, "0:59"),
            (277, "4:37"),
            (300, "5:00"),
            (1_800, "30:00"),
            (3_600, "60:00"),
            (u16::MAX, "99:59"),
        ] {
            assert_eq!(mss(secs), s);
        }
    }
}
