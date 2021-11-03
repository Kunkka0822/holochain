use crate::PeerStrat;
use crate::*;
use rand::thread_rng;
use rand::Rng;
use std::collections::HashSet;
use std::iter;

/// Maximum number of iterations. If we iterate this much, we assume the
/// system is divergent (unable to reach equilibrium).
const MAX_ITERS: usize = 80;

/// Number of consecutive rounds of no movement before declaring convergence.
const CONVERGENCE_WINDOW: usize = 3;

/// Level of detail in reporting.
const DETAIL: u8 = 0;

fn full_len() -> f64 {
    2f64.powi(32)
}

#[test]
fn only_change_one() {
    observability::test_run().ok();
    let n = 1000;
    let j = 0.0;
    // let j = 1f64 / n as f64 / 100.0;
    let s = ArcLenStrategy::Constant(0.1);

    let run = |check_gaps| {
        let strat = PeerStratAlpha {
            check_gaps,
            redundancy_target: 50,
            ..Default::default()
        }
        .into();

        let mut peers = simple_parameterized_generator(n, j, s);
        peers[0].half_length = MAX_HALF_LENGTH;
        let dynamic = Some(maplit::hashset![0]);
        let vergence = determine_vergence(|| {
            let stats = run_one_epoch(&strat, &mut peers, dynamic.as_ref(), DETAIL);
            tracing::debug!("{}", peers[0].coverage());
            stats
        });
        // print_arcs(&peers);
        report(&vergence);
        vergence
    };

    assert!(matches!(run(true), Vergence::Divergent(_)));
    assert!(matches!(run(false), Vergence::Convergent(_)));
}

#[test]
fn parameterized_stability_test() {
    let n = 1000;
    let j = 1f64 / n as f64 / 3.0;
    let s = ArcLenStrategy::Constant(0.1);

    let r = 50;
    let strat = PeerStratAlpha {
        redundancy_target: r,
        ..Default::default()
    };
    let kind = PeerStrat::Alpha(strat);

    let mut peers = simple_parameterized_generator(n, j, s);
    tracing::info!("{}", EpochStats::oneline_header());
    let vergence = determine_vergence(|| {
        let stats = run_one_epoch(&kind, &mut peers, None, DETAIL);
        tracing::info!("{}", stats.oneline());
        stats
    });
    report(&vergence);
    vergence.assert_convergent();
}

#[test]
fn min_redundancy_is_maintained() {
    todo!("Check that min redundancy is maintained at all times");
}

fn report(stats: &Vergence) {
    if let Vergence::Convergent(stats) = stats {
        tracing::info!("{:?}", stats.last().unwrap());
        tracing::info!("Reached equilibrium in {} iterations", stats.len());
    } else {
        tracing::error!("failed to reach equilibrium in {} iterations", MAX_ITERS);
    }
}

/// Run iterations until there is no movement of any arc
/// TODO: this may be unreasonable, and we may need to just ensure that arcs
/// settle down into a reasonable level of oscillation
fn determine_vergence<F>(mut step: F) -> Vergence
where
    F: FnMut() -> EpochStats,
{
    let mut n_delta_count = 0;
    let mut history = vec![];
    for i in 1..=MAX_ITERS {
        if n_delta_count >= CONVERGENCE_WINDOW {
            tracing::info!("Converged in {} iterations", i - CONVERGENCE_WINDOW);
            break;
        }

        let stats = step();
        if stats.gross_delta_avg == 0.0 {
            n_delta_count += 1;
        } else if n_delta_count > 0 {
            panic!("we don't expect a system in equilibirum to suddenly start moving again!")
        } else {
            history.push(stats);
        }
    }
    if n_delta_count == 0 {
        Vergence::Divergent(history)
    } else {
        Vergence::Convergent(history)
    }
}

/// Resize every arc based on neighbors' arcs, and compute stats about this iteration
/// kind: The resizing strategy to use
/// peers: The list of peers in this epoch
/// dynamic_peer_indices: Indices of peers who should be updated. If None, all peers will be updated.
/// detail: Level of output detail. More is more verbose. detail: u8,
fn run_one_epoch(
    kind: &PeerStrat,
    peers: &mut Vec<DhtArc>,
    dynamic_peer_indices: Option<&HashSet<usize>>,
    detail: u8,
) -> EpochStats {
    let mut net = 0.0;
    let mut gross = 0.0;
    let mut delta_min = full_len() / 2.0;
    let mut delta_max = -full_len() / 2.0;
    let mut index_min = peers.len();
    let mut index_max = peers.len();
    for i in 0..peers.len() {
        if let Some(dynamic) = dynamic_peer_indices {
            if !dynamic.contains(&i) {
                continue;
            }
        }
        let p = peers.clone();
        let arc = peers.get_mut(i).unwrap();
        let bucket = DhtArcBucket::new(*arc, p.clone());
        let density = bucket.peer_view(kind);
        let before = arc.absolute_length() as f64;
        arc.update_length(density);
        let after = arc.absolute_length() as f64;
        let delta = after - before;
        net += delta;
        gross += delta.abs();
        if delta < delta_min {
            delta_min = delta;
            index_min = i;
        }
        if delta > delta_max {
            delta_max = delta;
            index_max = i;
        }
    }

    if detail == 1 {
        tracing::info!("min: |{}| {}", peers[index_min].to_ascii(64), index_min);
        tracing::info!("max: |{}| {}", peers[index_max].to_ascii(64), index_max);
        tracing::info!("");
    } else if detail == 2 {
        print_arcs(peers);
        get_input();
    }

    let tot = peers.len() as f64;
    let min_redundancy = check_redundancy(peers.clone());
    EpochStats {
        net_delta_avg: net / tot / full_len(),
        gross_delta_avg: gross / tot / full_len(),
        min_redundancy: min_redundancy,
        delta_min: delta_min / full_len(),
        delta_max: delta_max / full_len(),
    }
}

/// Generate a list of DhtArcs based on 3 parameters:
/// N: total # of peers
/// J: random jitter of peer locations
/// S: strategy for generating arc lengths
fn simple_parameterized_generator(n: usize, j: f64, s: ArcLenStrategy) -> Vec<DhtArc> {
    tracing::info!("N = {}, J = {}", n, j);
    tracing::info!("Arc len generation: {:?}", s);
    let halflens = s.gen(n);
    generate_evenly_spaced_with_half_lens_and_jitter(j, halflens)
}

/// Define arcs by centerpoint and halflen in the unit interval [0.0, 1.0]
fn unit_arcs<H: Iterator<Item = (f64, f64)>>(arcs: H) -> Vec<DhtArc> {
    let fc = full_len();
    let fh = MAX_HALF_LENGTH as f64;
    arcs.map(|(c, h)| DhtArc::new((c * fc).min(u32::MAX as f64) as u32, (h * fh) as u32))
        .collect()
}

/// Each agent is perfect evenly spaced around the DHT,
/// with the halflens specified by the iterator.
fn generate_evenly_spaced_with_half_lens_and_jitter<H: Iterator<Item = f64>>(
    jitter: f64,
    hs: H,
) -> Vec<DhtArc> {
    let mut rng = thread_rng();
    let hs: Vec<_> = hs.collect();
    let n = hs.len() as f64;
    unit_arcs(hs.into_iter().enumerate().map(|(i, h)| {
        (
            (i as f64 / n) + (2.0 * jitter * rng.gen::<f64>()) - jitter,
            h,
        )
    }))
}

#[derive(Debug)]
enum Vergence {
    Convergent(Vec<EpochStats>),
    Divergent(Vec<EpochStats>),
}

impl Vergence {
    pub fn assert_convergent(&self) {
        assert!(
            matches!(self, Self::Convergent(_)),
            "failed to reach equilibrium in {} iterations",
            MAX_ITERS
        )
    }

    pub fn assert_divergent(&self) {
        assert!(
            matches!(self, Self::Divergent(_)),
            "sequence was expected to diverge, but converged",
        )
    }
}

#[derive(Debug)]
struct EpochStats {
    net_delta_avg: f64,
    gross_delta_avg: f64,
    delta_max: f64,
    delta_min: f64,
    // delta_variance: f64,
    min_redundancy: u32,
}

impl EpochStats {
    pub fn oneline_header() -> String {
        format!("rdun   net Δ%   gross Δ%   min Δ%   max Δ%")
    }

    pub fn oneline(&self) -> String {
        format!(
            "{:4}   {:>+6.3}   {:>8.3}   {:>6.3}   {:>6.3}",
            self.min_redundancy,
            self.net_delta_avg * 100.0,
            self.gross_delta_avg * 100.0,
            self.delta_min * 100.0,
            self.delta_max * 100.0,
        )
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum ArcLenStrategy {
    Random,
    Constant(f64),
    HalfAndHalf(f64, f64),
}

impl ArcLenStrategy {
    pub fn gen(&self, num: usize) -> Box<dyn Iterator<Item = f64>> {
        match self {
            Self::Random => {
                let mut rng = thread_rng();
                Box::new(iter::repeat_with(move || rng.gen()).take(num))
            }
            Self::Constant(v) => Box::new(iter::repeat(*v).take(num)),
            Self::HalfAndHalf(a, b) => Box::new(
                iter::repeat(*a)
                    .take(num / 2)
                    .chain(iter::repeat(*b).take(num / 2)),
            ),
        }
    }
}

/// View ascii for all arcs
fn print_arcs(arcs: &Vec<DhtArc>) {
    for (i, arc) in arcs.into_iter().enumerate() {
        println!("|{}| {}", arc.to_ascii(64), i);
    }
}

/// Wait for input, to slow down overwhelmingly large iterations
fn get_input() {
    let mut input_string = String::new();
    std::io::stdin()
        .read_line(&mut input_string)
        .ok()
        .expect("Failed to read line");
}
