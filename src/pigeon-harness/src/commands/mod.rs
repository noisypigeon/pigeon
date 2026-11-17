//! Command dispatch.

#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "session")]
pub mod bench;
#[cfg(feature = "eviction")]
pub mod eviction;
#[cfg(feature = "infra")]
pub mod infra;
#[cfg(feature = "session")]
pub mod session;
#[cfg(feature = "signing")]
pub mod signing;
#[cfg(feature = "infra")]
pub mod workers;

use crate::cli::{Commands, DevArgs};

/// Every policy held, or the command succeeded.
pub const SUCCESS: i32 = 0;
/// A policy was violated.
pub const VIOLATION: i32 = 1;
/// An invalid policy or a usage error.
pub const INVALID: i32 = 2;

/// Runs a command and returns its exit code.
#[must_use]
pub fn dispatch(command: Commands) -> i32 {
    match command {
        #[cfg(feature = "eviction")]
        Commands::Eviction(c) => eviction::run(c),
        #[cfg(feature = "infra")]
        Commands::Infra(c) => infra::run(infra(c)),
        #[cfg(feature = "infra")]
        Commands::Sites(c) => infra::run(crate::cli::InfraOp::Sites(c)),
        #[cfg(feature = "infra")]
        Commands::Git(c) => infra::run(crate::cli::InfraOp::Git(c)),
        #[cfg(feature = "infra")]
        Commands::Workers(c) => infra::run(crate::cli::InfraOp::Workers(c)),
        #[cfg(feature = "infra")]
        Commands::Release(g) => infra::run(match g.command {
            Some(crate::cli::ReleaseCommand::Tags(a)) => crate::cli::InfraOp::Tags(a),
            None => crate::cli::InfraOp::Release(g.args),
        }),
        #[cfg(feature = "agent")]
        Commands::Instance(a) => agent::run(crate::cli::AgentOp::Instance(a)),
        #[cfg(feature = "agent")]
        Commands::Engines(a) => agent::run(crate::cli::AgentOp::Engines(a)),
        #[cfg(feature = "agent")]
        Commands::Ops(a) => agent::run(crate::cli::AgentOp::Operations(a)),
        #[cfg(feature = "agent")]
        Commands::Run(a) => agent::run(crate::cli::AgentOp::Run(a)),
        #[cfg(feature = "agent")]
        Commands::Decisions(a) => agent::run(crate::cli::AgentOp::Decisions(a)),
        #[cfg(feature = "agent")]
        Commands::Agent(c) => agent::run(match c {
            crate::cli::AgentCommand::Serve(a) => crate::cli::AgentOp::Serve(a),
            crate::cli::AgentCommand::Listen(a) => crate::cli::AgentOp::Jobs(a.into_jobs()),
            crate::cli::AgentCommand::PoolTools(a) => crate::cli::AgentOp::PoolTools(a),
        }),
        #[cfg(feature = "agent")]
        Commands::Keys(c) => agent::run(match c {
            crate::cli::KeysCommand::Totp => crate::cli::AgentOp::TotpSecret,
            crate::cli::KeysCommand::Enroll => crate::cli::AgentOp::EnrollSecret,
            crate::cli::KeysCommand::Vapid => crate::cli::AgentOp::VapidKey,
        }),
        #[cfg(feature = "signing")]
        Commands::Signing(c) => signing::run(c),
        #[cfg(feature = "session")]
        Commands::Session(c) => session::run(&c),
        Commands::Dev(a) => dev(a),
    }
}

/// `pigeon dev`.
fn dev(args: DevArgs) -> i32 {
    match args.command {
        #[cfg(feature = "infra")]
        Some(crate::cli::DevCommand::Scope(a)) => {
            return infra::run(crate::cli::InfraOp::Scope(a));
        }
        #[cfg(not(feature = "infra"))]
        Some(c) => match c {},
        None => {}
    }
    if args.list {
        crate::dev::list();
        return SUCCESS;
    }
    let Some(task) = args.task else {
        eprintln!("dev: name a task; `pigeon dev --list` lists them");
        return INVALID;
    };
    let here = std::env::current_dir().unwrap_or_default();
    let Some(root) = args.root.or_else(|| crate::dev::workspace_root(&here)) else {
        eprintln!("dev: no workspace Cargo.toml above {}", here.display());
        return INVALID;
    };
    // A repository root whose workspace is still under src/pigeon-cli; see step 4a.
    #[cfg(feature = "infra")]
    let root = ::agent::checks::scope::workspace(&root);
    crate::dev::run(&root, &task, args.dry_run)
}

#[cfg(feature = "infra")]
fn infra(c: crate::cli::InfraCommand) -> crate::cli::InfraOp {
    use crate::cli::{InfraCommand as I, InfraOp as C};
    match c {
        I::Validate(a) => C::Validate(a),
        I::Render(a) => C::Render(a),
        I::Plan(a) => C::Plan(a),
        I::Apply(a) => C::Apply(a),
        I::Show(a) => C::Show(a),
        I::Lock(a) => C::Lock(a),
        I::Capabilities(a) => C::Capabilities(a),
        I::Secrets(a) => C::Secrets(a),
    }
}
