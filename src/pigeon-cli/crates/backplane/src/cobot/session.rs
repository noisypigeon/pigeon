//! A run's context, with its state pulled from wherever `--state` names; shared by the CLI and
//! backplane so neither assembles one itself.

use std::path::{Path, PathBuf};

use crate::cobot::state::{Mirror, Store};
use crate::cobot::{Context, Error};

/// Where an R2 store's state is mirrored for a run, under the root.
pub const MIRROR_DIR: &str = ".cobot/state-r2";

/// A run's context and, for R2 state, the mirror it was pulled into.
#[derive(Debug)]
pub struct Session {
    /// The run's context.
    pub ctx: Context,
    /// The R2 mirror, if `--state` named one.
    pub mirror: Option<Mirror>,
}

/// The time now, in Unix seconds.
#[must_use]
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Opens a session: parses `state` (a directory, or `r2://<bucket>/<prefix>`), pulls R2 state
/// into the root's mirror, and reads the passphrases and runner credentials through `var`.
///
/// # Errors
///
/// [`Error::State`] for a bad `state` or an R2 failure; [`Error::Passphrase`] without a valid
/// state passphrase.
pub fn open(
    root: &Path,
    providers: &Path,
    state: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Session, Error> {
    let (dir, mirror) = match Store::parse(state, &var).map_err(Error::State)? {
        Store::Dir(dir) => (dir, None),
        Store::R2 { bucket, prefix } => {
            let m = Mirror::pull(bucket, &prefix, &root.join(MIRROR_DIR), now())?;
            (m.dir().to_path_buf(), Some(m))
        }
    };
    let ctx = Context {
        root: root.to_path_buf(),
        providers: providers.to_path_buf(),
        state: dir,
        state_passphrase: crate::cobot::passphrase(var(crate::cobot::PASSPHRASE_VAR))?,
        secrets_passphrase: var(crate::cobot::secrets::PASSPHRASE_VAR),
        workload_secrets_passphrase: var(crate::cobot::secrets::WORKLOAD_PASSPHRASE_VAR),
        today: crate::cobot::generations::today(),
        cloudflare_token: crate::cobot::cloudflare::token(
            var(crate::cobot::CREDENTIALS_VAR).as_deref(),
            var(crate::cobot::cloudflare::TOKEN_VAR),
        ),
        cloudflare_api: None,
        platform: None,
        adopt: false,
        provider_config: None,
        provider_token: false,
        credentials: var(crate::cobot::CREDENTIALS_VAR),
    };
    Ok(Session { ctx, mirror })
}

/// The directory `render` writes against for `state`, without pulling anything.
///
/// # Errors
///
/// [`Error::State`] for a bad `state`.
pub fn render_dir(
    root: &Path,
    state: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf, Error> {
    Ok(match Store::parse(state, var).map_err(Error::State)? {
        Store::Dir(dir) => dir,
        Store::R2 { .. } => root.join(MIRROR_DIR),
    })
}

impl Session {
    /// Writes R2 state back; the number of objects written, or `None` for a directory.
    ///
    /// # Errors
    ///
    /// [`Error::State`] if another writer changed state, or R2 fails.
    pub fn push(&mut self) -> Result<Option<usize>, Error> {
        push(&mut self.mirror)
    }

    /// Applies stored plan `id`, writing R2 state back at each checkpoint (control-plane ADR §7).
    ///
    /// # Errors
    ///
    /// As [`crate::cobot::apply_stored`], and [`Error::State`] if a write back fails.
    pub fn apply_stored(
        &mut self,
        id: &str,
        now: u64,
    ) -> Result<Vec<crate::cobot::UnitPlan>, Error> {
        let mirror = &mut self.mirror;
        crate::cobot::apply_stored_with(&self.ctx, id, now, &mut || push(mirror).map(|_| ()))
    }
}

fn push(mirror: &mut Option<Mirror>) -> Result<Option<usize>, Error> {
    mirror
        .as_mut()
        .map(|m| m.push(now()).map(|w| w.len()))
        .transpose()
}
