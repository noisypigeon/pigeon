//! The platform's architecture as described from the tree, with core's graph and layout; see
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §2.

#[cfg(feature = "engines")]
mod describe;
#[cfg(feature = "engines")]
mod network;
#[cfg(feature = "engines")]
mod place;

pub use pigeon_core::architecture::*;

#[cfg(feature = "engines")]
pub use describe::{describe, describe_with};
#[cfg(feature = "engines")]
pub use network::{
    Kind, LINKS, Network, ROUTE_KEYS, Row, SERVICES, STANDALONE, Service, evidenced, hosts_in,
    internal, inventory, literals, network, outbound_files,
};
#[cfg(feature = "engines")]
pub use place::{Diagram, place};
