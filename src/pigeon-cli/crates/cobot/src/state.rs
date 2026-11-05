//! Where state lives: a directory, or an R2 prefix mirrored into one; see the phase 3
//! amendment §4 of `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;
use crate::s3::{self, Bucket, Credentials, Precondition};

/// The scheme naming an R2 store.
pub const SCHEME: &str = "r2://";
/// The R2 endpoint, `https://<account>.<jurisdiction>.r2.cloudflarestorage.com`.
pub const ENDPOINT_VAR: &str = "COBOT_STATE_R2_ENDPOINT";
/// The R2 access key id.
pub const ACCESS_KEY_VAR: &str = "COBOT_STATE_R2_ACCESS_KEY_ID";
/// The R2 secret access key.
pub const SECRET_KEY_VAR: &str = "COBOT_STATE_R2_SECRET_ACCESS_KEY";

/// The objects' content type; state is encrypted JSON.
const CONTENT_TYPE: &str = "application/octet-stream";

/// The body marking removed state; a pull skips it.
pub const TOMBSTONE: &[u8] = b"cobot:removed\n";

/// A state store named by `--state`.
#[derive(Debug, Clone)]
pub enum Store {
    /// A local directory, such as a checkout of the state repository.
    Dir(PathBuf),
    /// An R2 bucket and the prefix its units' state lives under.
    R2 {
        /// The bucket.
        bucket: Bucket,
        /// The prefix, without slashes at either end.
        prefix: String,
    },
}

impl Store {
    /// Parses `spec`: `r2://<bucket>/<prefix>`, with its endpoint and keys from `var`, or a
    /// directory.
    ///
    /// # Errors
    ///
    /// A message naming what is malformed or missing; never a value.
    pub fn parse(spec: &str, var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let Some(rest) = spec.strip_prefix(SCHEME) else {
            return Ok(Self::Dir(PathBuf::from(spec)));
        };
        let (name, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        let prefix = prefix.trim_end_matches('/');
        let ok = |p: &str| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                && p != "."
                && p != ".."
        };
        if !ok(name) || !prefix.split('/').all(ok) {
            return Err(format!("`--state` must be {SCHEME}<bucket>/<prefix>"));
        }
        let need = |k: &str| {
            var(k)
                .filter(|v| !v.is_empty())
                .ok_or(format!("{k} is not set"))
        };
        let endpoint = need(ENDPOINT_VAR)?;
        if !secure(&endpoint) {
            return Err(format!(
                "{ENDPOINT_VAR} must be https://, or http:// only to a loopback test server"
            ));
        }
        Ok(Self::R2 {
            bucket: Bucket {
                endpoint,
                name: name.to_string(),
                creds: Credentials {
                    access_key: need(ACCESS_KEY_VAR)?,
                    secret_key: need(SECRET_KEY_VAR)?,
                },
            },
            prefix: prefix.to_string(),
        })
    }
}

/// A local copy of an R2 prefix, with the `ETag` each object had when it was read.
#[derive(Debug)]
pub struct Mirror {
    bucket: Bucket,
    prefix: String,
    dir: PathBuf,
    /// Each object read, by relative path: its `ETag`, and its body's SHA-256 (`None` for a
    /// tombstone).
    read: BTreeMap<String, (String, Option<String>)>,
}

fn fail(e: &s3::Failure) -> Error {
    Error::State(e.to_string())
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

impl Mirror {
    /// Empties `dir`, then copies every object under `prefix/` into it.
    ///
    /// # Errors
    ///
    /// [`Error::State`] if R2 fails, or [`Error::Io`] if `dir` cannot be written.
    pub fn pull(bucket: Bucket, prefix: &str, dir: &Path, now: u64) -> Result<Self, Error> {
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(io(dir))?;
        }
        fs::create_dir_all(dir).map_err(io(dir))?;
        let lead = format!("{prefix}/");
        let mut read = BTreeMap::new();
        for object in bucket.list(now).map_err(|e| fail(&e))? {
            let Some(rel) = object.key.strip_prefix(&lead) else {
                continue;
            };
            if !safe(rel) {
                return Err(Error::State(format!(
                    "refusing object key `{}`",
                    object.key
                )));
            }
            let Some((body, etag)) = bucket.get_tagged(&object.key, now).map_err(|e| fail(&e))?
            else {
                continue;
            };
            if body == TOMBSTONE {
                read.insert(rel.to_string(), (etag, None));
                continue;
            }
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(io(parent))?;
            }
            fs::write(&path, &body).map_err(io(&path))?;
            read.insert(rel.to_string(), (etag, Some(s3::sha256_hex(&body))));
        }
        Ok(Self {
            bucket,
            prefix: prefix.to_string(),
            dir: dir.to_path_buf(),
            read,
        })
    }

    /// The local directory to run against.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes back what changed: each changed object only if its `ETag` is unchanged, each new
    /// one only if none appeared, and a [`TOMBSTONE`] over each removed one only if its `ETag`
    /// is unchanged. The keys written. What it writes becomes what was read, so a run may push
    /// again.
    ///
    /// # Errors
    ///
    /// [`Error::State`] if another writer changed an object since it was last read or written,
    /// or R2 fails.
    pub fn push(&mut self, now: u64) -> Result<Vec<String>, Error> {
        let mut local = BTreeMap::new();
        files(&self.dir, &self.dir, &mut local)?;
        let mut changed = Vec::new();
        for (rel, body) in &local {
            let key = format!("{}/{rel}", self.prefix);
            let when = match self.read.get(rel) {
                Some((_, Some(sha))) if *sha == s3::sha256_hex(body) => continue,
                Some((etag, _)) => Precondition::Matches(etag.clone()),
                None => Precondition::Absent,
            };
            let Some(etag) = self
                .bucket
                .put_if(&key, body, CONTENT_TYPE, &when, now)
                .map_err(|e| fail(&e))?
            else {
                return Err(Error::State(format!(
                    "`{key}` changed in R2 since it was read; not overwriting it"
                )));
            };
            self.read
                .insert(rel.clone(), (etag, Some(s3::sha256_hex(body))));
            changed.push(key);
        }
        let removed: Vec<(String, String)> = self
            .read
            .iter()
            .filter(|(r, (_, sha))| sha.is_some() && !local.contains_key(*r))
            .map(|(r, (etag, _))| (r.clone(), etag.clone()))
            .collect();
        for (rel, etag) in removed {
            let key = format!("{}/{rel}", self.prefix);
            let when = Precondition::Matches(etag);
            let Some(etag) = self
                .bucket
                .put_if(&key, TOMBSTONE, CONTENT_TYPE, &when, now)
                .map_err(|e| fail(&e))?
            else {
                return Err(Error::State(format!(
                    "`{key}` changed in R2 since it was read; not removing it"
                )));
            };
            self.read.insert(rel, (etag, None));
            changed.push(key);
        }
        Ok(changed)
    }
}

/// Whether an object key's relative part stays inside the mirror on any platform: `/`-separated
/// segments of `[A-Za-z0-9._-]`, none `.` or `..`, so never a backslash, drive letter or root.
fn safe(rel: &str) -> bool {
    !rel.is_empty()
        && rel.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
}

/// Whether `endpoint` is TLS, or plain HTTP only to a loopback host (tests).
fn secure(endpoint: &str) -> bool {
    if endpoint.starts_with("https://") {
        return true;
    }
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    !authority.contains('@') && matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

fn files(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
    for entry in fs::read_dir(dir).map_err(io(dir))? {
        let path = entry.map_err(io(dir))?.path();
        if path.is_dir() {
            files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.insert(rel, fs::read(&path).map_err(io(&path))?);
        }
    }
    Ok(())
}
