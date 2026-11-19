//! When a failed deploy of `main` holds Auto; see
//! .claude/decisions/2026-11-19-security-auto-held-when-platform-down.md §2.

use crate::control::{Deployed, Outcome};
use crate::deploy::serving::Serving;

/// Failed deploys in a row that hold Auto, whatever serves.
pub const STREAK_FAILURES: u32 = 3;
/// How long `main` may go without a passed deploy before a failure holds Auto, in seconds.
pub const STREAK_MAX: u64 = 2 * 3_600;

/// The failed deploys of `main` since the last that passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    /// How many, the latest included.
    pub failures: u32,
    /// When the first of them finished, in Unix seconds.
    pub since: u64,
}

impl Streak {
    /// The streak at the head of `history`, newest first, skipping superseded deploys.
    #[must_use]
    pub fn of(history: &[Deployed]) -> Self {
        let mut s = Self {
            failures: 0,
            since: 0,
        };
        for d in history {
            match d.outcome {
                Outcome::Failed => (s.failures, s.since) = (s.failures + 1, d.at),
                Outcome::Superseded | Outcome::Running => {}
                Outcome::Passed => break,
            }
        }
        s
    }

    /// What holds Auto next, for a failure that doesn't.
    #[must_use]
    pub fn next(&self) -> String {
        format!(
            "{} of {STREAK_FAILURES} failures, or {} hours without a deploy, hold Auto",
            self.failures,
            STREAK_MAX / 3_600
        )
    }
}

/// Why a failed deploy holds Auto, or `None` while a colour serves and `main` isn't stuck: no
/// `serving` report, the platform down, a pipeline change, or a long or repeated streak.
#[must_use]
pub fn holds(
    serving: Option<&Serving>,
    streak: Streak,
    pipeline: bool,
    now: u64,
) -> Option<String> {
    let Some(s) = serving else {
        return Some("the deploy didn't report what serves".into());
    };
    if let Some(why) = s.down() {
        return Some(why);
    }
    if pipeline {
        return Some("it changed the deploy pipeline".into());
    }
    if streak.failures >= STREAK_FAILURES {
        return Some(format!(
            "{} deploys of main failed in a row",
            streak.failures
        ));
    }
    if streak.failures > 0 && now.saturating_sub(streak.since) >= STREAK_MAX {
        return Some(format!(
            "main has not deployed for {} hours",
            STREAK_MAX / 3_600
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up() -> Serving {
        Serving {
            lease: "pigeon-control".into(),
            front: true,
            colors: [("pigeon-control".to_string(), true)].into(),
        }
    }

    fn deployed(outcome: Outcome, at: u64) -> Deployed {
        serde_json::from_value(serde_json::json!({
            "sha": "a", "outcome": outcome, "at": at, "published": [], "violations": []
        }))
        .unwrap()
    }

    #[test]
    fn a_failure_holds_only_when_down_unreported_pipeline_or_stuck() {
        let one = Streak {
            failures: 1,
            since: 100,
        };
        assert_eq!(holds(Some(&up()), one, false, 200), None);
        assert!(holds(None, one, false, 200).is_some());
        let mut down = up();
        down.colors.insert("pigeon-control".into(), false);
        assert!(holds(Some(&down), one, false, 200).is_some());
        assert!(holds(Some(&up()), one, true, 200).is_some());
        let three = Streak {
            failures: STREAK_FAILURES,
            since: 100,
        };
        assert!(holds(Some(&up()), three, false, 200).is_some());
        assert!(holds(Some(&up()), one, false, 100 + STREAK_MAX).is_some());
    }

    #[test]
    fn the_streak_runs_back_to_the_last_passed_deploy() {
        let h = [
            deployed(Outcome::Failed, 30),
            deployed(Outcome::Superseded, 20),
            deployed(Outcome::Failed, 10),
            deployed(Outcome::Passed, 5),
            deployed(Outcome::Failed, 1),
        ];
        assert_eq!(
            Streak::of(&h),
            Streak {
                failures: 2,
                since: 10
            }
        );
    }
}
