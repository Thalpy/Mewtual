//! One reporting convention for design 13's measurements.
//!
//! This exists so 13.6 and 13.7 do not each invent their own summary statistic. Every figure
//! either profile reports is a [`Spread`], and the conventions it encodes were all bought by
//! getting them wrong first:
//!
//! - a single averaged figure carries no information about whether a difference between two cases
//!   is real, and re-measuring identical fixtures on this machine has moved by up to 86%;
//! - the middle value is the **upper median**, not an interpolated one, so it never reports a
//!   value the clock did not produce - but that has to be named, because it is not what "median"
//!   unqualified would mean;
//! - a printed `0/1000/1000` is compatible with several different zero counts, so the zero count
//!   is carried separately rather than inferred from the spread;
//! - "nonzero" and "resolved well enough to take a ratio from" are different predicates, and
//!   conflating them printed confident-looking fractions off a single clock tick.

/// Min, **upper median**, max of a sample set, converted to microseconds per unit of work.
///
/// `per` is how many units of work one sample covers: 1 for a phase timed once per trial, or the
/// repetition count for a batched phase - in which case each sample is itself a batch mean, and
/// this is a spread *of means* rather than of individual timings. A batched spread cannot reveal
/// one slow individual operation hidden inside an ordinary batch, which is what a conservative
/// worst-case threshold would eventually need.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::store) struct Spread {
    pub(in crate::store) min_us: u128,
    pub(in crate::store) upper_median_us: u128,
    pub(in crate::store) max_us: u128,
    pub(in crate::store) samples: usize,
    /// How many raw samples read exactly zero milliseconds.
    pub(in crate::store) zero_samples: usize,
    /// Sum of the raw millisecond samples, so a spread of all-zeros is visibly "below the clock's
    /// resolution" rather than "free".
    pub(in crate::store) raw_total_ms: u64,
    /// The upper median of the **raw** millisecond samples, before dividing by `per`.
    ///
    /// Resolution is a property of the clock, so it has to be judged on what the clock actually
    /// read - not on the per-unit figure. For a batch of 64 the per-unit value is 64x smaller
    /// than the sample, so testing the per-unit value against "one tick" demanded a 64 ms batch
    /// and rejected well-resolved 12 ms ones as unresolved.
    pub(in crate::store) raw_upper_median_ms: u64,
}

impl Spread {
    pub(in crate::store) fn of(samples: &[u64], per: u128) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        let mut us: Vec<u128> = samples.iter().map(|ms| *ms as u128 * 1_000 / per).collect();
        us.sort_unstable();
        let mut raw: Vec<u64> = samples.to_vec();
        raw.sort_unstable();
        Self {
            min_us: us[0],
            upper_median_us: us[us.len() / 2],
            max_us: us[us.len() - 1],
            samples: us.len(),
            zero_samples: samples.iter().filter(|ms| **ms == 0).count(),
            raw_total_ms: samples.iter().sum(),
            raw_upper_median_ms: raw[raw.len() / 2],
        }
    }

    /// Resolved *well enough to take a ratio from*, which is stronger than nonzero.
    ///
    /// Judged on the **raw** sample the clock produced, not the per-unit figure derived from it.
    /// A sample of a single tick is not measured to better than 100% of itself: the true value
    /// lies somewhere within one whole millisecond.
    ///
    /// Three rules were tried. Checking the raw *sum* let seven zeros plus one 1 ms sample
    /// through and printed "100% deferrable". Rejecting only a zero median let one-tick medians
    /// through. Testing `upper_median_us > 1_000` fixed that for unbatched phases but was 64x too
    /// strict for batched ones, because dividing by `per` first means a well-resolved 12 ms batch
    /// reports 187 us and fails - so batched phases were being suppressed as "unresolved" when
    /// they were the best-resolved figures in the run.
    ///
    /// **This is a floor, not a precision guarantee.** It admits a two-tick reading, and two
    /// phases both read as 2 ms put the true ratio anywhere in roughly 40 to 60% while 50 is
    /// printed. A ratio drawn from small raw medians is a rough one; only where both phases are
    /// tens of ticks is the printed figure tight.
    pub(in crate::store) fn resolved_for_ratio(&self) -> bool {
        self.raw_upper_median_ms > 1
    }
}

impl std::fmt::Display for Spread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The raw median is printed alongside the per-unit spread so a reader can see what the
        // clock actually read, rather than having to reconstruct it as `us * per / 1000`.
        write!(
            f,
            "{}/{}/{}(z{} raw{}ms)",
            self.min_us,
            self.upper_median_us,
            self.max_us,
            self.zero_samples,
            self.raw_upper_median_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolution is a property of the clock, so a batched phase must be judged on the sample the
    /// clock produced rather than on the per-unit figure derived from it.
    ///
    /// This is a regression test. The predicate used to read `upper_median_us > 1_000`, which is
    /// right for an unbatched phase and 64x too strict for a batch of 64: a 12 ms batch - twelve
    /// clock ticks, among the best-resolved figures in a run - divides down to 187 us and was
    /// reported as *unresolved*, suppressing ratios that were perfectly sound.
    #[test]
    fn a_well_resolved_batch_is_not_reported_as_unresolved() {
        let twelve_ms_batches = [12, 12, 12, 12, 12, 12, 12, 12];
        let batched = Spread::of(&twelve_ms_batches, 64);
        assert_eq!(batched.raw_upper_median_ms, 12);
        assert_eq!(batched.upper_median_us, 187, "12 ms over 64 reps");
        assert!(
            batched.resolved_for_ratio(),
            "a twelve-tick batch is well resolved; only the per-unit figure is small"
        );

        // And the unbatched case the predicate was originally written for still behaves: one tick
        // is not resolved, two are.
        assert!(!Spread::of(&[1, 1, 1, 1], 1).resolved_for_ratio());
        assert!(Spread::of(&[2, 2, 2, 2], 1).resolved_for_ratio());

        // The batched boundary, which the predicate is judged on the raw sample for: a one-tick
        // batch is still unresolved however small the per-unit figure looks, and two ticks pass.
        // Without these the fix could have over-corrected into accepting single-tick batches.
        let one_tick_batch = Spread::of(&[1; 8], 64);
        assert_eq!(one_tick_batch.upper_median_us, 15);
        assert!(
            !one_tick_batch.resolved_for_ratio(),
            "a single-tick batch is not resolved just because dividing by 64 makes it look small"
        );
        assert!(Spread::of(&[2; 8], 64).resolved_for_ratio());

        // The per-unit figure and the raw median must describe the same sample.
        let s = Spread::of(&[3, 5, 9], 10);
        assert_eq!(s.raw_upper_median_ms, 5);
        assert_eq!(s.upper_median_us, 5 * 1_000 / 10);
        // Seven zeros and one nonzero sample: the raw *sum* is positive, which is what an even
        // earlier version wrongly accepted.
        let mostly_zero = Spread::of(&[0, 0, 0, 0, 0, 0, 0, 1], 1);
        assert!(mostly_zero.raw_total_ms > 0);
        assert!(!mostly_zero.resolved_for_ratio());
        assert_eq!(mostly_zero.zero_samples, 7);
    }
}
