//! Placeholder library; its purpose is decided by a later ADR.

/// This crate's version.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_package_version() {
        assert_ne!(super::version(), "");
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }
}
