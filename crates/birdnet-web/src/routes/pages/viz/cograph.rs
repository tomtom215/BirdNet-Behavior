//! The co-occurrence graph the chord diagram, the ladder matrix and the ranked list
//! all draw, and the order they draw it in.
//!
//! A [`CoGraph`] is species and the strength of every pair: how much of their time two
//! species share. Three pictures of one graph have to agree, so the numbers they print
//! and the order they put the species in come from here and nowhere else.
//!
//! # Why the order is computed
//!
//! Around a ring, a link between two species is a chord, and two chords cross when their
//! four ends interleave. The old chord diagram laid the species out in the order the
//! query ranked them (busiest first), which has nothing to do with who is heard with
//! whom, so its ribbons crossed wherever they happened to. The order is the one thing
//! about a chord diagram a drawer can choose, and it decides whether the picture reads
//! as a few clusters or as a tangle. [`chord_order`] looks for the one with the least
//! weighted crossing, and the same order gives the matrix its blocks.
//!
//! Pure functions, no I/O.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

/// Species and the strength of every pair among them.
#[derive(Clone, Debug, Default)]
pub struct CoGraph {
    /// Display names, busiest species first. The index is the species' colour slot, so a
    /// species keeps its colour however the pictures order it.
    pub names: Vec<String>,
    /// `strength[i][j]` in `0..=1`: the pair's share, relative to the strongest pair
    /// among these species. Symmetric, zero on the diagonal.
    pub strength: Vec<Vec<f64>>,
    /// `share[i][j]` in `0..=1`: the pair's share as measured, the fraction of the
    /// five-minute blocks either was heard in that held both (Jaccard).
    pub share: Vec<Vec<f64>>,
    /// `blocks[i][j]`: five-minute blocks the two were heard in together.
    pub blocks: Vec<Vec<i64>>,
}

/// The step of the sequential ramp, 1 to 7, for a strength `v` in `0..=1`: seven even steps.
///
/// A strength is a proportion of the strongest pair, so (unlike a count, which `kit::seq_step`
/// spaces by its square root) its ramp is linear: with square-root spacing a pair at half the
/// strongest was already step 5 of 7, and a ring of pairs between half and all of the strongest,
/// which is most rings, was three shades of the darkest brick.
#[must_use]
pub fn share_step(v: f64) -> usize {
    ((v.clamp(0.0, 1.0) * 7.0).ceil() as usize).clamp(1, 7)
}

/// One link between two species, `a < b`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    /// The first species (index into [`CoGraph::names`]).
    pub a: usize,
    /// The second species, with `b > a`.
    pub b: usize,
    /// Strength in `0..=1`, relative to the strongest pair.
    pub v: f64,
}

impl CoGraph {
    /// How many species.
    #[must_use]
    pub const fn n(&self) -> usize {
        self.names.len()
    }

    /// Whether the matrices cover every species (a malformed graph draws nothing).
    #[must_use]
    pub fn is_square(&self) -> bool {
        let n = self.n();
        [self.strength.len(), self.share.len(), self.blocks.len()]
            .iter()
            .all(|&l| l >= n)
            && self.strength.iter().take(n).all(|r| r.len() >= n)
            && self.share.iter().take(n).all(|r| r.len() >= n)
            && self.blocks.iter().take(n).all(|r| r.len() >= n)
    }

    /// Every link with any strength, strongest first (ties by index, so the order is
    /// the same on every request).
    #[must_use]
    pub fn ranked_links(&self) -> Vec<Link> {
        if !self.is_square() {
            return Vec::new();
        }
        let n = self.n();
        let mut out = Vec::new();
        for a in 0..n {
            for b in a + 1..n {
                let v = self.strength[a][b];
                if v > 0.0 && v.is_finite() {
                    out.push(Link { a, b, v });
                }
            }
        }
        out.sort_by(|x, y| y.v.total_cmp(&x.v).then(x.a.cmp(&y.a)).then(x.b.cmp(&y.b)));
        out
    }

    /// The `k` strongest links.
    #[must_use]
    pub fn top_links(&self, k: usize) -> Vec<Link> {
        let mut all = self.ranked_links();
        all.truncate(k);
        all
    }
}

/// The weighted number of chord crossings when the species stand in `order` around a
/// ring: for every two links with four distinct ends whose ends interleave, the product
/// of their strengths.
///
/// `order[p]` is the species at position `p`; every species named by a link must be in it.
#[must_use]
pub fn crossing_cost(order: &[usize], links: &[Link]) -> f64 {
    let top = order.iter().copied().max().map_or(0, |m| m + 1);
    let mut pos = vec![usize::MAX; top.max(links.iter().map(|l| l.b + 1).max().unwrap_or(0))];
    for (p, &s) in order.iter().enumerate() {
        pos[s] = p;
    }
    let ends: Vec<(usize, usize, f64)> = links
        .iter()
        .filter(|l| pos[l.a] != usize::MAX && pos[l.b] != usize::MAX)
        .map(|l| (pos[l.a].min(pos[l.b]), pos[l.a].max(pos[l.b]), l.v))
        .collect();
    let mut cost = 0.0;
    for (i, &(a0, a1, wa)) in ends.iter().enumerate() {
        for &(b0, b1, wb) in &ends[i + 1..] {
            // Strictly interleaved: one end of the second strictly inside the first and
            // the other strictly outside. A shared end is not a crossing.
            let inside0 = a0 < b0 && b0 < a1;
            let inside1 = a0 < b1 && b1 < a1;
            if inside0 != inside1 && b0 != a0 && b0 != a1 && b1 != a0 && b1 != a1 {
                cost += wa * wb;
            }
        }
    }
    cost
}

/// What a placement costs: its crossings, then (a tie-break an order of magnitude
/// smaller) how far the strong links reach round the ring, so two species heard together
/// a great deal end up side by side where the crossings do not care.
fn placement_cost(order: &[usize], links: &[Link]) -> f64 {
    let n = order.len().max(1);
    let mut pos = vec![0usize; order.iter().copied().max().map_or(0, |m| m + 1)];
    for (p, &s) in order.iter().enumerate() {
        pos[s] = p;
    }
    let reach: f64 = links
        .iter()
        .filter(|l| l.a < pos.len() && l.b < pos.len())
        .map(|l| {
            let d = pos[l.a].abs_diff(pos[l.b]);
            let d = d.min(n - d);
            l.v * d as f64
        })
        .sum();
    crossing_cost(order, links) + 0.02 * reach / n as f64
}

/// A small deterministic generator (the order must be the same on every request, so the
/// page and its cache agree).
struct Lcg(u64);

impl Lcg {
    const fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// Greedy improvement of one arrangement: swap two species, or move one to another place,
/// whenever that lowers the cost, until nothing does.
fn improve(order: &mut Vec<usize>, links: &[Link]) -> f64 {
    let n = order.len();
    let mut best = placement_cost(order, links);
    for _ in 0..200 {
        let mut improved = false;
        for i in 0..n {
            for j in i + 1..n {
                order.swap(i, j);
                let c = placement_cost(order, links);
                if c + 1e-12 < best {
                    best = c;
                    improved = true;
                } else {
                    order.swap(i, j);
                }
            }
        }
        for i in 0..n {
            for j in 0..n {
                if i == j {
                    continue;
                }
                let s = order.remove(i);
                order.insert(j, s);
                let c = placement_cost(order, links);
                if c + 1e-12 < best {
                    best = c;
                    improved = true;
                } else {
                    order.remove(j);
                    order.insert(i, s);
                }
            }
        }
        if !improved {
            break;
        }
    }
    best
}

/// The order to stand `nodes` in around a ring (or down a ladder) so the links among them
/// cross as little as can be found.
///
/// A local search from several deterministic starting arrangements, keeping the cheapest.
/// It is a search and not a proof: the minimum is NP-hard in general, and for the ten or so
/// species drawn here it is in practice the minimum (the tests hold it to zero on graphs
/// that have a crossing-free order). The result starts at the busiest species and runs
/// towards its stronger neighbour, so the same data always turns the same way.
#[must_use]
pub fn chord_order(nodes: &[usize], links: &[Link]) -> Vec<usize> {
    let n = nodes.len();
    if n <= 2 {
        return nodes.to_vec();
    }
    let in_play: Vec<Link> = links
        .iter()
        .copied()
        .filter(|l| nodes.contains(&l.a) && nodes.contains(&l.b))
        .collect();
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15 ^ (n as u64) ^ ((in_play.len() as u64) << 20));
    let mut starts: Vec<Vec<usize>> = vec![nodes.to_vec()];
    let mut rev = nodes.to_vec();
    rev.reverse();
    starts.push(rev);
    for _ in 0..10 {
        let mut s = nodes.to_vec();
        for i in (1..n).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            s.swap(i, j);
        }
        starts.push(s);
    }
    let mut best: Option<(f64, Vec<usize>)> = None;
    for mut s in starts {
        let c = improve(&mut s, &in_play);
        if best.as_ref().is_none_or(|(bc, _)| c + 1e-12 < *bc) {
            best = Some((c, s));
        }
    }
    let mut order = best.map_or_else(|| nodes.to_vec(), |(_, o)| o);

    // Canonical turn: begin at the species with the most total strength (the first of the
    // input on a tie) and head towards whichever neighbour it is more strongly linked to.
    let weight = |s: usize| -> f64 {
        in_play
            .iter()
            .filter(|l| l.a == s || l.b == s)
            .map(|l| l.v)
            .sum()
    };
    let first = nodes
        .iter()
        .copied()
        .max_by(|&x, &y| weight(x).total_cmp(&weight(y)).then(y.cmp(&x)))
        .unwrap_or(nodes[0]);
    let at = order.iter().position(|&s| s == first).unwrap_or(0);
    order.rotate_left(at);
    let link_to = |s: usize, t: usize| -> f64 {
        in_play
            .iter()
            .find(|l| (l.a == s && l.b == t) || (l.a == t && l.b == s))
            .map_or(0.0, |l| l.v)
    };
    if link_to(first, order[n - 1]) > link_to(first, order[1]) + 1e-12 {
        order[1..].reverse();
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graph of `n` species whose links are exactly `pairs`, each with strength `v`.
    fn graph(n: usize, pairs: &[(usize, usize, f64)]) -> CoGraph {
        let mut s = vec![vec![0.0; n]; n];
        for &(a, b, v) in pairs {
            s[a][b] = v;
            s[b][a] = v;
        }
        CoGraph {
            names: (0..n).map(|i| format!("Species {i}")).collect(),
            share: s.clone(),
            strength: s,
            blocks: vec![vec![0; n]; n],
        }
    }

    /// Two groups of three species (three, because a group of four cannot be drawn without
    /// its two diagonals crossing), every pair inside a group linked and nothing between the
    /// groups, with the species numbered so the groups interleave (0,2,4 and 1,3,5):
    /// the order the query hands over, busiest first, is nothing to do with who sings
    /// with whom.
    fn two_interleaved_clusters() -> (CoGraph, Vec<Link>) {
        let mut pairs = Vec::new();
        for group in [[0, 2, 4], [1, 3, 5]] {
            for (i, &a) in group.iter().enumerate() {
                for &b in &group[i + 1..] {
                    pairs.push((a.min(b), a.max(b), 0.6));
                }
            }
        }
        let g = graph(6, &pairs);
        let links = g.ranked_links();
        (g, links)
    }

    /// The ring order is chosen, not inherited.
    ///
    /// The old diagram stood the species in the order the query ranked them. For two
    /// clusters that interleave in that ranking this is the worst case: every chord of one
    /// cluster crosses chords of the other. This test is observed failing against that
    /// (`identity`) order, which is the assertion's own first line, and passing for the
    /// order `chord_order` returns.
    #[test]
    fn the_ring_order_separates_clusters_instead_of_inheriting_the_query_order() {
        let (g, links) = two_interleaved_clusters();
        let identity: Vec<usize> = (0..g.n()).collect();
        let inherited = crossing_cost(&identity, &links);
        assert!(
            inherited > 1.0,
            "the fixture must be a graph the inherited order tangles ({inherited})"
        );
        let order = chord_order(&identity, &links);
        let chosen = crossing_cost(&order, &links);
        assert!(
            chosen < 1e-9,
            "two clusters have a crossing-free ring order, found {order:?} costing {chosen}"
        );
    }

    /// The counterpart: a search that always answered "zero" would pass the test above.
    /// A complete graph cannot avoid crossings in any order, and the cost it reports must
    /// not be a flattering one: K5 has five crossings however it is arranged.
    #[test]
    fn a_graph_that_must_cross_is_reported_as_crossing() {
        let mut pairs = Vec::new();
        for a in 0..5 {
            for b in a + 1..5 {
                pairs.push((a, b, 1.0));
            }
        }
        let g = graph(5, &pairs);
        let links = g.ranked_links();
        let order = chord_order(&[0, 1, 2, 3, 4], &links);
        assert!((crossing_cost(&order, &links) - 5.0).abs() < 1e-9);
    }

    /// The order is a permutation of what it was given, whatever the graph looks like.
    #[test]
    fn the_order_is_a_permutation_and_is_stable() {
        let (g, links) = two_interleaved_clusters();
        let nodes: Vec<usize> = (0..g.n()).collect();
        let a = chord_order(&nodes, &links);
        let b = chord_order(&nodes, &links);
        assert_eq!(
            a, b,
            "same data, same turn: the page and its cache must agree"
        );
        let mut sorted = a;
        sorted.sort_unstable();
        assert_eq!(sorted, nodes);
        // Few species: nothing to arrange, and nothing lost.
        assert_eq!(chord_order(&[3, 1], &[]), vec![3, 1]);
        assert!(chord_order(&[], &[]).is_empty());
    }

    /// Chords that share an end do not cross, and a link nobody is standing at is ignored.
    #[test]
    fn shared_ends_are_not_crossings() {
        let links = [
            Link { a: 0, b: 2, v: 1.0 },
            Link { a: 0, b: 3, v: 1.0 },
            Link { a: 1, b: 4, v: 1.0 },
        ];
        // 0-2 and 0-3 share an end; 1-4 crosses both (1 is between 0 and 2, 4 is outside).
        assert!((crossing_cost(&[0, 1, 2, 3, 4], &links) - 2.0).abs() < 1e-9);
        assert!(crossing_cost(&[0, 2], &links[..1]).abs() < 1e-9);
    }

    #[test]
    fn the_ramp_is_seven_even_steps() {
        assert_eq!(share_step(0.0), 1);
        assert_eq!(share_step(0.14), 1);
        assert_eq!(share_step(0.15), 2);
        assert_eq!(share_step(0.5), 4);
        assert_eq!(share_step(1.0), 7);
        assert_eq!(share_step(7.0), 7);
        assert_eq!(share_step(f64::NAN.max(0.0)), 1);
    }

    #[test]
    fn ranked_links_are_strongest_first_and_skip_the_empty() {
        let g = graph(4, &[(0, 1, 0.3), (2, 3, 0.9), (0, 3, 0.6)]);
        let l = g.ranked_links();
        assert_eq!(
            l.iter().map(|x| (x.a, x.b)).collect::<Vec<_>>(),
            vec![(2, 3), (0, 3), (0, 1)]
        );
        assert_eq!(g.top_links(2).len(), 2);
        assert!(CoGraph::default().ranked_links().is_empty());
    }
}
