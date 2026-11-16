//! Routes: each Worker surface declares the paths it serves, and the router is the table joining
//! them; see `.claude/decisions/2026-11-10-development-factory-crates-functional-core.md` §4.

/// The paths a route serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// This path only.
    Exact(&'static str),
    /// Every path under this prefix.
    Prefix(&'static str),
}

impl Path {
    /// Whether `path` is served.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        match self {
            Self::Exact(p) => path == *p,
            Self::Prefix(p) => path.starts_with(p),
        }
    }
}

/// A route: its paths, an HTTP method or any, and what serves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route<M: 'static, S: 'static> {
    /// Its paths.
    pub path: Path,
    /// Its method; none serves any.
    pub method: Option<M>,
    /// What serves it.
    pub serve: S,
}

impl<M, S> Route<M, S> {
    /// A route for `path` with any method.
    pub const fn any(path: Path, serve: S) -> Self {
        Self {
            path,
            method: None,
            serve,
        }
    }

    /// A route for `path` with `method` only.
    pub const fn only(method: M, path: Path, serve: S) -> Self {
        Self {
            path,
            method: Some(method),
            serve,
        }
    }
}

/// The first route of `table`, surface by surface, that serves `method` on `path`. An exact path
/// wins over a prefix, wherever each is declared.
#[must_use]
pub fn find<'a, M: PartialEq, S>(
    table: &'a [&'a [Route<M, S>]],
    method: &M,
    path: &str,
) -> Option<&'a Route<M, S>> {
    let serves =
        |r: &&Route<M, S>| r.path.matches(path) && r.method.as_ref().is_none_or(|m| m == method);
    let all = || table.iter().flat_map(|s| s.iter());
    all()
        .filter(|r| matches!(r.path, Path::Exact(_)))
        .find(serves)
        .or_else(|| all().find(serves))
}
