//! The platform's architecture as a graph, and its layout; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §2.

#[cfg(feature = "engines")]
mod describe;
mod layout;
#[cfg(feature = "engines")]
mod network;

#[cfg(feature = "engines")]
pub use describe::{describe, describe_with};
pub use layout::{GroupBox, LaneBox, Layout, Placed, Routed, layout};
#[cfg(feature = "engines")]
pub use network::{Kind, LINKS, Network, Row, evidenced, network};

use serde::{Deserialize, Serialize};

/// A column of the diagram, left to right in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    /// Policies.
    Policy,
    /// The repository and pull requests.
    Source,
    /// Routes and Access.
    Edge,
    /// Workers, Durable Objects and crons.
    Control,
    /// D1, R2, secrets and stores.
    Data,
    /// Backplane's operations.
    Operations,
    /// Containers and engines.
    Run,
    /// Units.
    Units,
    /// Providers and their capabilities.
    Providers,
    /// Sites.
    Sites,
}

/// One part of the platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Unique, stable across builds.
    pub id: String,
    /// What it is, such as `engine`, `unit` or `site`.
    pub kind: String,
    /// Its column.
    pub lane: Lane,
    /// What the diagram shows.
    pub label: String,
    /// The file and key it was derived from.
    pub source: String,
}

/// One connection, and what flows along it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// The node it leaves.
    pub from: String,
    /// The node it reaches.
    pub to: String,
    /// `runs`, `governs`, `request`, `binds`, `signed-request`, `apply`, `credential`,
    /// `state` or `artifact`.
    pub carries: String,
    /// The file and key it was derived from.
    pub source: String,
}

/// A box drawn around nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Unique, stable across builds.
    pub id: String,
    /// What it is, such as `local` or `cloudflare`.
    pub kind: String,
    /// What the diagram shows.
    pub label: String,
    /// Its nodes' ids.
    pub members: Vec<String>,
    /// The group it sits inside, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// The whole graph: nodes and groups sorted by id, edges by their ends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    /// Every node.
    pub nodes: Vec<Node>,
    /// Every edge.
    pub edges: Vec<Edge>,
    /// Every group.
    pub groups: Vec<Group>,
}

impl Graph {
    /// Sorts nodes and groups by id and edges by their ends, dropping duplicates and any
    /// edge whose ends are not both nodes.
    pub fn normalize(&mut self) {
        self.nodes.sort_by(|a, b| a.id.cmp(&b.id));
        self.nodes.dedup_by(|a, b| a.id == b.id);
        let known = |id: &str| {
            self.nodes
                .binary_search_by(|n| n.id.as_str().cmp(id))
                .is_ok()
        };
        let mut edges: Vec<Edge> = self
            .edges
            .drain(..)
            .filter(|e| e.from != e.to && known(&e.from) && known(&e.to))
            .collect();
        edges.sort_by(|a, b| (&a.from, &a.to, &a.carries).cmp(&(&b.from, &b.to, &b.carries)));
        edges.dedup_by(|a, b| (&a.from, &a.to, &a.carries) == (&b.from, &b.to, &b.carries));
        self.edges = edges;
        for g in &mut self.groups {
            g.members.retain(|m| known(m));
            g.members.sort();
            g.members.dedup();
        }
        self.groups.retain(|g| !g.members.is_empty());
        self.groups.sort_by(|a, b| a.id.cmp(&b.id));
    }
}
