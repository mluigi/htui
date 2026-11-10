//! The report's statistics (MOD-162 T2, P7): ranks, recall@k and MRR, Wilson intervals, the exact
//! McNemar test, AUC and recall at zero false alarms, percentiles. Pure functions, no I/O.

use crate::types::CorpusId;

/// z for the 95% Wilson interval.
pub const WILSON_Z: f64 = 1.96;

/// 1-based rank of the first entry of `ranked` that is one of `targets`; `None` when none is.
pub fn first_target_rank(ranked: &[CorpusId], targets: &[CorpusId]) -> Option<usize> {
    let _ = (ranked, targets);
    None
}

/// `rank <= k`.
pub fn hit_at(rank: Option<usize>, k: usize) -> bool {
    let _ = (rank, k);
    false
}

/// `1 / rank` when `rank <= cutoff`, else 0 (MRR@cutoff summand).
pub fn reciprocal_rank(rank: Option<usize>, cutoff: usize) -> f64 {
    let _ = (rank, cutoff);
    -1.0
}

/// Wilson score interval (low, high), clamped to [0, 1]; `n == 0` gives (0.0, 1.0).
pub fn wilson_interval(successes: usize, n: usize) -> (f64, f64) {
    let _ = (successes, n);
    (-1.0, -1.0)
}

/// Exact two-sided McNemar: `min(1, 2 · P[X <= min(b, c)])`, X ~ Bin(b + c, 1/2); `b + c == 0` gives 1.
/// Computed as t0 = 0.5^n, t(i+1) = t(i)·(n−i)/(i+1), summed in f64 (no overflow for n ≤ 1000).
pub fn mcnemar_exact(only_a: usize, only_b: usize) -> f64 {
    let _ = (only_a, only_b);
    -1.0
}

/// Mann-Whitney AUC of positives over negatives, a tie counting one half; `None` if either is empty.
pub fn auc(positives: &[f64], negatives: &[f64]) -> Option<f64> {
    let _ = (positives, negatives);
    None
}

/// Fraction of positives strictly above every negative; `None` when positives is empty; an empty
/// negatives slice makes the threshold −∞.
pub fn recall_at_zero_false_alarms(positives: &[f64], negatives: &[f64]) -> Option<f64> {
    let _ = (positives, negatives);
    None
}

/// Nearest-rank percentile (p in 0..=100) of a copy sorted with `f64::total_cmp`; `None` if empty.
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let _ = (values, p);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `got` equals `want` to `want`'s third significant digit: off by less than half a unit in
    /// that digit (`0.000244` accepts `0.00024414…`; an exact `0.03125` needs `0.03125 ± 5e-5`).
    fn three_digits(got: f64, want: f64) -> bool {
        if want == 0.0 {
            return got == 0.0;
        }
        let unit = 10_f64.powf(want.abs().log10().floor() - 2.0);
        (got - want).abs() <= unit / 2.0
    }

    /// `got` rounds to `want` at two decimal places.
    fn two_places(got: f64, want: f64) -> bool {
        ((got * 100.0).round() - want * 100.0).abs() < 1e-9
    }

    fn ids(keys: &[&str]) -> Vec<CorpusId> {
        keys.iter().map(|k| CorpusId::new(*k)).collect()
    }

    #[test]
    fn mcnemar_matches_ana37s_p_values() {
        for (only_a, only_b, want) in [
            (13, 0, 0.000244),
            (15, 0, 0.000061),
            (6, 0, 0.03125),
            (5, 0, 0.0625),
            (8, 5, 0.581),
            (22, 5, 0.00151),
        ] {
            let got = mcnemar_exact(only_a, only_b);
            assert!(
                three_digits(got, want),
                "mcnemar_exact({only_a}, {only_b}) = {got}, want {want}"
            );
        }
    }

    #[test]
    fn mcnemar_without_discordant_pairs_is_one() {
        assert_eq!(mcnemar_exact(0, 0), 1.0);
    }

    #[test]
    fn mcnemar_never_exceeds_one() {
        // Balanced discordance: 2 · P[X <= 5] for Bin(10, 1/2) is above 1 before the cap.
        assert_eq!(mcnemar_exact(5, 5), 1.0);
        assert_eq!(mcnemar_exact(1, 0), 1.0);
    }

    #[test]
    fn mcnemar_is_symmetric() {
        for (b, c) in [(13, 0), (8, 5), (22, 5), (0, 7), (3, 4), (500, 480)] {
            assert_eq!(mcnemar_exact(b, c), mcnemar_exact(c, b), "({b}, {c})");
        }
    }

    #[test]
    fn mcnemar_stays_finite_at_a_thousand_pairs() {
        let p = mcnemar_exact(1000, 0);
        assert!(p > 0.0 && p < 1e-290, "{p}");
        let q = mcnemar_exact(520, 480);
        assert!(q.is_finite() && q > 0.0 && q < 1.0, "{q}");
    }

    #[test]
    fn wilson_matches_ana37s_intervals() {
        for (successes, n, want_low, want_high) in [
            (22, 24, 0.74, 0.98),
            (9, 24, 0.21, 0.57),
            (47, 60, 0.66, 0.87),
        ] {
            let (low, high) = wilson_interval(successes, n);
            assert!(
                two_places(low, want_low) && two_places(high, want_high),
                "wilson_interval({successes}, {n}) = ({low}, {high}), want ({want_low}, {want_high})"
            );
        }
    }

    #[test]
    fn wilson_of_nothing_is_the_whole_range() {
        assert_eq!(wilson_interval(0, 0), (0.0, 1.0));
    }

    #[test]
    fn wilson_stays_within_zero_and_one() {
        let (low, high) = wilson_interval(0, 5);
        assert_eq!(low, 0.0);
        assert!(high > 0.0 && high < 1.0);
        let (low, high) = wilson_interval(5, 5);
        assert!(low > 0.0 && low < 1.0);
        assert!(high <= 1.0);
        for (s, n) in [(0, 1), (1, 1), (3, 7), (24, 24)] {
            let (low, high) = wilson_interval(s, n);
            assert!((0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high));
            assert!(low <= high);
        }
    }

    #[test]
    fn auc_counts_ties_as_one_half() {
        assert_eq!(auc(&[3.0, 2.0], &[1.0]), Some(1.0));
        assert_eq!(auc(&[1.0], &[1.0]), Some(0.5));
        assert_eq!(auc(&[1.0, 3.0], &[2.0]), Some(0.5));
        assert_eq!(auc(&[1.0], &[2.0, 3.0]), Some(0.0));
    }

    #[test]
    fn auc_needs_both_sides() {
        assert_eq!(auc(&[], &[1.0]), None);
        assert_eq!(auc(&[1.0], &[]), None);
    }

    #[test]
    fn recall_at_zero_false_alarms_is_strictly_above_every_negative() {
        let got = recall_at_zero_false_alarms(&[0.9, 0.5, 0.8], &[0.7]).expect("positives");
        assert!((got - 2.0 / 3.0).abs() < 1e-12, "{got}");
        assert_eq!(recall_at_zero_false_alarms(&[0.5], &[0.5]), Some(0.0));
        assert_eq!(
            recall_at_zero_false_alarms(&[0.9, 0.2], &[0.1, 0.95]),
            Some(0.0)
        );
    }

    #[test]
    fn recall_at_zero_false_alarms_edges() {
        assert_eq!(recall_at_zero_false_alarms(&[], &[0.5]), None);
        assert_eq!(recall_at_zero_false_alarms(&[0.1, -3.0], &[]), Some(1.0));
    }

    #[test]
    fn first_target_rank_is_one_based() {
        let ranked = ids(&["MOD-1", "DEC:ANA-4", "R-STO-8", "MOD-2"]);
        assert_eq!(first_target_rank(&ranked, &ids(&["MOD-1"])), Some(1));
        assert_eq!(
            first_target_rank(&ranked, &ids(&["R-STO-8", "MOD-2"])),
            Some(3)
        );
        assert_eq!(
            first_target_rank(&ranked, &ids(&["MOD-2", "DEC:ANA-4"])),
            Some(2)
        );
        assert_eq!(first_target_rank(&ranked, &ids(&["ANA-4"])), None);
        assert_eq!(first_target_rank(&ranked, &[]), None);
        assert_eq!(first_target_rank(&[], &ids(&["MOD-1"])), None);
    }

    #[test]
    fn hit_at_is_rank_within_k() {
        assert!(hit_at(Some(1), 1));
        assert!(hit_at(Some(5), 5));
        assert!(!hit_at(Some(6), 5));
        assert!(!hit_at(None, 10));
    }

    #[test]
    fn reciprocal_rank_is_cut_off() {
        assert_eq!(reciprocal_rank(Some(1), 10), 1.0);
        assert_eq!(reciprocal_rank(Some(4), 10), 0.25);
        assert_eq!(reciprocal_rank(Some(10), 10), 0.1);
        assert_eq!(reciprocal_rank(Some(11), 10), 0.0);
        assert_eq!(reciprocal_rank(None, 10), 0.0);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let values: Vec<f64> = (1..=20).rev().map(f64::from).collect();
        assert_eq!(percentile(&values, 50.0), Some(10.0));
        assert_eq!(percentile(&values, 95.0), Some(19.0));
        assert_eq!(percentile(&values, 100.0), Some(20.0));
        assert_eq!(percentile(&values, 0.0), Some(1.0));
        assert_eq!(percentile(&[7.5], 50.0), Some(7.5));
        assert_eq!(percentile(&[], 50.0), None);
    }
}
