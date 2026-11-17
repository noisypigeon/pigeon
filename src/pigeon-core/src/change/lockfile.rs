//! A restack's conflict in `Cargo.lock` alone, resolved by regenerating the change's lockfile;
//! see `.claude/decisions/2026-11-16-development-merge-path-measured-faster.md` §2.

use std::collections::BTreeSet;

/// The lockfile a restack regenerates when it alone conflicts.
pub const LOCKFILE: &str = "Cargo.lock";

/// A registry or git package a lockfile pins.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pinned {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Where it comes from.
    pub source: String,
    /// Its checksum, when its source gives one.
    pub checksum: Option<String>,
}

/// Whether a restack's conflicting paths are the lockfile alone.
#[must_use]
pub fn regenerates(conflicts: &[String]) -> bool {
    matches!(conflicts, [p] if p.as_str() == LOCKFILE)
}

/// The registry and git packages `text` pins, workspace packages (no `source`) aside; none when
/// it isn't a lockfile.
#[must_use]
pub fn pinned(text: &str) -> Option<BTreeSet<Pinned>> {
    let doc: toml::Value = toml::from_str(text).ok()?;
    let packages = match doc.get("package") {
        None => return Some(BTreeSet::new()),
        Some(p) => p.as_array()?,
    };
    let mut out = BTreeSet::new();
    for p in packages {
        let field = |k: &str| p.get(k).and_then(toml::Value::as_str).map(str::to_string);
        let Some(source) = field("source") else {
            continue;
        };
        out.insert(Pinned {
            name: field("name")?,
            version: field("version")?,
            source,
            checksum: field("checksum"),
        });
    }
    Some(out)
}

/// Whether `regenerated` pins exactly the registry and git packages a three-way merge of
/// `fork`, `main` and `change` keeps: those all three pin, and those either side added since
/// `fork`. An empty `fork` pins nothing.
#[must_use]
pub fn accepts(fork: &str, main: &str, change: &str, regenerated: &str) -> bool {
    let (Some(f), Some(m), Some(c), Some(r)) = (
        pinned(fork),
        pinned(main),
        pinned(change),
        pinned(regenerated),
    ) else {
        return false;
    };
    let kept = f.iter().filter(|p| m.contains(*p) && c.contains(*p));
    let added = m.difference(&f).chain(c.difference(&f));
    let merged: BTreeSet<&Pinned> = kept.chain(added).collect();
    r.iter().collect::<BTreeSet<&Pinned>>() == merged
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

    /// A lockfile of `packages` (name, version, checksum); one with no checksum is the
    /// workspace's.
    fn lock(packages: &[(&str, &str, Option<&str>)]) -> String {
        let mut text = String::from("version = 4\n");
        for (name, version, checksum) in packages {
            let _ = write!(
                text,
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
            );
            if let Some(sum) = checksum {
                let _ = write!(text, "source = \"{REGISTRY}\"\nchecksum = \"{sum}\"\n");
            }
        }
        text
    }

    const SERDE: (&str, &str, Option<&str>) = ("serde", "1.0.0", Some("aa"));
    const ITOA: (&str, &str, Option<&str>) = ("itoa", "1.0.1", Some("bb"));

    #[test]
    fn only_the_lockfile_alone_in_conflict_regenerates() {
        assert!(regenerates(&["Cargo.lock".into()]));
        assert!(!regenerates(&["Cargo.lock".into(), "a.txt".into()]));
        assert!(!regenerates(&[]));
        assert!(!regenerates(&["src/x/Cargo.lock".into()]));
    }

    #[test]
    fn workspace_packages_are_not_pinned() {
        let p = pinned(&lock(&[("pigeon-core", "0.1.0", None), SERDE])).expect("parses");
        let names: Vec<String> = p.into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["serde"]);
    }

    #[test]
    fn a_release_bump_and_a_new_workspace_crate_regenerated_are_accepted() {
        let fork = lock(&[("pigeon-agent", "0.1.43", None), SERDE]);
        let main = lock(&[("pigeon-agent", "0.1.44", None), SERDE]);
        let change = lock(&[
            ("pigeon-agent", "0.1.43", None),
            ("pigeon-new", "0.1.0", None),
            SERDE,
        ]);
        let regenerated = lock(&[
            ("pigeon-agent", "0.1.44", None),
            ("pigeon-new", "0.1.0", None),
            SERDE,
        ]);
        assert!(accepts(&fork, &main, &change, &regenerated));
    }

    #[test]
    fn a_package_either_side_added_is_kept() {
        let fork = lock(&[SERDE]);
        let added = lock(&[SERDE, ITOA]);
        assert!(accepts(&fork, &fork, &added, &added));
        assert!(accepts(&fork, &added, &fork, &added));
        assert!(!accepts(&fork, &fork, &added, &fork));
    }

    #[test]
    fn a_version_or_checksum_the_regeneration_changed_is_refused() {
        let fork = lock(&[SERDE]);
        assert!(!accepts(
            &fork,
            &fork,
            &fork,
            &lock(&[("serde", "1.0.1", Some("aa"))])
        ));
        assert!(!accepts(
            &fork,
            &fork,
            &fork,
            &lock(&[("serde", "1.0.0", Some("cc"))])
        ));
    }

    #[test]
    fn an_upgrade_on_main_the_regeneration_missed_is_refused() {
        let fork = lock(&[SERDE]);
        let main = lock(&[("serde", "1.0.1", Some("dd"))]);
        assert!(!accepts(&fork, &main, &fork, &fork));
        assert!(accepts(&fork, &main, &fork, &main));
    }

    #[test]
    fn a_package_one_side_removed_stays_removed() {
        let fork = lock(&[SERDE, ITOA]);
        let main = lock(&[SERDE]);
        assert!(accepts(&fork, &main, &fork, &main));
        assert!(!accepts(&fork, &main, &fork, &fork));
    }

    #[test]
    fn a_missing_fork_lockfile_pins_nothing_and_conflict_markers_are_refused() {
        let l = lock(&[SERDE]);
        assert!(accepts("", &l, &l, &l));
        assert!(!accepts(&l, &l, &l, "<<<<<<< ours\nversion = 4\n"));
    }
}
