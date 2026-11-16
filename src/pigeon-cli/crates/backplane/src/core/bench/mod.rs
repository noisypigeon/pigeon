//! Benches: a session's own build-and-test Containers, pure; see
//! `.claude/decisions/2026-11-16-development-session-bench-containers.md`.

pub mod admit;
pub mod allow;
pub mod deal;

/// Benches one session may hold.
pub const MAX_PER_SESSION: u8 = 3;
/// Default cap on benches across the fleet.
pub const DEFAULT_MAX: u32 = 24;
/// Hard ceiling on the fleet cap.
pub const CEILING: u32 = 32;
/// Default idle timeout, in minutes.
pub const DEFAULT_IDLE_MINUTES: u32 = 30;
/// Shortest idle timeout, in minutes.
pub const MIN_IDLE_MINUTES: u32 = 5;
/// Longest idle timeout, in minutes.
pub const MAX_IDLE_MINUTES: u32 = 120;
/// Default daily Container-hour budget.
pub const DEFAULT_HOURS: u32 = 80;
/// Default command timeout, in minutes.
pub const DEFAULT_TIMEOUT_MINUTES: u32 = 30;
/// Longest command timeout, in minutes.
pub const MAX_TIMEOUT_MINUTES: u32 = 90;
/// Largest `target` directory, in MB, before a clean.
pub const TARGET_CEILING_MB: u64 = 10_240;
/// Least free disk, in MB, before a clean.
pub const MIN_FREE_MB: u64 = 3_072;
/// Bytes of log returned per page.
pub const LOG_PAGE: usize = 65_536;
/// Longest a wait call blocks, in seconds.
pub const MAX_WAIT_SECONDS: u32 = 25;
/// Ref prefix of a session's scratch branch.
pub const SCRATCH: &str = "refs/heads/scratch/";

/// True for `session_` followed by 1 to 56 ASCII alphanumerics.
#[must_use]
pub fn valid_session_id(id: &str) -> bool {
    match id.strip_prefix("session_") {
        Some(rest) => {
            (1..=56).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// The session ID from a label, when valid.
#[must_use]
pub fn session_id(label: &str) -> Option<&str> {
    let id = crate::sessions::session_id(label);
    valid_session_id(id).then_some(id)
}

/// The bench name `<id>.<k>`, when the ID and `k` are valid.
#[must_use]
pub fn name(session_id: &str, k: u8) -> Option<String> {
    (valid_session_id(session_id) && k < MAX_PER_SESSION).then(|| format!("{session_id}.{k}"))
}

/// Splits a bench name into its session ID and `k`.
#[must_use]
pub fn parse_name(name: &str) -> Option<(&str, u8)> {
    let (id, digits) = name.rsplit_once('.')?;
    let mut bytes = digits.bytes();
    let d = bytes.next()?;
    if bytes.next().is_some() || !d.is_ascii_digit() {
        return None;
    }
    let k = d - b'0';
    (valid_session_id(id) && k < MAX_PER_SESSION).then_some((id, k))
}

/// A session's scratch ref.
#[must_use]
pub fn scratch_ref(session_id: &str) -> String {
    format!("{SCRATCH}{session_id}")
}

/// The session ID a scratch ref belongs to, when valid.
#[must_use]
pub fn scratch_owner(refname: &str) -> Option<&str> {
    refname
        .strip_prefix(SCRATCH)
        .filter(|rest| valid_session_id(rest))
}

/// True when `target` is too large or free disk too small.
#[must_use]
pub fn needs_clean(target_mb: u64, free_mb: u64) -> bool {
    target_mb > TARGET_CEILING_MB || free_mb < MIN_FREE_MB
}

/// A command timeout in minutes, defaulted and bounded.
///
/// # Errors
///
/// Refuses a value outside 1 to 90.
pub fn timeout(minutes: Option<u32>) -> Result<u32, String> {
    match minutes {
        None => Ok(DEFAULT_TIMEOUT_MINUTES),
        Some(m) if (1..=MAX_TIMEOUT_MINUTES).contains(&m) => Ok(m),
        Some(_) => Err("bench: timeout must be 1 to 90 minutes".to_string()),
    }
}

/// An idle timeout in minutes, defaulted and bounded.
///
/// # Errors
///
/// Refuses a value outside 5 to 120.
pub fn idle(minutes: Option<u32>) -> Result<u32, String> {
    match minutes {
        None => Ok(DEFAULT_IDLE_MINUTES),
        Some(m) if (MIN_IDLE_MINUTES..=MAX_IDLE_MINUTES).contains(&m) => Ok(m),
        Some(_) => Err("bench: idle timeout must be 5 to 120 minutes".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_from_labels() {
        assert_eq!(
            session_id("https://claude.ai/code/session_abc123"),
            Some("session_abc123")
        );
        assert_eq!(
            session_id("https://claude.ai/code/session_abc/"),
            Some("session_abc")
        );
        assert_eq!(session_id("https://claude.ai/code/session_a/b"), None);
        assert_eq!(session_id("https://claude.ai/code/session_a-b"), None);
        assert_eq!(session_id("https://claude.ai/code/session_a.b"), None);
        assert_eq!(session_id("https://claude.ai/code/session_"), None);
    }

    #[test]
    fn session_id_length_bound() {
        let ok = format!("session_{}", "a".repeat(56));
        let long = format!("session_{}", "a".repeat(57));
        assert!(valid_session_id(&ok));
        assert!(!valid_session_id(&long));
        assert_eq!(session_id(&format!("https://claude.ai/code/{long}")), None);
    }

    #[test]
    fn name_by_k() {
        assert_eq!(name("session_a", 0), Some("session_a.0".to_string()));
        assert_eq!(name("session_a", 2), Some("session_a.2".to_string()));
        assert_eq!(name("session_a", 3), None);
        assert_eq!(name("x", 0), None);
    }

    #[test]
    fn parse_name_round_trips() {
        for k in 0..3 {
            let n = name("session_a", k).unwrap();
            assert_eq!(parse_name(&n), Some(("session_a", k)));
        }
    }

    #[test]
    fn parse_name_refuses() {
        assert_eq!(parse_name("x.1"), None);
        assert_eq!(parse_name("session_a.12"), None);
        assert_eq!(parse_name("session_a.3"), None);
        assert_eq!(parse_name("session_a"), None);
    }

    #[test]
    fn scratch_round_trips() {
        let r = scratch_ref("session_a");
        assert_eq!(r, "refs/heads/scratch/session_a");
        assert_eq!(scratch_owner(&r), Some("session_a"));
    }

    #[test]
    fn scratch_owner_refuses() {
        assert_eq!(scratch_owner("refs/heads/scratch/session_a/b"), None);
        assert_eq!(scratch_owner("refs/heads/change/1-x"), None);
    }

    #[test]
    fn needs_clean_bounds() {
        assert!(!needs_clean(10_240, 10_000));
        assert!(needs_clean(10_241, 10_000));
        assert!(needs_clean(0, 3_071));
        assert!(!needs_clean(0, 3_072));
    }

    #[test]
    fn timeout_bounds() {
        assert_eq!(timeout(None), Ok(30));
        assert_eq!(timeout(Some(1)), Ok(1));
        assert_eq!(timeout(Some(90)), Ok(90));
        let e = "bench: timeout must be 1 to 90 minutes".to_string();
        assert_eq!(timeout(Some(0)), Err(e.clone()));
        assert_eq!(timeout(Some(91)), Err(e));
    }

    #[test]
    fn idle_bounds() {
        assert_eq!(idle(None), Ok(30));
        assert_eq!(idle(Some(5)), Ok(5));
        assert_eq!(idle(Some(120)), Ok(120));
        let e = "bench: idle timeout must be 5 to 120 minutes".to_string();
        assert_eq!(idle(Some(4)), Err(e.clone()));
        assert_eq!(idle(Some(121)), Err(e));
    }
}
