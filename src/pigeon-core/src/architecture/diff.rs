//! The standby's architecture held against the primary's; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its step 10 amendment.

use super::{Edge, Graph};
use std::collections::BTreeSet;

/// What two graphs differ by, each entry an id: a node's `id`, an edge's [`edge_id`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// In the standby, not the primary.
    pub added: Vec<String>,
    /// In the primary, not the standby.
    pub removed: Vec<String>,
}

/// An edge's stable id: `<from> -> <to> (<carries>)`.
#[must_use]
pub fn edge_id(e: &Edge) -> String {
    format!("{} -> {} ({})", e.from, e.to, e.carries)
}

fn ids(g: &Graph) -> BTreeSet<String> {
    g.nodes
        .iter()
        .map(|n| n.id.clone())
        .chain(g.edges.iter().map(edge_id))
        .collect()
}

/// The nodes and edges `standby` has that `primary` lacks (added), and the reverse (removed), by id; groups and labels aren't compared.
#[must_use]
pub fn diff(primary: &Graph, standby: &Graph) -> Diff {
    let (p, s) = (ids(primary), ids(standby));
    Diff {
        added: s.difference(&p).cloned().collect(),
        removed: p.difference(&s).cloned().collect(),
    }
}

/// What `d` holds beyond what a change adds (`added`) and removes (`removed`), each as `added <id>` or `removed <id>`; empty when the standby is as expected.
#[must_use]
pub fn expected(d: &Diff, added: &[String], removed: &[String]) -> Vec<String> {
    let mut out: Vec<String> = d
        .added
        .iter()
        .filter(|id| !added.contains(id))
        .map(|id| format!("added {id}"))
        .collect();
    out.extend(
        d.removed
            .iter()
            .filter(|id| !removed.contains(id))
            .map(|id| format!("removed {id}")),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str) -> super::super::Node {
        super::super::Node {
            id: id.into(),
            kind: "k".into(),
            lane: super::super::Lane::Control,
            label: label.into(),
            source: "s".into(),
        }
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            carries: "request".into(),
            source: "s".into(),
        }
    }

    fn pair() -> (Graph, Graph) {
        let primary = Graph {
            nodes: vec![node("a", "A"), node("b", "B")],
            edges: vec![edge("a", "b")],
            groups: Vec::new(),
        };
        let standby = Graph {
            nodes: vec![node("a", "Other"), node("c", "C")],
            edges: vec![edge("a", "c")],
            groups: Vec::new(),
        };
        (primary, standby)
    }

    #[test]
    fn a_standby_differs_by_its_nodes_and_edges_only() {
        let (p, s) = pair();
        assert_eq!(
            diff(&p, &s),
            Diff {
                added: vec!["a -> c (request)".into(), "c".into()],
                removed: vec!["a -> b (request)".into(), "b".into()],
            }
        );
    }

    #[test]
    fn only_what_the_change_names_is_expected() {
        let (p, s) = pair();
        let d = diff(&p, &s);
        assert_eq!(
            expected(&d, &["c".into(), "a -> c (request)".into()], &["b".into()]),
            vec!["removed a -> b (request)".to_string()]
        );
        let all = expected(&d, &[], &[]);
        assert_eq!(all.len(), 4);
        assert!(all[0].starts_with("added ") && all[1].starts_with("added "));
    }
}
