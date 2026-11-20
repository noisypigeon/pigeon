//! A promote's state machine: drain the primary, flip the lease, watch the new primary, flip back on a failure; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20 amendment.

use serde::{Deserialize, Serialize};

use super::Lease;

/// How long a drain waits for the primary's running jobs, and how long the watch lasts, in seconds.
pub const WINDOW_S: u64 = 600;

/// A promote in progress, kept by `MainRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Promote {
    /// The colour that served when it began.
    pub from: String,
    /// The colour it promotes.
    pub to: String,
    /// Draining or watching.
    pub phase: Phase,
    /// When the phase began, in seconds.
    pub since: u64,
}

/// A promote's phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The primary takes no new acts; its running jobs finish.
    Draining,
    /// The lease names the new primary, which is probed.
    Watching,
}

/// What `MainRun` saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A deploy's standby `color` passed its checks; `hold` is the owner's `promote.hold`.
    StandbyPassed {
        /// The standby.
        color: String,
        /// Whether the owner holds promotion.
        hold: bool,
    },
    /// The owner's `lease.flip` to `color`.
    Flip {
        /// The colour to promote.
        color: String,
    },
    /// The primary's running jobs.
    Running(u64),
    /// One probe of the new primary on its own host and of the platform host through the front: both passed.
    Probed(bool),
}

/// What a step decides: the promote kept after it, the lease to write behind `write`'s fence, and the event to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The promote after this step; None when none is in progress.
    pub promote: Option<Promote>,
    /// The lease to write, fenced on the lease's epoch before this step.
    pub lease: Option<Lease>,
    /// The event naming the transition.
    pub event: Option<&'static str>,
}

const SERVING: &str = "serving";
const PROMOTING: &str = "promoting";

/// The step `input` takes, at `now` seconds, from `lease` and `promote`; a stale promote (the lease names neither side as it expects) is dropped.
#[must_use]
pub fn step(lease: &Lease, promote: Option<&Promote>, input: &Input, now: u64) -> Step {
    let none = |promote: Option<Promote>| Step {
        promote,
        lease: None,
        event: None,
    };
    let Some(p) = promote else {
        let (Input::StandbyPassed {
            color: to,
            hold: false,
        }
        | Input::Flip { color: to }) = input
        else {
            return none(None);
        };
        if lease.state != SERVING || *to == lease.color {
            return none(None);
        }
        return Step {
            promote: Some(Promote {
                from: lease.color.clone(),
                to: to.clone(),
                phase: Phase::Draining,
                since: now,
            }),
            lease: Some(Lease {
                state: PROMOTING.into(),
                ..lease.clone()
            }),
            event: Some("lease.promoting"),
        };
    };
    match p.phase {
        Phase::Draining => {
            if lease.color != p.from || lease.state != PROMOTING {
                return none(None);
            }
            match input {
                Input::Running(0) => Step {
                    promote: Some(Promote {
                        phase: Phase::Watching,
                        since: now,
                        ..p.clone()
                    }),
                    lease: Some(Lease {
                        color: p.to.clone(),
                        epoch: lease.epoch + 1,
                        state: SERVING.into(),
                    }),
                    event: Some("lease.promoted"),
                },
                Input::Running(_) if now.saturating_sub(p.since) >= WINDOW_S => Step {
                    promote: None,
                    lease: Some(Lease {
                        state: SERVING.into(),
                        ..lease.clone()
                    }),
                    event: Some("lease.promote_aborted"),
                },
                _ => none(Some(p.clone())),
            }
        }
        Phase::Watching => {
            if lease.color != p.to {
                return none(None);
            }
            match input {
                Input::Probed(false) => Step {
                    promote: None,
                    lease: Some(Lease {
                        color: p.from.clone(),
                        epoch: lease.epoch + 1,
                        state: SERVING.into(),
                    }),
                    event: Some("lease.flipped_back"),
                },
                Input::Probed(true) if now.saturating_sub(p.since) >= WINDOW_S => none(None),
                _ => none(Some(p.clone())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(color: &str, epoch: u64, state: &str) -> Lease {
        Lease {
            color: color.into(),
            epoch,
            state: state.into(),
        }
    }

    fn passed(color: &str, hold: bool) -> Input {
        Input::StandbyPassed {
            color: color.into(),
            hold,
        }
    }

    #[test]
    fn a_promote_drains_flips_watches_and_ends() {
        let l = lease("pigeon-control", 1, "serving");
        let s = step(&l, None, &passed("pigeon-green", false), 100);
        assert_eq!(s.event, Some("lease.promoting"));
        assert_eq!(s.lease, Some(lease("pigeon-control", 1, "promoting")));
        let l = s.lease.unwrap();
        let p = s.promote.unwrap();
        assert_eq!((p.phase, p.since), (Phase::Draining, 100));

        let s = step(&l, Some(&p), &Input::Running(2), 200);
        assert_eq!(
            s,
            Step {
                promote: Some(p.clone()),
                lease: None,
                event: None
            }
        );

        let s = step(&l, Some(&p), &Input::Running(0), 300);
        assert_eq!(s.event, Some("lease.promoted"));
        assert_eq!(s.lease, Some(lease("pigeon-green", 2, "serving")));
        let l = s.lease.unwrap();
        let p = s.promote.unwrap();
        assert_eq!((p.phase, p.since), (Phase::Watching, 300));

        let s = step(&l, Some(&p), &Input::Probed(true), 600);
        assert_eq!(s.promote, Some(p.clone()));
        let s = step(&l, Some(&p), &Input::Probed(true), 900);
        assert_eq!(
            s,
            Step {
                promote: None,
                lease: None,
                event: None
            }
        );
    }

    #[test]
    fn a_failed_probe_flips_back_to_the_warm_colour() {
        let l = lease("pigeon-green", 2, "serving");
        let p = Promote {
            from: "pigeon-control".into(),
            to: "pigeon-green".into(),
            phase: Phase::Watching,
            since: 300,
        };
        let s = step(&l, Some(&p), &Input::Probed(false), 360);
        assert_eq!(s.event, Some("lease.flipped_back"));
        assert_eq!(s.lease, Some(lease("pigeon-control", 3, "serving")));
        assert_eq!(s.promote, None);
    }

    #[test]
    fn a_drain_past_its_window_aborts_and_never_flips() {
        let l = lease("pigeon-control", 1, "promoting");
        let p = Promote {
            from: "pigeon-control".into(),
            to: "pigeon-green".into(),
            phase: Phase::Draining,
            since: 100,
        };
        let s = step(&l, Some(&p), &Input::Running(1), 100 + WINDOW_S);
        assert_eq!(s.event, Some("lease.promote_aborted"));
        assert_eq!(s.lease, Some(lease("pigeon-control", 1, "serving")));
        assert_eq!(s.promote, None);
    }

    #[test]
    fn enumeration_what_starts_a_promote_and_what_doesnt() {
        let l = lease("pigeon-control", 1, "serving");
        let flip = Input::Flip {
            color: "pigeon-green".into(),
        };
        assert_eq!(step(&l, None, &flip, 0).event, Some("lease.promoting"));
        for (lease, input) in [
            (l.clone(), passed("pigeon-green", true)),
            (l.clone(), passed("pigeon-control", false)),
            (lease("pigeon-control", 1, "promoting"), flip.clone()),
            (l.clone(), Input::Running(0)),
            (l.clone(), Input::Probed(false)),
        ] {
            assert_eq!(
                step(&lease, None, &input, 0),
                Step {
                    promote: None,
                    lease: None,
                    event: None
                }
            );
        }
    }

    #[test]
    fn a_promote_the_lease_no_longer_matches_is_dropped() {
        let p = Promote {
            from: "pigeon-control".into(),
            to: "pigeon-green".into(),
            phase: Phase::Draining,
            since: 0,
        };
        let moved = lease("pigeon-blue", 5, "serving");
        assert_eq!(step(&moved, Some(&p), &Input::Running(0), 1).promote, None);
        let w = Promote {
            phase: Phase::Watching,
            ..p
        };
        assert_eq!(step(&moved, Some(&w), &Input::Probed(false), 1).lease, None);
    }
}
