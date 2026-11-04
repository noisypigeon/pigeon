//! A run's context, with its state pulled from wherever `--state` names; shared by the CLI and
//! backplane so neither assembles one itself.

use std::path::{Path, PathBuf};

use crate::state::{Mirror, Store};
use crate::{Context, Error};

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
        state_passphrase: crate::passphrase(var(crate::PASSPHRASE_VAR))?,
        secrets_passphrase: var(crate::secrets::PASSPHRASE_VAR),
        workload_secrets_passphrase: var(crate::secrets::WORKLOAD_PASSPHRASE_VAR),
        today: crate::generations::today(),
        credentials: var(crate::CREDENTIALS_VAR),
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
    pub fn push(&self) -> Result<Option<usize>, Error> {
        self.mirror
            .as_ref()
            .map(|m| m.push(now()).map(|w| w.len()))
            .transpose()
    }
}
