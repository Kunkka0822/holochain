use crate::Loc;
use kitsune_p2p_dht_arc::ArcInterval;
use kitsune_p2p_timestamp::Timestamp;

use crate::quantum::{SpaceSegment, SpacetimeCoords, TimeSegment, Topology};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, derive_more::Constructor)]
pub struct RegionCoords {
    pub space: SpaceSegment,
    pub time: TimeSegment,
}

impl RegionCoords {
    /// Map the quantized coordinates into the actual Timestamp and DhtLocation
    /// bounds specifying the region
    pub fn to_bounds(&self, topo: &Topology) -> RegionBounds {
        RegionBounds {
            x: self.space.loc_bounds(topo),
            t: self.time.timestamp_bounds(topo),
        }
    }

    pub fn contains(&self, coords: &SpacetimeCoords) -> bool {
        self.space.contains(coords.space) && self.time.contains(coords.time)
    }
}

#[derive(Debug)]
pub struct RegionBounds {
    pub x: (Loc, Loc),
    pub t: (Timestamp, Timestamp),
}

impl RegionBounds {
    pub fn new(x: (Loc, Loc), t: (Timestamp, Timestamp)) -> Self {
        Self { x, t }
    }

    pub fn contains(&self, x: &Loc, t: &Timestamp) -> bool {
        self.arc_interval().contains(x) && self.time_range().contains(t)
    }

    pub fn arc_interval(&self) -> ArcInterval {
        ArcInterval::new(self.x.0, self.x.1)
    }

    pub fn time_range(&self) -> std::ops::RangeInclusive<Timestamp> {
        self.t.0..=self.t.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // proptest::proptest! {
    //     #[test]
    //     fn region_bounds() {
    //         let topo = Topology::standard_epoch();
    //         let b1 = RegionCoords::new(SpaceSegment::new(27, 38), TimeSegment::new(4, 7412352))
    //             .to_bounds(&topo);
    //             dbg!(b1);
    //         assert!(b1.x.0 != b1.x.1);
    //         assert!(b1.t.0 != b1.t.1);
    //     }

    // }

    #[test]
    fn region_bounds_regressions() {
        use std::str::FromStr;
        let topo = Topology::standard_epoch();
        let b =
            RegionCoords::new(SpaceSegment::new(12, 100), TimeSegment::new(4, 12)).to_bounds(&topo);

        dbg!(&b);
        assert_eq!(b.x.0, 409600.into());
        assert_eq!(b.x.1, 17186815.into());
        assert_eq!(b.t.0, Timestamp::from_str("2022-01-01T16:00:00Z").unwrap());
        assert_eq!(
            b.t.1,
            Timestamp::from_str("2022-01-01T01:08:25.032703Z").unwrap()
        );
    }
}
