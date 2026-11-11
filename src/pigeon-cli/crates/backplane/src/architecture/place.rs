//! Where the network's boxes sit and how its links run: rows by kind, columns centred, and
//! orthogonal routes through the gaps between rows; see the 2026-11-11 amendment of
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use super::network::{Link, Part};

/// A box's width.
pub const W: i64 = 220;
/// A box's height.
pub const H: i64 = 100;
/// The distance between columns' centres.
const PITCH: i64 = 320;
/// The distance between rows' tops.
const ROW: i64 = 260;
/// The diagram's width.
const WIDTH: i64 = 1000;
/// Where a link between boxes of one row runs, below their tops.
const ACROSS: i64 = 56;

/// A placed box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Boxed {
    /// Its part.
    pub id: String,
    /// Its left edge.
    pub x: i64,
    /// Its top edge.
    pub y: i64,
    /// Its width.
    pub w: i64,
    /// Its height.
    pub h: i64,
}

/// A routed link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    /// Its link.
    pub id: String,
    /// Its SVG path.
    pub d: String,
    /// Where its label sits.
    pub lx: i64,
    /// Where its label's baseline sits.
    pub ly: i64,
    /// `start` or `middle`.
    pub anchor: &'static str,
}

/// The connection diagram's geometry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diagram {
    /// Its width.
    pub width: i64,
    /// Its height, from the first row's top to the last row's bottom.
    pub height: i64,
    /// Every box.
    pub boxes: Vec<Boxed>,
    /// Every link drawn.
    pub routes: Vec<Route>,
}

fn row_of(kind: &str) -> Option<i64> {
    match kind {
        "object" => Some(0),
        "container" => Some(1),
        "database" | "bucket" | "git" => Some(2),
        _ => None,
    }
}

/// Places `parts` and routes `links` between them; a part of no row, such as GitHub's, and a
/// link to one, are left to the page's flow.
#[must_use]
pub fn place(parts: &[Part], links: &[Link]) -> Diagram {
    let mut rows: BTreeMap<i64, Vec<&Part>> = BTreeMap::new();
    for p in parts {
        if let Some(r) = row_of(&p.kind) {
            rows.entry(r).or_default().push(p);
        }
    }
    // The part its row calls most sits in the middle.
    for row in rows.values_mut() {
        let into = |p: &Part| {
            links
                .iter()
                .filter(|l| l.to == p.id && l.from != p.id && row.iter().any(|q| q.id == l.from))
                .count()
        };
        let best = (0..row.len()).max_by_key(|&i| (into(row[i]), std::cmp::Reverse(i)));
        if let Some(i) = best.filter(|&i| into(row[i]) > 0) {
            let p = row.remove(i);
            row.insert(row.len() / 2, p);
        }
    }
    let mut boxes = Vec::new();
    for (r, row) in &rows {
        let n = i64::try_from(row.len()).unwrap_or(0);
        for (i, p) in (0..).zip(row.iter()) {
            let centre = WIDTH / 2 + (2 * i - (n - 1)) * PITCH / 2;
            boxes.push(Boxed {
                id: p.id.clone(),
                x: centre - W / 2,
                y: r * ROW,
                w: W,
                h: H,
            });
        }
    }
    let at = |id: &str| boxes.iter().find(|b| b.id == id);
    let drawn: Vec<&Link> = links
        .iter()
        .filter(|l| at(&l.from).is_some() && at(&l.to).is_some())
        .collect();
    let mut routes = Vec::new();
    let mut down = Vec::new();
    for l in &drawn {
        let (Some(s), Some(t)) = (at(&l.from), at(&l.to)) else {
            continue;
        };
        if l.from == l.to {
            let (r, top) = (s.x + s.w, s.y);
            routes.push(Route {
                id: l.id.clone(),
                d: format!(
                    "M{} {top}V{}H{}V{}H{}",
                    r - 28,
                    top - 14,
                    r + 16,
                    top + 26,
                    r + 1
                ),
                lx: r + 22,
                ly: top - 17,
                anchor: "start",
            });
        } else if s.y == t.y {
            let y = s.y + ACROSS;
            let (from, to) = if s.x < t.x {
                (s.x + s.w, t.x - 1)
            } else {
                (s.x, t.x + t.w + 1)
            };
            routes.push(Route {
                id: l.id.clone(),
                d: format!("M{from} {y}H{to}"),
                lx: i64::midpoint(from, to),
                ly: y - 8,
                anchor: "middle",
            });
        } else if t.y > s.y {
            down.push((*l, s, t));
        }
    }
    routes.extend(route_down(&down, &boxes));
    routes.sort_by(|a, b| a.id.cmp(&b.id));
    let height = boxes.iter().map(|b| b.y + b.h).max().unwrap_or(0);
    Diagram {
        width: WIDTH,
        height,
        boxes,
        routes,
    }
}

/// Spreads `n` points evenly inside `from..to`, the `k`th of them.
fn spread(from: i64, to: i64, k: usize, n: usize) -> i64 {
    let (k, n) = (i64::try_from(k).unwrap_or(0), i64::try_from(n).unwrap_or(1));
    from + (to - from) * (k + 1) / (n + 1)
}

/// Each downward link leaves its box's bottom, runs to a channel in the gap above its target's
/// row, across, and into its target's top; one that passes a row keeps clear of its boxes.
fn route_down(down: &[(&Link, &Boxed, &Boxed)], boxes: &[Boxed]) -> Vec<Route> {
    let centre = |b: &Boxed| b.x + b.w / 2;
    let mut exit: BTreeMap<&str, i64> = BTreeMap::new();
    let mut sources: BTreeMap<&str, Vec<&(&Link, &Boxed, &Boxed)>> = BTreeMap::new();
    for d in down {
        sources.entry(d.1.id.as_str()).or_default().push(d);
    }
    for (_, mut ls) in sources {
        ls.sort_by_key(|(l, _, t)| (centre(t), l.id.clone()));
        for (k, (l, s, _)) in ls.iter().enumerate() {
            exit.insert(l.id.as_str(), spread(s.x, s.x + s.w, k, ls.len()));
        }
    }
    let mut entry: BTreeMap<&str, i64> = BTreeMap::new();
    let mut targets: BTreeMap<&str, Vec<&(&Link, &Boxed, &Boxed)>> = BTreeMap::new();
    for d in down {
        targets.entry(d.2.id.as_str()).or_default().push(d);
    }
    for (_, mut ls) in targets {
        ls.sort_by_key(|(l, _, _)| (exit[l.id.as_str()], l.id.clone()));
        for (k, (l, _, t)) in ls.iter().enumerate() {
            entry.insert(l.id.as_str(), spread(t.x, t.x + t.w, k, ls.len()));
        }
    }
    // Each gap's channels, the longest run highest.
    let mut gaps: BTreeMap<i64, Vec<(&Link, i64)>> = BTreeMap::new();
    for (l, _, t) in down {
        let span = (exit[l.id.as_str()] - entry[l.id.as_str()]).abs();
        gaps.entry(t.y).or_default().push((l, span));
    }
    let mut channel: BTreeMap<&str, i64> = BTreeMap::new();
    for (top, mut ls) in gaps {
        ls.sort_by_key(|(l, span)| (std::cmp::Reverse(*span), l.id.clone()));
        let (from, to) = (top - ROW + H + 12, top - 12);
        for (k, (l, _)) in ls.iter().enumerate() {
            channel.insert(l.id.as_str(), spread(from, to, k, ls.len()));
        }
    }
    let mut out = Vec::new();
    for (l, s, t) in down {
        let id = l.id.as_str();
        let (mut x, tx, cy) = (exit[id], entry[id], channel[id]);
        let mut d = format!("M{x} {}", s.y + s.h);
        // A box of a row in between is passed beside it.
        if let Some(b) = boxes
            .iter()
            .find(|b| b.y > s.y && b.y < t.y && x > b.x - 12 && x < b.x + b.w + 12)
        {
            let jog = b.y - (ROW - H) / 2;
            x = if x - b.x < b.x + b.w - x {
                b.x - 16
            } else {
                b.x + b.w + 16
            };
            let _ = write!(d, "V{jog}H{x}");
        }
        let (lx, ly, anchor) = if x == tx {
            let _ = write!(d, "V{}", t.y - 1);
            (x + 6, (s.y + s.h + t.y) / 2, "start")
        } else {
            let _ = write!(d, "V{cy}H{tx}V{}", t.y - 1);
            (i64::midpoint(x, tx), cy - 7, "middle")
        };
        out.push(Route {
            id: l.id.clone(),
            d,
            lx,
            ly,
            anchor,
        });
    }
    out
}
