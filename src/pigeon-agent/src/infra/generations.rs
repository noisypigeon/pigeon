//! Date-keyed token generations, from core, and today's day from the clock.

pub use crate::core::infra::generations::*;

/// Today, as a [`Day`], from the system clock.
#[must_use]
pub fn today() -> Day {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs / 86_400).unwrap_or(0)
}
