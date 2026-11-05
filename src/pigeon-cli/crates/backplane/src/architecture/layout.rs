//! Places a [`Graph`] in lanes; pure and deterministic, per §2 "The layout".

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use super::{Graph, Lane};

/// The grid unit.
const UNIT: i64 = 8;
/// A node's height.
const NODE_H: i64 = 40;
/// A node's row, gap included.
const ROW: i64 = 56;
/// A waypoint's row.
const WAY_ROW: i64 = 18;
/// The gutter between lanes.
const GAP: i64 = 64;
/// The narrowest lane.
const MIN_W: i64 = 112;
/// Where lane headings sit; rows start below them.
const TOP: i64 = 104;
/// The left margin.
const LEFT: i64 = 32;
/// Barycentre sweeps, each left then right.
const SWEEPS: usize = 4;

/// A lane's column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneBox {
    /// The lane.
    pub lane: Lane,
    /// Its left edge.
    pub x: i64,
    /// Its width.
    pub w: i64,
}

/// A node's rectangle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    /// The node.
    pub id: String,
    /// Left.
    pub x: i64,
    /// Top.
    pub y: i64,
    /// Width.
    pub w: i64,
    /// Height.
    pub h: i64,
}

/// A group's box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupBox {
    /// The group.
    pub id: String,
    /// Left.
    pub x: i64,
    /// Top.
    pub y: i64,
    /// Width.
    pub w: i64,
    /// Height.
    pub h: i64,
}

/// An edge's SVG path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routed {
    /// The node it leaves.
    pub from: String,
    /// The node it reaches.
    pub to: String,
    /// What flows along it.
    pub carries: String,
    /// Its SVG path, in integer coordinates.
    pub d: String,
    /// Where it starts.
    pub start: (i64, i64),
    /// Where it ends.
    pub end: (i64, i64),
}

/// Where everything goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// The drawing's width.
    pub width: i64,
    /// The drawing's height.
    pub height: i64,
    /// Each non-empty lane, left to right.
    pub lanes: Vec<LaneBox>,
    /// Each node, by id.
    pub nodes: Vec<Placed>,
    /// Each group, by id.
    pub groups: Vec<GroupBox>,
    /// Each edge, in the graph's order.
    pub edges: Vec<Routed>,
}

/// A row in a lane: a node, or a long edge's waypoint.
struct Slot {
    id: String,
    lane: usize,
    way: bool,
    /// For a waypoint: its edge's group, when both ends share one.
    grp: Option<String>,
    width: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

fn label_width(label: &str) -> i64 {
    let chars = i64::try_from(label.chars().count()).unwrap_or(i64::MAX / 100);
    // 7.4px per monospace character, 28px of padding, rounded up to the grid.
    ((chars * 74 + 280 + 10 * UNIT - 1) / (10 * UNIT) * UNIT).max(MIN_W)
}

/// The top-level ancestor of the group `id`.
fn top_group(parent: &BTreeMap<&str, Option<&str>>, id: &str) -> String {
    let mut at = id;
    while let Some(Some(p)) = parent.get(at) {
        at = p;
    }
    at.to_string()
}

/// Places every node, group and edge of `graph`, which should be [`Graph::normalize`]d.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn layout(graph: &Graph) -> Layout {
    let used: Vec<Lane> = graph
        .nodes
        .iter()
        .map(|n| n.lane)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let lane_of: BTreeMap<Lane, usize> = used.iter().enumerate().map(|(i, l)| (*l, i)).collect();

    // Groups: each node's innermost group, and each group's top-level ancestor.
    let parent: BTreeMap<&str, Option<&str>> = graph
        .groups
        .iter()
        .map(|gr| (gr.id.as_str(), gr.parent.as_deref()))
        .collect();
    let top_of_group = |id: &str| top_group(&parent, id);
    let mut innermost: BTreeMap<&str, &str> = BTreeMap::new();
    for gr in &graph.groups {
        for m in &gr.members {
            let replace = innermost.get(m.as_str()).is_none_or(|cur| {
                parent.get(cur).copied().flatten().is_none() && gr.parent.is_some()
            });
            if replace {
                innermost.insert(m.as_str(), gr.id.as_str());
            }
        }
    }
    let tops: Vec<&str> = graph
        .groups
        .iter()
        .filter(|gr| gr.parent.is_none())
        .map(|gr| gr.id.as_str())
        .collect();

    // Slots: nodes, then one waypoint per lane a long edge crosses.
    let mut slots: Vec<Slot> = graph
        .nodes
        .iter()
        .map(|n| Slot {
            id: n.id.clone(),
            lane: lane_of[&n.lane],
            way: false,
            grp: innermost.get(n.id.as_str()).map(|gr| top_of_group(gr)),
            width: label_width(&n.label),
        })
        .collect();
    let slot_of: BTreeMap<String, usize> = slots
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.clone(), i))
        .collect();
    let mut chains: Vec<Vec<usize>> = Vec::new();
    for (k, e) in graph.edges.iter().enumerate() {
        let (Some(&src), Some(&dst)) = (slot_of.get(&e.from), slot_of.get(&e.to)) else {
            chains.push(Vec::new());
            continue;
        };
        let (la, lb) = (slots[src].lane, slots[dst].lane);
        let shared = match (&slots[src].grp, &slots[dst].grp) {
            (Some(one), Some(other)) if one == other => Some(one.clone()),
            _ => None,
        };
        let mut chain = vec![src];
        if la.abs_diff(lb) > 1 {
            let lanes: Vec<usize> = if la < lb {
                (la + 1..lb).collect()
            } else {
                (lb + 1..la).rev().collect()
            };
            for i in lanes {
                slots.push(Slot {
                    id: format!("~{k:05}:{i:02}"),
                    lane: i,
                    way: true,
                    grp: shared.clone(),
                    width: 0,
                });
                chain.push(slots.len() - 1);
            }
        }
        chain.push(dst);
        chains.push(chain);
    }

    let rank = |s: &Slot| {
        s.grp
            .as_deref()
            .and_then(|gr| tops.iter().position(|t| *t == gr))
            .unwrap_or(usize::MAX)
    };
    let inner = |s: &Slot| {
        if s.way {
            return 1;
        }
        usize::from(
            innermost
                .get(s.id.as_str())
                .is_none_or(|gr| parent.get(gr).copied().flatten().is_none()),
        )
    };

    let mut lanes: Vec<Vec<usize>> = vec![Vec::new(); used.len()];
    for (i, s) in slots.iter().enumerate() {
        lanes[s.lane].push(i);
    }
    for lane in &mut lanes {
        lane.sort_by(|&a, &b| (slots[a].way, &slots[a].id).cmp(&(slots[b].way, &slots[b].id)));
    }
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); slots.len()];
    for chain in &chains {
        for pair in chain.windows(2) {
            neighbours[pair[0]].push(pair[1]);
            neighbours[pair[1]].push(pair[0]);
        }
    }

    // Barycentre sweeps, by relative position within each lane.
    let mut at = vec![0.5_f64; slots.len()];
    let reindex = |lanes: &[Vec<usize>], at: &mut [f64]| {
        for lane in lanes {
            #[allow(clippy::cast_precision_loss)]
            let span = (lane.len().max(2) - 1) as f64;
            for (i, &s) in lane.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let v = if lane.len() > 1 { i as f64 / span } else { 0.5 };
                at[s] = v;
            }
        }
    };
    reindex(&lanes, &mut at);
    for _ in 0..SWEEPS {
        let order: Vec<usize> = (0..lanes.len()).chain((0..lanes.len()).rev()).collect();
        for li in order {
            let bary: BTreeMap<usize, f64> = lanes[li]
                .iter()
                .map(|&s| {
                    let others: Vec<f64> = neighbours[s]
                        .iter()
                        .filter(|&&o| slots[o].lane != li)
                        .map(|&o| at[o])
                        .collect();
                    #[allow(clippy::cast_precision_loss)]
                    let v = if others.is_empty() {
                        at[s]
                    } else {
                        others.iter().sum::<f64>() / others.len() as f64
                    };
                    (s, v)
                })
                .collect();
            lanes[li].sort_by(|&a, &b| {
                rank(&slots[a])
                    .cmp(&rank(&slots[b]))
                    .then(inner(&slots[a]).cmp(&inner(&slots[b])))
                    .then(bary[&a].partial_cmp(&bary[&b]).unwrap_or(Ordering::Equal))
                    .then(slots[a].id.cmp(&slots[b].id))
            });
            reindex(&lanes, &mut at);
        }
    }

    // A top-level group's span: its members' lanes, and every lane between.
    let span = |gr: &str| {
        let ls: Vec<usize> = graph
            .groups
            .iter()
            .find(|x| x.id == gr)
            .map(|x| {
                x.members
                    .iter()
                    .filter_map(|m| slot_of.get(m))
                    .map(|&s| slots[s].lane)
                    .collect()
            })
            .unwrap_or_default();
        (ls.iter().min().copied(), ls.iter().max().copied())
    };
    let mut spanned = BTreeSet::new();
    for gr in &tops {
        if let (Some(lo), Some(hi)) = span(gr) {
            spanned.extend(lo..=hi);
        }
    }
    // In a spanned lane: members, then pass-through waypoints, then everything else.
    for (i, lane) in lanes.iter_mut().enumerate() {
        if spanned.contains(&i) {
            let key = |s: &Slot| match (&s.grp, s.way) {
                (Some(_), _) => 0,
                (None, true) => 1,
                (None, false) => 2,
            };
            lane.sort_by_key(|&s| key(&slots[s]));
        }
    }

    // Columns.
    let mut cursor = LEFT;
    let mut cols: Vec<(i64, i64)> = Vec::new();
    for lane in &lanes {
        let w = lane
            .iter()
            .filter(|&&s| !slots[s].way)
            .map(|&s| slots[s].width)
            .max()
            .unwrap_or(MIN_W)
            .max(MIN_W);
        cols.push((cursor, w));
        cursor += w + GAP;
    }

    // Top-level groups stack: each band starts below every earlier band in its lanes, and
    // the rest of a lane starts below every band that spans it.
    let mut fill = vec![0_i64; lanes.len()];
    let mut band_start: BTreeMap<&str, i64> = BTreeMap::new();
    for gr in &tops {
        let (Some(lo), Some(hi)) = span(gr) else {
            continue;
        };
        let start = fill[lo..=hi].iter().copied().max().unwrap_or(0);
        let height = lanes
            .iter()
            .map(|lane| {
                lane.iter()
                    .filter(|&&s| slots[s].grp.as_deref() == Some(*gr))
                    .map(|&s| if slots[s].way { WAY_ROW } else { ROW })
                    .sum::<i64>()
            })
            .max()
            .unwrap_or(0);
        for f in &mut fill[lo..=hi] {
            *f = start + height + ROW;
        }
        band_start.insert(gr, start);
    }
    let mut pos: Vec<(i64, i64, i64, i64)> = vec![(0, 0, 0, 0); slots.len()];
    for (i, lane) in lanes.iter().enumerate() {
        let (cx, cw) = cols[i];
        let mut y = 0;
        let mut current: Option<&str> = None;
        let mut below = false;
        for &s in lane {
            let slot = &slots[s];
            match slot.grp.as_deref() {
                Some(gr) if current != Some(gr) => {
                    y = y.max(band_start.get(gr).copied().unwrap_or(0));
                    current = Some(gr);
                }
                None if !slot.way && !below => {
                    y = y.max(fill[i]);
                    below = true;
                }
                _ => {}
            }
            pos[s] = (cx, TOP + y, cw, if slot.way { 0 } else { NODE_H });
            y += if slot.way { WAY_ROW } else { ROW };
        }
    }

    let groups: Vec<GroupBox> = graph
        .groups
        .iter()
        .map(|gr| {
            let ps: Vec<_> = gr
                .members
                .iter()
                .filter_map(|m| slot_of.get(m))
                .map(|&s| pos[s])
                .collect();
            let pad = if gr.parent.is_some() { 10 } else { 22 };
            let x0 = ps.iter().map(|p| p.0).min().unwrap_or(0) - pad;
            let y0 = ps.iter().map(|p| p.1).min().unwrap_or(0) - pad - 14;
            let x1 = ps.iter().map(|p| p.0 + p.2).max().unwrap_or(0) + pad;
            let y1 = ps.iter().map(|p| p.1 + p.3).max().unwrap_or(0) + pad;
            GroupBox {
                id: gr.id.clone(),
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            }
        })
        .collect();

    // Ports, spread along each side in the order of the nodes at the other end.
    let sides = |a: usize, b: usize| match slots[a].lane.cmp(&slots[b].lane) {
        Ordering::Equal => (Side::Right, Side::Right),
        Ordering::Less => (Side::Right, Side::Left),
        Ordering::Greater => (Side::Left, Side::Right),
    };
    // Each side's ports hold (the slot at the other end, the edge), so parallel edges part.
    let mut ports: BTreeMap<(usize, bool), Vec<(usize, usize)>> = BTreeMap::new();
    for (k, chain) in chains.iter().enumerate().filter(|(_, c)| c.len() >= 2) {
        let n = chain.len();
        let (s, _) = sides(chain[0], chain[1]);
        let (_, t) = sides(chain[n - 2], chain[n - 1]);
        ports
            .entry((chain[0], s == Side::Right))
            .or_default()
            .push((chain[1], k));
        ports
            .entry((chain[n - 1], t == Side::Right))
            .or_default()
            .push((chain[n - 2], k));
    }
    for others in ports.values_mut() {
        others.sort_by(|&(a, ka), &(b, kb)| {
            pos[a]
                .1
                .cmp(&pos[b].1)
                .then(slots[a].id.cmp(&slots[b].id))
                .then(ka.cmp(&kb))
        });
    }
    let port_y = |s: usize, side: Side, edge: usize| {
        let p = pos[s];
        if slots[s].way {
            return p.1;
        }
        let list = &ports[&(s, side == Side::Right)];
        let n = i64::try_from(list.len()).unwrap_or(1) + 1;
        let at = list.iter().position(|&(_, k)| k == edge).unwrap_or(0);
        let k = i64::try_from(at).unwrap_or(0) + 1;
        p.1 + (p.3 * k + n / 2) / n
    };
    let side_x = |s: usize, side: Side| {
        let p = pos[s];
        if side == Side::Right { p.0 + p.2 } else { p.0 }
    };

    let edges: Vec<Routed> = graph
        .edges
        .iter()
        .zip(&chains)
        .enumerate()
        .filter(|(_, (_, c))| c.len() >= 2)
        .map(|(k, (e, chain))| {
            let mut path = String::new();
            let mut start = (0, 0);
            let mut end = (0, 0);
            for (i, pair) in chain.windows(2).enumerate() {
                let (here, next) = (pair[0], pair[1]);
                let (out_side, in_side) = sides(here, next);
                let (x1, y1) = (side_x(here, out_side), port_y(here, out_side, k));
                let (x2, y2) = (side_x(next, in_side), port_y(next, in_side, k));
                if i == 0 {
                    start = (x1, y1);
                    let _ = write!(path, "M{x1} {y1}");
                }
                let bend = if out_side == in_side {
                    GAP / 2
                } else {
                    (x2 - x1).abs() / 2
                };
                let c1 = if out_side == Side::Right {
                    x1 + bend
                } else {
                    x1 - bend
                };
                let c2 = if in_side == Side::Right {
                    x2 + bend
                } else {
                    x2 - bend
                };
                let _ = write!(path, "C{c1} {y1} {c2} {y2} {x2} {y2}");
                if slots[next].way {
                    let through = if in_side == Side::Left {
                        pos[next].0 + pos[next].2
                    } else {
                        pos[next].0
                    };
                    let _ = write!(path, "H{through}");
                } else {
                    end = (x2, y2);
                }
            }
            Routed {
                from: e.from.clone(),
                to: e.to.clone(),
                carries: e.carries.clone(),
                d: path,
                start,
                end,
            }
        })
        .collect();

    let nodes: Vec<Placed> = graph
        .nodes
        .iter()
        .map(|n| {
            let p = pos[slot_of[&n.id]];
            Placed {
                id: n.id.clone(),
                x: p.0,
                y: p.1,
                w: p.2,
                h: p.3,
            }
        })
        .collect();
    let width = if lanes.is_empty() {
        2 * LEFT
    } else {
        cursor - GAP + LEFT
    };
    let height = nodes
        .iter()
        .map(|p| p.y + p.h)
        .chain(groups.iter().map(|b| b.y + b.h))
        .max()
        .unwrap_or(TOP)
        + 32;
    Layout {
        width,
        height,
        lanes: used
            .iter()
            .zip(&cols)
            .map(|(l, (x, w))| LaneBox {
                lane: *l,
                x: *x,
                w: *w,
            })
            .collect(),
        nodes,
        groups,
        edges,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Edge, Graph, Group, Lane, Node};
    use super::{Layout, layout};

    const LANES: [Lane; 10] = [
        Lane::Policy,
        Lane::Source,
        Lane::Edge,
        Lane::Control,
        Lane::Data,
        Lane::Operations,
        Lane::Run,
        Lane::Units,
        Lane::Providers,
        Lane::Sites,
    ];

    /// A small deterministic generator, so the property test needs no dependency.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(self.0 >> 33).unwrap_or(0) % below.max(1)
        }
    }

    fn graph(seed: u64) -> Graph {
        let mut r = Lcg(seed);
        let n = 2 + r.next(24);
        let mut g = Graph::default();
        for i in 0..n {
            g.nodes.push(Node {
                id: format!("n{i:02}"),
                kind: "unit".into(),
                lane: LANES[r.next(LANES.len())],
                label: "x".repeat(1 + r.next(30)),
                source: "test".into(),
            });
        }
        for _ in 0..r.next(3 * n) {
            g.edges.push(Edge {
                from: format!("n{:02}", r.next(n)),
                to: format!("n{:02}", r.next(n)),
                carries: ["request", "binds"][r.next(2)].into(),
                source: "test".into(),
            });
        }
        if n > 4 {
            let members: Vec<String> = (0..=r.next(4))
                .map(|_| format!("n{:02}", r.next(n)))
                .collect();
            g.groups.push(Group {
                id: "g".into(),
                kind: "cloudflare".into(),
                label: "group".into(),
                members: members.clone(),
                parent: None,
            });
            g.groups.push(Group {
                id: "h".into(),
                kind: "container".into(),
                label: "inner".into(),
                members: members[..1].to_vec(),
                parent: Some("g".into()),
            });
            let others: Vec<String> = (0..=r.next(3))
                .map(|_| format!("n{:02}", r.next(n)))
                .filter(|m| !members.contains(m))
                .collect();
            g.groups.push(Group {
                id: "k".into(),
                kind: "local".into(),
                label: "other".into(),
                members: others,
                parent: None,
            });
        }
        g.normalize();
        g
    }

    fn check(g: &Graph, l: &Layout) {
        for (i, a) in l.nodes.iter().enumerate() {
            for b in &l.nodes[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{} overlaps {}", a.id, b.id);
            }
        }
        let at = |id: &str| l.nodes.iter().find(|p| p.id == id).expect("placed");
        let on_border = |p: &super::Placed, (x, y): (i64, i64)| {
            (x == p.x || x == p.x + p.w) && (p.y..=p.y + p.h).contains(&y)
        };
        assert_eq!(l.edges.len(), g.edges.len());
        for e in &l.edges {
            assert!(
                on_border(at(&e.from), e.start),
                "{} starts off its node",
                e.from
            );
            assert!(on_border(at(&e.to), e.end), "{} ends off its node", e.to);
        }
        for (gr, b) in g.groups.iter().zip(&l.groups) {
            for m in &gr.members {
                let p = at(m);
                assert!(
                    b.x <= p.x && p.x + p.w <= b.x + b.w && b.y <= p.y && p.y + p.h <= b.y + b.h,
                    "{} is outside {}",
                    m,
                    gr.id
                );
            }
        }
        for b in l.groups.iter().filter(|b| b.id != "h") {
            let members = &g
                .groups
                .iter()
                .find(|x| x.id == b.id)
                .expect("group")
                .members;
            for p in l.nodes.iter().filter(|p| !members.contains(&p.id)) {
                let inside =
                    b.x < p.x && p.x + p.w < b.x + b.w && b.y < p.y && p.y + p.h < b.y + b.h;
                assert!(
                    !inside,
                    "{} is not a member of {} but sits inside it",
                    p.id, b.id
                );
            }
        }
        for (i, a) in l.edges.iter().enumerate() {
            for b in &l.edges[i + 1..] {
                if a.from == b.from && a.to == b.to {
                    assert_ne!(a.d, b.d, "parallel edges {} → {} coincide", a.from, a.to);
                }
            }
        }
    }

    #[test]
    fn generated_graphs_lay_out_without_overlap_and_on_their_borders() {
        for seed in 0..300 {
            let g = graph(seed);
            let l = layout(&g);
            check(&g, &l);
            assert_eq!(layout(&g), l, "seed {seed} is not deterministic");
        }
    }

    #[test]
    fn an_empty_graph_lays_out_empty() {
        let l = layout(&Graph::default());
        assert!(l.nodes.is_empty() && l.lanes.is_empty() && l.edges.is_empty());
    }
}
