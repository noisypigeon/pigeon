//! Resolves a policy's target and runs its rules against it.

use std::fs;
use std::path::Path;

use crate::policy::{NamedTarget, Target};
use crate::report::{Report, Violation};
use crate::rules;
use crate::{Error, LoadedPolicy, PullRequest};

/// Runs one policy. Returns `false` when it was skipped.
pub fn run(
    loaded: &LoadedPolicy,
    root: &Path,
    pull_request: Option<&PullRequest>,
    report: &mut Report,
) -> Result<bool, Error> {
    let violation = |path: &Path, message: String| Violation {
        path: path.display().to_string(),
        line: None,
        policy: loaded.name.clone(),
        message,
    };
    match &loaded.policy.target {
        Target::Named(NamedTarget::Tree) => {
            rules::run_tree(loaded, root, &mut report.violations);
        }
        Target::Named(NamedTarget::PullRequest) => {
            let Some(pr) = pull_request else {
                return Ok(false);
            };
            rules::run_pull_request(loaded, pr, root, &mut report.violations);
        }
        Target::File(t) => match fs::read_to_string(root.join(&t.file)) {
            Ok(content) => {
                rules::run_document(loaded, &t.file, &content, root, &mut report.violations);
            }
            Err(e) => report
                .violations
                .push(violation(&t.file, format!("cannot read: {e}"))),
        },
        Target::Files(t) => {
            for file in &t.files {
                match fs::read_to_string(root.join(file)) {
                    Ok(content) => {
                        rules::run_document(loaded, file, &content, root, &mut report.violations);
                    }
                    Err(e) if t.optional && e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => report
                        .violations
                        .push(violation(file, format!("cannot read: {e}"))),
                }
            }
        }
        Target::Dir(t) => {
            let dir = root.join(&t.dir);
            let mut names = Vec::new();
            match fs::read_dir(&dir) {
                Ok(entries) => {
                    for entry in entries {
                        let entry = entry.map_err(|source| Error::Io {
                            path: dir.clone(),
                            source,
                        })?;
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if entry.path().is_file() && !t.exclude.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                Err(e) => {
                    report
                        .violations
                        .push(violation(&t.dir, format!("cannot list: {e}")));
                    return Ok(true);
                }
            }
            names.sort();
            if names.len() < t.min {
                report.violations.push(violation(
                    &t.dir,
                    format!("holds {} file(s); at least {} required", names.len(), t.min),
                ));
            }
            for name in names {
                let rel = t.dir.join(&name);
                match fs::read_to_string(root.join(&rel)) {
                    Ok(content) => {
                        rules::run_document(loaded, &rel, &content, root, &mut report.violations);
                    }
                    Err(e) => report
                        .violations
                        .push(violation(&rel, format!("cannot read: {e}"))),
                }
            }
        }
    }
    Ok(true)
}
