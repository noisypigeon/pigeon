//! A placeholder; a later amendment to
//! `.claude/decisions/2026-11-02-release-crate-units-and-images.md` decides its purpose.

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `pigeon gantry` reports until gantry does something.
#[must_use]
pub fn status() -> String {
    format!("gantry {VERSION}: not implemented yet")
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_names_the_version() {
        assert_eq!(
            super::status(),
            format!("gantry {}: not implemented yet", super::VERSION)
        );
    }
}
