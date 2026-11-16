//! The router's lookup: each surface's routes, joined into one table; see
//! `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §4.

use backplane::routes::{Path, Route, find};

const GIT: &[Route<&str, &str>] = &[Route::any(Path::Prefix("/git/"), "git")];
const APPROVE: &[Route<&str, &str>] = &[
    Route::any(Path::Prefix("/approve/"), "approve"),
    Route::any(Path::Exact("/approvals"), "approvals"),
    Route::any(Path::Prefix("/approvals/"), "approvals/*"),
];
const PUSH: &[Route<&str, &str>] = &[Route::any(Path::Exact("/approvals/push"), "push")];
const API: &[Route<&str, &str>] = &[
    Route::only("POST", Path::Exact("/mcp"), "mcp"),
    Route::only("GET", Path::Exact("/v0/operations"), "operations"),
];
const TABLE: &[&[Route<&str, &str>]] = &[API, GIT, APPROVE, PUSH];

fn served(method: &'static str, path: &str) -> Option<&'static str> {
    find(TABLE, &method, path).map(|r| r.serve)
}

#[test]
fn a_path_reaches_the_surface_that_declares_it() {
    assert_eq!(
        served("GET", "/git/pigeon/pigeon.git/info/refs"),
        Some("git")
    );
    assert_eq!(served("GET", "/approve/7"), Some("approve"));
    assert_eq!(served("GET", "/approvals"), Some("approvals"));
    assert_eq!(served("GET", "/nowhere"), None);
    assert_eq!(served("GET", "/gi"), None, "a prefix matches whole");
}

#[test]
fn an_exact_path_wins_over_a_prefix_declared_before_it() {
    assert_eq!(served("POST", "/approvals/push"), Some("push"));
    assert_eq!(served("GET", "/approvals/other"), Some("approvals/*"));
}

#[test]
fn a_route_with_a_method_serves_only_that_method() {
    assert_eq!(served("POST", "/mcp"), Some("mcp"));
    assert_eq!(served("GET", "/mcp"), None);
    assert_eq!(served("GET", "/v0/operations"), Some("operations"));
}
