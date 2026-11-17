//! The latency figures of a step, as `cargo xtask latency` counts them: percentiles by nearest
//! rank, and a question that got no answer, or a SERVFAIL or REFUSED one, counted as the whole
//! wait, because the client that asked it waited at least that long and got nothing.

/// The latency figures of one step, in microseconds; each `None` without a question to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Latency {
    pub p50_us: Option<u32>,
    pub p95_us: Option<u32>,
    pub p99_us: Option<u32>,
    /// The slowest answer that came; a lost question is not an answer.
    pub max_us: Option<u32>,
}

/// The figures of `received`, the times of the answers that came (sorted here, in place), with
/// `unanswered` more questions counted as `wait_us` each. A percentile is the nearest rank over
/// all of them: the p99 of 1,000 is the 990th smallest. An answer slower than the wait counts as
/// the wait.
pub fn latency(received: &mut [u32], unanswered: usize, wait_us: u32) -> Latency {
    received.sort_unstable();
    let count = received.len() + unanswered;
    let rank = |percent: usize| -> Option<u32> {
        let rank = (percent * count).div_ceil(100);
        let index = rank.checked_sub(1)?;
        Some(match received.get(index) {
            Some(&us) => us.min(wait_us),
            None => wait_us,
        })
    };
    Latency {
        p50_us: rank(50),
        p95_us: rank(95),
        p99_us: rank(99),
        max_us: received.last().copied(),
    }
}

/// Microseconds as milliseconds with three decimals, or `-`.
pub fn ms_text(us: Option<u32>) -> String {
    us.map_or_else(
        || "-".to_owned(),
        |us| format!("{:.3}", f64::from(us) / 1000.0),
    )
}

/// Microseconds as milliseconds, for JSON; `None` stays `None`.
pub fn ms_value(us: Option<u32>) -> Option<f64> {
    us.map(|us| f64::from(us) / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let mut times: Vec<u32> = (1..=1000).rev().collect();
        let figures = latency(&mut times, 0, 2_000_000);
        assert_eq!(
            figures,
            Latency {
                p50_us: Some(500),
                p95_us: Some(950),
                p99_us: Some(990),
                max_us: Some(1000),
            }
        );
    }

    #[test]
    fn an_unanswered_question_counts_as_the_wait() {
        // 98 answers and 2 lost: the 99th of 100 is a lost one.
        let mut times = vec![100; 98];
        let figures = latency(&mut times, 2, 2_000_000);
        assert_eq!(figures.p95_us, Some(100));
        assert_eq!(figures.p99_us, Some(2_000_000));
        assert_eq!(figures.max_us, Some(100));
        // All lost: every percentile is the wait, and there is no slowest answer.
        let all_lost = latency(&mut [], 3, 1_000);
        assert_eq!((all_lost.p50_us, all_lost.max_us), (Some(1_000), None));
        assert_eq!(latency(&mut [], 0, 1_000), Latency::default());
        // An answer over the wait counts as the wait.
        assert_eq!(latency(&mut [5_000], 0, 1_000).p50_us, Some(1_000));
    }

    #[test]
    fn milliseconds_have_three_decimals() {
        assert_eq!(ms_text(Some(1234)), "1.234");
        assert_eq!(ms_text(None), "-");
        assert_eq!(ms_value(Some(250)), Some(0.25));
    }
}
