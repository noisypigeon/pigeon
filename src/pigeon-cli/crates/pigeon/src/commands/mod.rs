//! Command dispatch.

#[cfg(feature = "agent")]
pub mod backplane;
#[cfg(feature = "session")]
pub mod bench;
#[cfg(feature = "infra")]
pub mod cobot;
#[cfg(feature = "eviction")]
pub mod sentinel;
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
    if let Some((old, new)) = renamed(&command) {
        eprintln!("pigeon: `pigeon {old}` is now `pigeon {new}`");
    }
    match command {
        #[cfg(feature = "eviction")]
        Commands::Eviction(c) => sentinel::run(c),
        #[cfg(feature = "infra")]
        Commands::Infra(c) => cobot::run(infra(c)),
        #[cfg(feature = "infra")]
        Commands::Sites(c) => cobot::run(crate::cli::CobotCommand::Sites(c)),
        #[cfg(feature = "infra")]
        Commands::Git(c) => cobot::run(crate::cli::CobotCommand::Git(c)),
        #[cfg(feature = "infra")]
        Commands::Workers(c) => cobot::run(crate::cli::CobotCommand::Workers(c)),
        #[cfg(feature = "infra")]
        Commands::Release(g) => cobot::run(match g.command {
            Some(crate::cli::ReleaseCommand::Tags(a)) => crate::cli::CobotCommand::Tags(a),
            None => crate::cli::CobotCommand::Release(g.args),
        }),
        #[cfg(feature = "agent")]
        Commands::Instance(a) => backplane::run(crate::cli::BackplaneCommand::Instance(a)),
        #[cfg(feature = "agent")]
        Commands::Engines(a) => backplane::run(crate::cli::BackplaneCommand::Engines(a)),
        #[cfg(feature = "agent")]
        Commands::Ops(a) => backplane::run(crate::cli::BackplaneCommand::Operations(a)),
        #[cfg(feature = "agent")]
        Commands::Run(a) => backplane::run(crate::cli::BackplaneCommand::Run(a)),
        #[cfg(feature = "agent")]
        Commands::Decisions(a) => backplane::run(crate::cli::BackplaneCommand::Decisions(a)),
        #[cfg(feature = "agent")]
        Commands::Agent(c) => backplane::run(match c {
            crate::cli::AgentCommand::Serve(a) => crate::cli::BackplaneCommand::Serve(a),
            crate::cli::AgentCommand::Listen(a) => {
                crate::cli::BackplaneCommand::Jobs(a.into_jobs())
            }
        }),
        #[cfg(feature = "agent")]
        Commands::Keys(c) => backplane::run(match c {
            crate::cli::KeysCommand::Totp => crate::cli::BackplaneCommand::TotpSecret,
            crate::cli::KeysCommand::Enroll => crate::cli::BackplaneCommand::EnrollSecret,
            crate::cli::KeysCommand::Vapid => crate::cli::BackplaneCommand::VapidKey,
        }),
        #[cfg(feature = "signing")]
        Commands::Signing(c) => signing::run(c),
        #[cfg(feature = "session")]
        Commands::Session(c) => session::run(&c),
        Commands::Dev(a) => dev(a),
        #[cfg(feature = "eviction")]
        Commands::Sentinel(c) => sentinel::run(c),
        #[cfg(feature = "infra")]
        Commands::Cobot(c) => cobot::run(c),
        #[cfg(any(feature = "agent", feature = "signing"))]
        Commands::Backplane(c) => backplane_alias(c),
    }
}

/// An old spelling's words after `pigeon`, and the new ones; dispatch prints them on stderr.
#[must_use]
pub fn renamed(command: &Commands) -> Option<(&'static str, &'static str)> {
    match command {
        #[cfg(feature = "eviction")]
        Commands::Sentinel(crate::cli::SentinelCommand::Check(_)) => {
            Some(("sentinel check", "eviction check"))
        }
        #[cfg(feature = "infra")]
        Commands::Cobot(c) => Some(cobot_spelling(c)),
        #[cfg(any(feature = "agent", feature = "signing"))]
        Commands::Backplane(c) => Some(backplane_spelling(c)),
        _ => None,
    }
}

#[cfg(feature = "agent")]
fn backplane_alias(c: crate::cli::BackplaneCommand) -> i32 {
    backplane::run(c)
}

#[cfg(all(feature = "signing", not(feature = "agent")))]
fn backplane_alias(c: crate::cli::BackplaneCommand) -> i32 {
    match c {
        crate::cli::BackplaneCommand::Signing(c) => signing::run(c),
    }
}

/// `pigeon dev`.
fn dev(args: DevArgs) -> i32 {
    match args.command {
        #[cfg(feature = "infra")]
        Some(crate::cli::DevCommand::Scope(a)) => {
            return cobot::run(crate::cli::CobotCommand::Scope(a));
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
    let root = ::backplane::cobot::scope::workspace(&root);
    crate::dev::run(&root, &task, args.dry_run)
}

#[cfg(feature = "infra")]
fn infra(c: crate::cli::InfraCommand) -> crate::cli::CobotCommand {
    use crate::cli::{CobotCommand as C, InfraCommand as I};
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

/// A `pigeon cobot` command's old and new spellings.
#[cfg(feature = "infra")]
fn cobot_spelling(c: &crate::cli::CobotCommand) -> (&'static str, &'static str) {
    use crate::cli::{
        CobotCommand as C, GitCommand as G, SecretsCommand as S, SitesCommand as Si,
        WorkersCommand as W, WorkersImages, WorkersSecrets,
    };
    match c {
        C::Validate(_) => ("cobot validate", "infra validate"),
        C::Render(_) => ("cobot render", "infra render"),
        C::Plan(_) => ("cobot plan", "infra plan"),
        C::Apply(_) => ("cobot apply", "infra apply"),
        C::Show(_) => ("cobot show", "infra show"),
        C::Lock(_) => ("cobot lock", "infra lock"),
        C::Capabilities(_) => ("cobot capabilities", "infra capabilities"),
        C::Secrets(S::Set(_)) => ("cobot secrets set", "infra secrets set"),
        C::Secrets(S::Check(_)) => ("cobot secrets check", "infra secrets check"),
        C::Sites(Si::Build(_)) => ("cobot sites build", "sites build"),
        C::Sites(Si::Publish(_)) => ("cobot sites publish", "sites publish"),
        C::Git(g) => match g {
            G::Apply(_) => ("cobot git apply", "git apply"),
            G::Status(_) => ("cobot git status", "git status"),
            G::Push(_) => ("cobot git push", "git push"),
            G::Fetch(_) => ("cobot git fetch", "git fetch"),
            G::Clone(_) => ("cobot git clone", "git clone"),
            G::Mirror(_) => ("cobot git mirror", "git mirror"),
            G::Sync(_) => ("cobot git sync", "git sync"),
        },
        C::Workers(w) => match w {
            W::Validate(_) => ("cobot workers validate", "workers validate"),
            W::Permissions(_) => ("cobot workers permissions", "workers permissions"),
            W::Render(_) => ("cobot workers render", "workers render"),
            W::Plan(_) => ("cobot workers plan", "workers plan"),
            W::Deploy(_) => ("cobot workers deploy", "workers deploy"),
            W::Status(_) => ("cobot workers status", "workers status"),
            W::Secrets(WorkersSecrets::Put(_)) => {
                ("cobot workers secrets put", "workers secrets put")
            }
            W::Teardown(_) => ("cobot workers teardown", "workers teardown"),
            W::Outputs(_) => ("cobot workers outputs", "workers outputs"),
            W::Images(WorkersImages::Push(_)) => {
                ("cobot workers images push", "workers images push")
            }
        },
        C::Scope(_) => ("cobot scope", "dev scope"),
        C::Release(_) => ("cobot release", "release"),
        C::Tags(_) => ("cobot tags", "release tags"),
    }
}

/// A `pigeon backplane` command's old and new spellings.
#[cfg(any(feature = "agent", feature = "signing"))]
fn backplane_spelling(c: &crate::cli::BackplaneCommand) -> (&'static str, &'static str) {
    use crate::cli::BackplaneCommand as B;
    match c {
        #[cfg(feature = "agent")]
        B::Instance(_) => ("backplane instance", "instance"),
        #[cfg(feature = "agent")]
        B::Engines(_) => ("backplane engines", "engines"),
        #[cfg(feature = "agent")]
        B::Operations(_) => ("backplane operations", "ops"),
        #[cfg(feature = "agent")]
        B::Run(_) => ("backplane run", "run"),
        #[cfg(feature = "agent")]
        B::Decisions(_) => ("backplane decisions", "decisions"),
        #[cfg(feature = "agent")]
        B::Serve(_) => ("backplane serve", "agent serve"),
        #[cfg(feature = "agent")]
        B::Jobs(_) => ("backplane jobs", "agent listen"),
        #[cfg(feature = "agent")]
        B::TotpSecret => ("backplane totp-secret", "keys totp"),
        #[cfg(feature = "agent")]
        B::EnrollSecret => ("backplane enroll-secret", "keys enroll"),
        #[cfg(feature = "agent")]
        B::VapidKey => ("backplane vapid-key", "keys vapid"),
        #[cfg(feature = "signing")]
        B::Signing(s) => match s {
            crate::cli::SigningCommand::Load(_) => ("backplane signing load", "signing load"),
            crate::cli::SigningCommand::Upload(_) => ("backplane signing upload", "signing upload"),
            crate::cli::SigningCommand::Clean(_) => ("backplane signing clean", "signing clean"),
        },
    }
}
