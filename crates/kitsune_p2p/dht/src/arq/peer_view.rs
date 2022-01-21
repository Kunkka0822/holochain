use kitsune_p2p_dht_arc::ArcInterval;

use super::{is_full, Arq, ArqBounded, ArqBounds, ArqSet, ArqStrat};

pub struct PeerView {
    /// The strategy which generated this view
    strat: ArqStrat,
    /// The peers in this view (TODO: replace with calculated values)
    peers: ArqSet,

    #[cfg(feature = "testing")]
    /// Omit the arq at this index from all peer considerations.
    /// Useful for tests which update all arqs, without needing to
    /// construct a new PeerView for each arq needing to be updated
    pub skip_index: Option<usize>,
}

impl PeerView {
    pub fn new(strat: ArqStrat, arqs: ArqSet) -> Self {
        Self {
            strat,
            peers: arqs,
            #[cfg(feature = "testing")]
            skip_index: None,
        }
    }

    /// The actual coverage of all arcs in this view.
    /// TODO: this only makes sense when the view contains all agents in the DHT.
    ///       So, it's more useful for testing. Probably want to tease out some
    ///       concept of a test DHT from this.
    pub fn actual_coverage(&self) -> f64 {
        actual_coverage(self.peers.iter())
    }

    /// Extrapolate the coverage of the entire network from our local view.
    pub fn extrapolated_coverage(&self, filter: &ArqBounds) -> f64 {
        self.extrapolated_coverage_and_filtered_count(filter).0
    }

    /// Return the extrapolated coverage and the number of arqs which match the filter.
    /// These two are complected together simply for efficiency's sake, to
    /// minimize computation
    ///
    /// TODO: this probably will be rewritten when PeerView is rewritten to
    /// have the filter baked in.
    pub fn extrapolated_coverage_and_filtered_count(&self, filter: &ArqBounds) -> (f64, usize) {
        let filter = filter.to_interval();
        if filter == ArcInterval::Empty {
            // More accurately this would be 0, but it's handy to not have
            // divide-by-zero crashes
            return (1.0, 1);
        }
        let filter_len = filter.length();

        let initial = (0, 0);

        // FIXME: We can't just filter arcs on the fly here, because we might be
        // trying to get coverage info for an area we don't have arcs for
        // (because we don't store arcs for agents outside of our arc).
        // So, we need to extrapolate the arcs we do have to extend into the
        // unknown area outside the filter.
        // For now though, just filter arcs on the fly so we have something to test.
        // But, this means that the behavior for growing arcs is going to be a bit
        // different in the future.
        let (sum, count) = self
            .filtered_arqs(filter)
            .fold(initial, |(sum, count), arq| (sum + arq.length(), count + 1));
        let cov = sum as f64 / filter_len as f64;
        (cov, count)
    }

    /// Compute the total coverage observed within the filter interval.
    pub fn raw_coverage(&self, filter: &ArqBounds) -> f64 {
        self.extrapolated_coverage(filter) * filter.to_interval().length() as f64 / 2f64.powf(32.0)
    }

    pub fn update_arq(&self, arq: &mut Arq) -> bool {
        self.update_arq_with_stats(arq).changed
    }

    /// Take an arq and potentially resize and requantize it based on this view.
    ///
    /// This represents an iterative step towards the ideal coverage, based on
    /// the observed coverage.
    /// This makes many assumptions, including:
    /// - this arc resizing algorithm is a good one, namely that the coverage
    ///     at any point of the DHT is close to the target range
    /// - all other peers are following the same algorithm
    /// - if we see a change that we need to make, we assume that a number of
    ///     peers are about to make a similar change, and that number is on
    ///     average the same as our target coverage
    ///
    /// More detail on these assumptions here:
    /// https://hackmd.io/@hololtd/r1IAIbr5Y/https%3A%2F%2Fhackmd.io%2FK_fkBj6XQO2rCUZRRL9n2g
    pub fn update_arq_with_stats(&self, arq: &mut Arq) -> UpdateArqStats {
        let (cov, num_peers) = self.extrapolated_coverage_and_filtered_count(&arq.to_bounds());

        let old_count = arq.count();
        let old_power = arq.power();
        let under = cov < self.strat.min_coverage;
        let over = cov > self.strat.max_coverage();

        // The ratio of ideal coverage vs actual observed coverage.
        // A ratio > 1 indicates undersaturation and a need to grow.
        // A ratio < 1 indicates oversaturation and a need to shrink.
        // We cap the growth at 2x in either direction
        let cov_ratio = (self.strat.midline_coverage() / cov).clamp(0.5, 2.0);

        // We want to know which of our peers are likely to be making a similar
        // update to us, because that will affect the overall coverage more
        // than the drop in the bucket that we can provide.
        //
        // If all peers have seen the same change as us since their last update,
        // they will on average move similarly to us, and so we should only make
        // a small step in the direction of the target, trusting that our peers
        // will do the same.
        //
        // Conversely, if all peers are stable, e.g. if we just came online to
        // find a situation where all peers around us are under-representing,
        // but stable, then we want to make a much bigger leap.
        let peer_velocity_factor = 1.0 + num_peers as f64;

        let growth_factor = (cov_ratio - 1.0) / peer_velocity_factor + 1.0;

        let new_count = if under {
            // Ensure we grow by at least 1
            (old_count as f64 * growth_factor).ceil() as u32
        } else if over {
            // Ensure we shrink by at least 1
            (old_count as f64 * growth_factor).floor() as u32
        } else {
            // No change if between the min and max target coverage
            old_count
        };

        if new_count != old_count {
            let mut tentative = arq.clone();
            tentative.count = new_count;

            // If shrinking caused us to go below the target coverage,
            // don't update. This happens when we shink too much and
            // lose sight of peers.
            let new_cov = self.extrapolated_coverage(&tentative.to_bounds());
            if over && (new_cov < self.strat.min_coverage) {
                return UpdateArqStats {
                    changed: false,
                    power: None,
                    num_peers,
                };
            }
        }

        // Commit the change to the count
        arq.count = new_count;

        let power_stats = self.power_stats(&arq);
        let PowerStats { median, .. } = power_stats;

        let power_above_min = |pow| {
            // not already at the minimum
            pow > self.strat.min_power
             // don't power down if power is already too low
             && (median as i8 - pow as i8) < self.strat.max_power_diff as i8
        };

        #[allow(unused_parens)]
        loop {
            // check for power downshift opportunity
            if arq.count < self.strat.min_chunks() {
                if power_above_min(arq.power) {
                    *arq = arq.downshift();
                } else {
                    arq.count = self.strat.min_chunks();
                }
            } else {
                break;
            }
        }

        let power_below_max = |pow| {
            // not already at the maximum
            pow < self.strat.max_power
            // don't power up if power is already too high
            && (pow as i8 - median as i8) < self.strat.max_power_diff as i8
        };

        #[allow(unused_parens)]
        loop {
            // check for power upshift opportunity
            if arq.count > self.strat.max_chunks() {
                if power_below_max(arq.power) {
                    // Attempt to requantize to the next higher power.
                    // If we only grew by one chunk, into an odd count, then don't
                    // force upshifting, because that would either require undoing
                    // the growth, or growing by 2 instead of 1. In this case, skip
                    // upshifting, and we'll upshift on the next update.
                    let force = new_count as i32 - old_count as i32 > 1;
                    if let Some(a) = arq.upshift(force) {
                        *arq = a
                    } else {
                        break;
                    }
                } else {
                    // If we could not upshift due to other constraints, then we cannot
                    // grow any larger than the max_chunks.
                    arq.count = self.strat.max_chunks();
                }
            } else {
                break;
            }
        }

        if is_full(arq.power(), arq.count()) {
            *arq = Arq::new_full(arq.center(), arq.power());
        }

        // check if anything changed
        let changed = !(arq.power() == old_power && arq.count() == old_count);

        UpdateArqStats {
            changed,
            power: Some(power_stats),
            num_peers,
        }
    }

    pub fn power_stats(&self, filter: &Arq) -> PowerStats {
        use statrs::statistics::*;
        let mut powers: Vec<_> = self
            .filtered_arqs(filter.to_interval())
            .filter(|a| a.count > 0)
            .map(|a| a.power as f64)
            .collect();
        powers.push(filter.power as f64);
        let powers = statrs::statistics::Data::new(powers);
        let median = powers.median() as u8;
        let std_dev = powers.std_dev().unwrap_or_default();
        if std_dev > self.strat.power_std_dev_threshold {
            // tracing::warn!("Large power std dev: {}", std_dev);
        }
        PowerStats { median, std_dev }
    }

    fn filtered_arqs<'a>(&'a self, filter: ArcInterval) -> impl Iterator<Item = &'a Arq> {
        let it = self.peers.arqs.iter();

        #[cfg(feature = "testing")]
        let it = it
            .enumerate()
            .filter(|(i, _)| self.skip_index.as_ref() != Some(i))
            .map(|(_, arq)| arq);

        it.filter(move |arq| filter.contains(arq.center))
    }
}

pub struct UpdateArqStats {
    pub changed: bool,
    pub power: Option<PowerStats>,
    pub num_peers: usize,
}

/// The actual coverage provided by these peers. Assumes that this is the
/// entire view of the DHT, all peers are accounted for here.
pub fn actual_coverage<'a, A: 'a, P: Iterator<Item = &'a A>>(peers: P) -> f64
where
    ArqBounds: From<&'a A>,
{
    peers
        .map(|a| ArqBounds::from(a).length() as f64 / 2f64.powf(32.0))
        .sum()
}

pub struct PowerStats {
    pub median: u8,
    pub std_dev: f64,
}

#[cfg(test)]
mod tests {

    use kitsune_p2p_dht_arc::ArcInterval;

    use crate::arq::pow2;
    use crate::Loc;

    use super::*;

    fn make_arq(pow: u8, lo: u32, hi: u32) -> Arq {
        ArqBounds::from_interval_rounded(
            pow,
            ArcInterval::new(pow2(pow) * lo, (pow2(pow) as u64 * hi as u64) as u32),
        )
        .to_arq()
    }

    #[test]
    fn test_filtered_arqs() {
        let pow = 25;
        let a = make_arq(pow, 0, 0x20);
        let b = make_arq(pow, 0x10, 0x30);
        let c = make_arq(pow, 0x20, 0x40);
        assert_eq!(a.center, Loc::from(pow2(pow) * 0x10 - 1));
        assert_eq!(b.center, Loc::from(pow2(pow) * 0x20 - 1));
        assert_eq!(c.center, Loc::from(pow2(pow) * 0x30 - 1));
        let arqs = ArqSet::new(vec![a, b, c]);
        arqs.print_arqs(64);
        let view = PeerView::new(Default::default(), arqs);

        let get = |b: Arq| {
            view.filtered_arqs(b.to_interval())
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(get(make_arq(pow, 0, 0x10)), vec![a]);
        assert_eq!(get(make_arq(pow, 0, 0x20)), vec![a, b]);
        assert_eq!(get(make_arq(pow, 0, 0x40)), vec![a, b, c]);
        assert_eq!(get(make_arq(pow, 0x10, 0x20)), vec![b]);
    }

    #[test]
    fn test_coverage() {
        let pow = 24;
        let arqs = ArqSet::new(
            (0..0x100)
                .step_by(0x10)
                .map(|x| make_arq(pow, x, x + 0x20))
                .collect(),
        );
        arqs.print_arqs(64);
        let view = PeerView::new(Default::default(), arqs);
        assert_eq!(
            view.extrapolated_coverage_and_filtered_count(&make_arq(pow, 0, 0x10).to_bounds()),
            (2.0, 1)
        );
        assert_eq!(
            view.extrapolated_coverage_and_filtered_count(&make_arq(pow, 0, 0x20).to_bounds()),
            (2.0, 2)
        );
        assert_eq!(
            view.extrapolated_coverage_and_filtered_count(&make_arq(pow, 0, 0x40).to_bounds()),
            (2.0, 4)
        );

        // TODO: when changing PeerView logic to bake in the filter,
        // this will probably change
        assert_eq!(
            view.extrapolated_coverage_and_filtered_count(&make_arq(pow, 0x10, 0x20).to_bounds()),
            (2.0, 1)
        );
    }
}
