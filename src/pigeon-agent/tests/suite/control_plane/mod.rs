//! The control plane's routing, schema, statements, MCP tools, routes, object versions and access. See
//! `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §2.

mod access;
mod control;
mod mcp;
mod migrations;
mod routes;
mod versions;
