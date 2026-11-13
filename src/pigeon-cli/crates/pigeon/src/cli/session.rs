//! `pigeon session` arguments; see
//! `.claude/decisions/2026-11-13-development-rust-session-event-follower.md`.

use clap::Subcommand;

/// `pigeon session` commands: a cloud agent session's view of the control plane.
#[derive(Subcommand, Debug)]
pub enum SessionCommand {
    /// Print one line per event the session must act on; resumes from the control plane's cursor
    Follow,
    /// Print where the session is, for the `SessionStart` hook
    Brief,
    /// Print the status line from the cache, never the network
    Statusline,
    /// Print a `systemMessage` when a change needs the session and nothing follows its events
    Stop,
}
