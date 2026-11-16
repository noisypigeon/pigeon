//! `pigeon session` arguments; see
//! `.claude/decisions/2026-11-13-development-rust-session-event-follower.md` and
//! `.claude/decisions/2026-11-15-development-no-python-rust-tooling.md`.

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
    /// Print `create_session`'s arguments, as JSON, for an executor of a brief
    Executor {
        /// The brief, `b<n>`
        brief: String,
        /// A tag to add, after the brief's own
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
    },
    /// Fetch main's newest `pigeon` build to `~/.cache/pigeon/bin/pigeon`, SHA-256 checked
    Install,
    /// Build and test on this session's benches, never here; see .claude/decisions/2026-11-16-development-session-bench-containers.md
    Bench {
        /// The bench command
        #[command(subcommand)]
        command: BenchCommand,
    },
}

/// `pigeon session bench` commands.
#[derive(Subcommand, Debug)]
pub enum BenchCommand {
    /// Push the working tree, uncommitted files included, to this session's scratch ref; print its commit
    Push,
    /// Push, run ARGV on a bench, stream its log, and exit with its exit code
    Run {
        /// The bench, 0 to 2
        #[arg(long, default_value_t = 0)]
        bench: u8,
        /// Minutes before it is killed, 1 to 90
        #[arg(long)]
        timeout: Option<u32>,
        /// The command, after `--`
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// Push, then run the change's checks across BENCHES benches, as the gate selects them
    Ci {
        /// How many benches, 1 to 3
        #[arg(long, default_value_t = 3)]
        benches: u8,
    },
    /// A run's state and its log from OFFSET
    Status {
        /// The run
        run: String,
        /// The bench
        #[arg(long, default_value_t = 0)]
        bench: u8,
        /// Log offset
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    /// Cancel a run
    Cancel {
        /// The run
        run: String,
        /// The bench
        #[arg(long, default_value_t = 0)]
        bench: u8,
    },
    /// Reset a bench: untracked files and the workspace's build; --hard destroys its Container
    Reset {
        /// The bench
        #[arg(long, default_value_t = 0)]
        bench: u8,
        /// Destroy the Container
        #[arg(long)]
        hard: bool,
    },
    /// This session's benches
    List,
    /// Start a bench without running anything
    Warm {
        /// The bench
        #[arg(long, default_value_t = 0)]
        bench: u8,
    },
}
