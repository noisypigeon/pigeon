//! `pigeon cobot sites`: build each static site with its pinned template, and
//! publish it to its R2 bucket; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §5 and §7,
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md`, and
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §1.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;

use crate::cobot::apex;
use crate::cobot::s3::{self, Bucket, Credentials};
use crate::cobot::static_site;
use crate::cobot::{Error, WORK_DIR, unit};

pub use crate::core::sites::assemble::*;
pub use crate::core::sites::checks::*;
pub use crate::core::sites::layout::*;
pub use crate::core::sites::publish::*;

/// Where templates live, as `<TEMPLATES>/<name>`.
pub const TEMPLATES: &str = "src/site-templates";
/// A template's theme and tag prefix, as `<PREFIX>-<name>`.
pub const PREFIX: &str = "site-template";

/// The version that reads the template from the instance's own tree at `HEAD`, while releases
/// pause; see the 2026-11-11 amendment of
/// `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md`.
pub const TREE: &str = "tree";
/// The post naming rule `YYYY-MM-DD-<slug>.md`.
pub const DATE_SLUG: &str = "date-slug";

/// A static site, as found under `workloads/`.
#[derive(Debug, Clone)]
pub struct Site {
    /// The unit path, `workloads/<zone>/<workload>`.
    pub unit: String,
    /// The unit's directory.
    pub dir: PathBuf,
    /// Its content: `site/` in its directory, or a manifest unit's `content`.
    pub content: PathBuf,
    /// The zone.
    pub zone: String,
    /// The workload.
    pub workload: String,
    /// The template it names, `static_site.template.name`.
    pub template: String,
    /// The pinned template version.
    pub version: String,
    /// The features it lists, `no-<feature>` turning a default off.
    pub features: Vec<String>,
    /// Its hosts' fully qualified names, as the apex assigns them.
    pub hosts: Vec<String>,
    /// Its settings, `static_site.site`, built through the template's contract.
    pub settings: Option<Value>,
    /// Its URL when a platform serves it at a path (`served_by`); see
    /// `.claude/decisions/2026-11-11-runtime-docs-under-platform-passkey.md` §4.
    pub served: Option<String>,
}

impl Site {
    /// The template tag it pins, or `<theme>@tree` for [`TREE`].
    #[must_use]
    pub fn tag(&self) -> String {
        if self.version == TREE {
            format!("{}@{TREE}", self.theme())
        } else {
            format!("{}/v{}", self.theme(), self.version)
        }
    }

    /// The revision its template is read at: `HEAD` for [`TREE`], else its tag.
    #[must_use]
    pub fn revision(&self) -> String {
        if self.version == TREE {
            "HEAD".into()
        } else {
            self.tag()
        }
    }

    /// Its template's theme name, which also names the template's tags.
    #[must_use]
    pub fn theme(&self) -> String {
        format!("{PREFIX}-{}", self.template)
    }

    /// Its template's project, inside a tag's tree.
    #[must_use]
    pub fn project(&self) -> String {
        format!("{TEMPLATES}/{}", self.template)
    }

    /// Its bucket.
    #[must_use]
    pub fn bucket(&self) -> String {
        static_site::bucket_name(&self.zone, &self.workload)
    }

    /// Its canonical host, for `base_url` and the live checks: the zone apex
    /// when it serves it, since `www` redirects there; else its first host.
    #[must_use]
    pub fn canonical(&self) -> Option<&str> {
        self.hosts
            .iter()
            .find(|h| **h == self.zone)
            .or_else(|| self.hosts.first())
            .map(String::as_str)
    }

    /// Its `base_url`: where a platform serves it, else its canonical host.
    #[must_use]
    pub fn url(&self) -> Option<String> {
        self.served
            .clone()
            .or_else(|| self.canonical().map(|h| format!("https://{h}")))
    }
}

/// A site's URL when a platform unit serves it at a path, from its `served_by`; see
/// `.claude/decisions/2026-11-11-runtime-docs-under-platform-passkey.md` §4.
fn served_url(m: &crate::cobot::manifest::Manifest, settings: &Value) -> Option<String> {
    let by = settings.get("served_by")?;
    let unit = m.units.get(by["unit"].as_str()?)?;
    let host = unit.settings["domains"][0].as_str()?;
    Some(format!("https://{host}{}", by["path"].as_str()?))
}

/// A built site: where Zola wrote it.
#[derive(Debug, Clone)]
pub struct Built {
    /// The site.
    pub site: Site,
    /// The build's `public/` directory.
    pub public: PathBuf,
    /// The effective features, as written into `[extra.features]`.
    pub features: BTreeMap<String, bool>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    default: bool,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    features: BTreeMap<String, Feature>,
    #[serde(default)]
    content: Content,
}

/// `content`: what a site's `site/` holds, or `none` for a template with no content.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Content {
    None(NoContent),
    Some {
        #[serde(default)]
        posts: Option<Posts>,
    },
}

impl Default for Content {
    fn default() -> Self {
        Self::Some { posts: None }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum NoContent {
    None,
}

#[derive(Debug, Deserialize)]
struct Posts {
    filename: String,
}

/// What a tag declares: its features' defaults, whether posts are named by
/// date, and whether its sites have no `site/`.
struct Declared {
    features: BTreeMap<String, bool>,
    date_slug: bool,
    no_content: bool,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |e| Error::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Runs git in `root` and returns stdout as bytes.
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| Error::Git {
            args: args.join(" "),
            message: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(Error::Git {
            args: args.join(" "),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output.stdout)
}

/// Every static site under `workloads/`, or those in `wanted`, by unit path.
///
/// # Errors
///
/// [`Error::UnknownUnit`] for a wanted path that is not a static site.
pub fn find(root: &Path, wanted: &[String]) -> Result<Vec<Site>, Error> {
    find_in(root, root, wanted)
}

/// [`find`] for the instance at `root` on the platform at `platform`.
///
/// # Errors
///
/// As [`find`].
pub fn find_in(root: &Path, platform: &Path, wanted: &[String]) -> Result<Vec<Site>, Error> {
    let mut out = Vec::new();
    let (units, problems) = unit::discover_in(root, platform);
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
    }
    for u in units {
        let Some(c) = &u.config else { continue };
        let Some(site) = &c.site else { continue };
        let Some(w) = &site.workload else { continue };
        let Some(settings) = static_site::settings(&c.config, site) else {
            continue;
        };
        let template = &settings["template"];
        let Some(name) = template["name"].as_str() else {
            return Err(Error::Io {
                path: PathBuf::from(&u.path).join(crate::cobot::config::CONFIG),
                message: "static_site.template.name is required".into(),
            });
        };
        let hosts = static_site::hosts(site, w)
            .iter()
            .map(|h| apex::fqdn(h, &site.zone))
            .collect();
        let content = match &site.manifest {
            Some(m) => m.content(&u.path).unwrap_or_else(|| u.dir.join("site")),
            None => u.dir.join("site"),
        };
        let served = match (&site.manifest, settings.get("served_by")) {
            (Some(m), Some(_)) => Some(served_url(m, settings).ok_or_else(|| Error::Io {
                path: PathBuf::from(&u.path).join(crate::cobot::config::CONFIG),
                message: "served_by: `unit` must be a unit with `domains`".into(),
            })?),
            _ => None,
        };
        let version = template["version"].as_str().unwrap_or_default().to_string();
        if version == TREE
            && !root
                .join(TEMPLATES)
                .join(name)
                .join("template.yaml")
                .is_file()
        {
            return Err(Error::Io {
                path: PathBuf::from(&u.path).join(crate::cobot::config::CONFIG),
                message: format!(
                    "static_site.template.version: `{TREE}` needs {TEMPLATES}/{name}/ in this instance's own tree"
                ),
            });
        }
        out.push(Site {
            unit: u.path.clone(),
            dir: u.dir.clone(),
            content,
            zone: site.zone.clone(),
            workload: w.clone(),
            template: name.to_string(),
            version,
            features: template["features"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            hosts,
            settings: settings.get("site").cloned(),
            served,
        });
    }
    if let Some(missing) = wanted
        .iter()
        .map(|w| w.trim_end_matches('/'))
        .find(|w| !out.iter().any(|s| s.unit == *w))
    {
        return Err(Error::UnknownUnit(missing.to_string()));
    }
    out.retain(|s| wanted.is_empty() || wanted.iter().any(|w| w.trim_end_matches('/') == s.unit));
    Ok(out)
}

/// The effective features: the tag's defaults, plus those listed, minus each `no-<feature>`.
///
/// # Errors
///
/// [`Error::Io`] naming a feature the template does not declare.
pub fn features(
    site: &Site,
    declared: &BTreeMap<String, bool>,
) -> Result<BTreeMap<String, bool>, Error> {
    let mut out = declared.clone();
    for f in &site.features {
        let (name, on) = f
            .strip_prefix("no-")
            .map_or((f.as_str(), true), |n| (n, false));
        if !declared.contains_key(name) {
            return Err(Error::Io {
                path: PathBuf::from(&site.unit).join(crate::cobot::config::CONFIG),
                message: format!(
                    "static_site.template.features: `{name}` is not a feature of {}",
                    site.tag()
                ),
            });
        }
        out.insert(name.to_string(), on);
    }
    Ok(out)
}

/// The site's tag's features, their defaults, its post naming and whether it
/// has content, from its `template.yaml`.
fn declared(root: &Path, site: &Site) -> Result<Declared, Error> {
    let at = format!("{}:{}/template.yaml", site.revision(), site.project());
    let text = git(root, &["show", &at])?;
    let manifest: Manifest =
        serde_saphyr::from_str(&String::from_utf8_lossy(&text)).map_err(|e| Error::Io {
            path: PathBuf::from(&at),
            message: e.to_string(),
        })?;
    let (date_slug, no_content) = match manifest.content {
        Content::None(_) => (false, true),
        Content::Some { posts } => (posts.is_some_and(|p| p.filename == DATE_SLUG), false),
    };
    Ok(Declared {
        features: manifest
            .features
            .into_iter()
            .map(|(k, f)| (k, f.default))
            .collect(),
        date_slug,
        no_content,
    })
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Error> {
    fs::create_dir_all(to).map_err(io(to))?;
    for e in fs::read_dir(from).map_err(io(from))?.flatten() {
        let target = to.join(e.file_name());
        let kind = e.file_type().map_err(io(&e.path()))?;
        if kind.is_symlink() {
            return Err(Error::Io {
                path: e.path(),
                message: "a site holds no symlinks".into(),
            });
        }
        if kind.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), &target).map_err(io(&target))?;
        }
    }
    Ok(())
}

/// Builds each site into `.cobot/<unit>/site/public`, from its template's
/// contract and its `static_site.site`.
///
/// # Errors
///
/// [`Error::Git`] for a template tag that does not resolve, [`Error::Io`] for
/// an unknown feature, settings the contract rejects, a misnamed post or an
/// unreadable site, [`Error::Tool`] if Zola fails.
pub fn build(root: &Path, sites: &[Site]) -> Result<Vec<Built>, Error> {
    build_with(root, sites, &BTreeMap::new())
}

/// [`build`], with each of `data` written to every site's `data/<name>.json` for its
/// template's `load_data`; see the architecture-site ADR §3.
///
/// # Errors
///
/// As [`build`], and [`Error::Io`] for a data name that is not `[a-z0-9-]+`.
pub fn build_with(
    root: &Path,
    sites: &[Site],
    data: &BTreeMap<String, Value>,
) -> Result<Vec<Built>, Error> {
    build_in(root, root, sites, data)
}

/// [`build_with`] for the instance at `root`, with templates and their tags from the
/// platform at `platform`; work goes under the instance.
///
/// # Errors
///
/// As [`build_with`].
pub fn build_in(
    root: &Path,
    platform: &Path,
    sites: &[Site],
    data: &BTreeMap<String, Value>,
) -> Result<Vec<Built>, Error> {
    if let Some(bad) = data.keys().find(|k| !is_data_name(k)) {
        return Err(Error::Io {
            path: PathBuf::from(format!("data/{bad}.json")),
            message: "a data name is lowercase letters, digits and dashes".into(),
        });
    }
    let head = git(root, &["rev-parse", "--short", "HEAD"])
        .or_else(|_| git(platform, &["rev-parse", "--short", "HEAD"]))?;
    let commit = String::from_utf8_lossy(&head).trim().to_string();
    let mut out = Vec::new();
    for site in sites {
        let declared = declared(platform, site)?;
        let features = features(site, &declared.features)?;
        let base = site.url().ok_or_else(|| Error::Io {
            path: PathBuf::from(&site.unit),
            message: "the apex assigns this static site no host".into(),
        })?;
        let work = root.join(WORK_DIR).join(&site.unit).join("site");
        if work.exists() {
            fs::remove_dir_all(&work).map_err(io(&work))?;
        }
        let settings = site.settings.as_ref().ok_or_else(|| Error::Io {
            path: PathBuf::from(&site.unit).join(crate::cobot::config::CONFIG),
            message: "static_site.site is required".into(),
        })?;
        assemble(platform, site, settings, &declared, &base, &features, &work)?;
        for (name, value) in data {
            let path = work.join("data").join(format!("{name}.json"));
            fs::create_dir_all(work.join("data")).map_err(io(&path))?;
            let text = serde_json::to_string(value).map_err(|e| Error::Io {
                path: path.clone(),
                message: e.to_string(),
            })?;
            fs::write(&path, text).map_err(io(&path))?;
        }
        run(
            Command::new("zola")
                .arg("build")
                .current_dir(&work)
                .env("SITE_COMMIT_SHA", &commit),
            "zola build",
        )?;
        out.push(Built {
            site: site.clone(),
            public: work.join("public"),
            features,
        });
    }
    Ok(out)
}

/// The template at the site's tag, under `themes/`.
fn theme(root: &Path, site: &Site, work: &Path) -> Result<(), Error> {
    let theme = work.join("themes").join(site.theme());
    fs::create_dir_all(&theme).map_err(io(&theme))?;
    let tar = git(
        root,
        &[
            "archive",
            "--format=tar",
            &format!("{}:{}", site.revision(), site.project()),
        ],
    )?;
    let archive = work.join("themes").join(format!("{}.tar", site.theme()));
    fs::write(&archive, tar).map_err(io(&archive))?;
    run(
        Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&theme),
        "tar -xf",
    )?;
    fs::remove_file(&archive).map_err(io(&archive))
}

/// A contract site: the tag's scaffold, the settings at their `x-site`
/// targets, then `site/`'s posts, pages and assets.
fn assemble(
    root: &Path,
    site: &Site,
    settings: &Value,
    declared: &Declared,
    base: &str,
    features: &BTreeMap<String, bool>,
    work: &Path,
) -> Result<(), Error> {
    let fail = |message: String| Error::Io {
        path: PathBuf::from(&site.unit).join(crate::cobot::config::CONFIG),
        message,
    };
    let schema_text = git(
        root,
        &[
            "show",
            &format!("{}:{}/schema.json", site.revision(), site.project()),
        ],
    )
    .map_err(|_| {
        fail(format!(
            "static_site.site: {} has no schema.json; it predates the config contract",
            site.tag()
        ))
    })?;
    let schema: Value = serde_json::from_slice(&schema_text).map_err(|e| {
        fail(format!(
            "{}:{}/schema.json: {e}",
            site.tag(),
            site.project()
        ))
    })?;
    check_settings(&schema, settings).map_err(fail)?;
    let src = site.content.clone();
    let problems = if declared.no_content {
        check_no_content(&src)
    } else {
        check_layout(&src, declared.date_slug)
    };
    if !problems.is_empty() {
        return Err(Error::Io {
            path: PathBuf::from(&site.unit).join("site"),
            message: problems.join("; "),
        });
    }

    theme(root, site, work)?;
    copy_dir(
        &work.join("themes").join(site.theme()).join("scaffold"),
        work,
    )?;
    let config_path = work.join("config.toml");
    let manifest_path = work.join("static/manifest.json");
    let (config, manifest) = configure(
        &schema,
        settings,
        &fs::read_to_string(&config_path).map_err(io(&config_path))?,
        &fs::read_to_string(&manifest_path).map_err(io(&manifest_path))?,
        &site.theme(),
        base,
        features,
    )
    .map_err(fail)?;
    fs::write(&config_path, config).map_err(io(&config_path))?;
    fs::write(&manifest_path, manifest).map_err(io(&manifest_path))?;

    for dir in SITE_DIRS {
        let from = src.join(dir);
        if from.is_dir() {
            let to = if dir == "assets" {
                work.join("static").join(dir)
            } else {
                work.join("content").join(dir)
            };
            copy_dir(&from, &to)?;
        }
    }
    Ok(())
}

/// A site whose template has no content must have no `site/`.
#[must_use]
pub fn check_no_content(src: &Path) -> Vec<String> {
    no_content_problems(&src.display().to_string(), src.exists())
}

/// Every way a contract site's `site/` breaks the layout; see [`layout_problems`].
#[must_use]
pub fn check_layout(src: &Path, date_slug: bool) -> Vec<String> {
    layout_problems(&listing(src, date_slug), date_slug)
}

/// What [`layout_problems`] needs of `src`: its entries, everything under `posts/` and `pages/`,
/// and, with `date_slug`, each post's text.
fn listing(src: &Path, date_slug: bool) -> Listing {
    let top = fs::read_dir(src).ok().map(|entries| {
        entries
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    e.file_type().is_ok_and(|t| t.is_dir()),
                )
            })
            .collect()
    });
    let mut nested = Vec::new();
    for dir in ["posts", "pages"] {
        walk_sorted(&src.join(dir), dir, &mut nested);
    }
    let mut posts = Vec::new();
    if date_slug && let Ok(entries) = fs::read_dir(src.join("posts")) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for e in entries {
            posts.push((
                e.file_name().to_string_lossy().into_owned(),
                fs::read_to_string(e.path()).unwrap_or_default(),
            ));
        }
    }
    Listing {
        shown: src.display().to_string(),
        top,
        nested,
        posts,
    }
}

/// Every entry under `dir`, as `at/<name>…` and whether it is a directory, sorted, depth first.
fn walk_sorted(dir: &Path, at: &str, out: &mut Vec<(String, bool)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let rel = format!("{at}/{}", e.file_name().to_string_lossy());
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        out.push((rel.clone(), is_dir));
        if is_dir {
            walk_sorted(&e.path(), &rel, out);
        }
    }
}

fn run(command: &mut Command, name: &str) -> Result<(), Error> {
    let output = command.output().map_err(|e| Error::Tool {
        command: name.into(),
        message: e.to_string(),
    })?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error::Tool {
        command: name.into(),
        message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
    for e in fs::read_dir(dir).map_err(io(dir))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if e.file_type().map_err(io(&e.path()))?.is_dir() {
            walk(&e.path(), &key, out)?;
        } else {
            out.insert(key, fs::read(e.path()).map_err(io(&e.path()))?);
        }
    }
    Ok(())
}

/// Every key the bucket should hold: each file under `public`, plus `x` and
/// `x/` copies of each `x/index.html` below the root.
///
/// # Errors
///
/// [`Error::Io`] for an unreadable file, or a file at [`TYPES`].
pub fn expected(public: &Path) -> Result<BTreeMap<String, Upload>, Error> {
    let mut files = BTreeMap::new();
    walk(public, "", &mut files)?;
    expected_of(files).map_err(|message| Error::Io {
        path: public.join(TYPES),
        message,
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Makes `bucket` hold exactly `expected`: puts each new or changed object,
/// compared by MD5 against its `ETag`, and deletes each stale key.
///
/// # Errors
///
/// [`Error::Tool`] naming the failed call; never a credential.
pub fn sync(bucket: &Bucket, expected: &BTreeMap<String, Upload>) -> Result<Synced, Error> {
    let fail = |e: s3::Failure| Error::Tool {
        command: "s3".into(),
        message: e.0,
    };
    let have: BTreeMap<String, String> = bucket
        .list(now())
        .map_err(fail)?
        .into_iter()
        .map(|o| (o.key, o.etag))
        .collect();
    let written: BTreeMap<String, String> = if have.contains_key(TYPES) {
        bucket
            .get(TYPES, now())
            .map_err(fail)?
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    let plan = sync_plan(expected, &have, &written);
    for key in &plan.put {
        let up = &expected[key];
        bucket
            .put(key, &up.body, up.content_type, now())
            .map_err(fail)?;
    }
    if let Some(body) = &plan.types {
        bucket
            .put(TYPES, body, content_type(TYPES), now())
            .map_err(fail)?;
    }
    for key in &plan.delete {
        bucket.delete(key, now()).map_err(fail)?;
    }
    let done = Synced {
        put: plan.put,
        unchanged: plan.unchanged,
        deleted: plan.delete,
    };
    Ok(done)
}

/// The site token's credentials and the account id, from `infra/cloudflare`'s
/// `tofu show -json` state; see [`credentials_of`].
#[must_use]
pub fn credentials(
    shown: &str,
    site: &Site,
    today: crate::cobot::generations::Day,
) -> Option<(Credentials, String)> {
    credentials_of(shown, &site.zone, &site.workload, today)
}

/// One live check's result.
#[derive(Debug, Clone)]
pub struct Check {
    /// What it checked.
    pub what: String,
    /// Whether it passed, after its retries.
    pub passed: bool,
}

fn fetch(url: &str) -> Option<(u16, Option<String>, String)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut r = agent.get(url).call().ok()?;
    let location = r
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = r.body_mut().read_to_string().unwrap_or_default();
    Some((r.status().as_u16(), location, body))
}

/// Checks the published site, retrying each check while a new domain comes up.
#[must_use]
pub fn checks(
    built: &Built,
    keys: &BTreeMap<String, Upload>,
    tries: u32,
    wait: Duration,
) -> Vec<Check> {
    // A served site sits behind the platform's passkey gate; its digests are checked instead.
    if built.site.served.is_some() {
        return Vec::new();
    }
    probes(
        built.site.canonical(),
        &built.site.zone,
        &built.site.hosts,
        keys.keys(),
        now(),
    )
    .into_iter()
    .map(|probe| {
        let passed = (0..tries).any(|i| {
            if i > 0 {
                std::thread::sleep(wait);
            }
            fetch(probe.url()).is_some_and(|(status, location, body)| {
                probe.passes(status, location.as_deref(), &body)
            })
        });
        Check {
            what: probe.what(),
            passed,
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use serde_json::json;

    use super::{
        Content, Manifest, Site, check_layout, check_no_content, check_post, check_settings,
        content_type, features, set_json, set_toml, site_targets,
    };

    fn site(features: &[&str]) -> Site {
        Site {
            unit: "workloads/z.com/blog".into(),
            dir: "workloads/z.com/blog".into(),
            content: "workloads/z.com/blog/site".into(),
            zone: "z.com".into(),
            workload: "blog".into(),
            template: "blog".into(),
            version: "1.0.0".into(),
            features: features.iter().map(|f| (*f).to_string()).collect(),
            hosts: vec!["z.com".into(), "www.z.com".into()],
            settings: None,
            served: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_copy_refuses_a_symlink_anywhere_in_the_tree() {
        let outside = tempfile::TempDir::new().expect("outside");
        fs::write(outside.path().join("secret"), "not for the site").expect("write");
        let src = tempfile::TempDir::new().expect("src");
        fs::create_dir_all(src.path().join("posts")).expect("mkdir");
        std::os::unix::fs::symlink(
            outside.path().join("secret"),
            src.path().join("posts/linked.md"),
        )
        .expect("link");
        let to = tempfile::TempDir::new().expect("to");
        let err = super::copy_dir(src.path(), to.path()).expect_err("refused");
        assert!(err.to_string().contains("no symlinks"), "{err}");
        assert!(!to.path().join("posts/linked.md").exists());
    }

    #[test]
    fn a_named_template_has_its_own_project_theme_and_tag() {
        let mut named = site(&[]);
        named.template = "architecture".into();
        assert_eq!(
            (named.project(), named.theme(), named.tag()),
            (
                "src/site-templates/architecture".to_string(),
                "site-template-architecture".to_string(),
                "site-template-architecture/v1.0.0".to_string()
            )
        );
    }

    #[test]
    fn content_none_parses_beside_a_posts_rule() {
        let none: Manifest = serde_saphyr::from_str("features: {}\ncontent: none\n").expect("none");
        assert!(matches!(none.content, Content::None(_)));
        let posts: Manifest =
            serde_saphyr::from_str("features: {}\ncontent:\n  posts: { filename: date-slug }\n")
                .expect("posts");
        assert!(matches!(posts.content, Content::Some { posts: Some(_) }));
        let absent: Manifest = serde_saphyr::from_str("features: {}\n").expect("absent");
        assert!(matches!(absent.content, Content::Some { posts: None }));
    }

    #[test]
    fn a_site_with_no_content_has_no_site_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("site");
        assert_eq!(check_no_content(&src), Vec::<String>::new());
        fs::create_dir_all(&src).expect("mkdir");
        assert!(check_no_content(&src)[0].contains("must not exist"));
    }

    #[test]
    fn the_zone_apex_is_canonical_when_served() {
        let mut s = site(&[]);
        s.hosts = vec!["www.z.com".into(), "z.com".into()];
        assert_eq!(s.canonical(), Some("z.com"));
        s.hosts = vec!["journal.z.com".into(), "j.z.com".into()];
        assert_eq!(s.canonical(), Some("journal.z.com"));
    }

    #[test]
    fn features_start_from_the_defaults() {
        let declared: BTreeMap<String, bool> =
            [("feed".to_string(), true), ("math".to_string(), false)].into();
        let got = features(&site(&["math", "no-feed"]), &declared).expect("known");
        assert_eq!(
            got,
            [("feed".to_string(), false), ("math".to_string(), true)].into()
        );
        let err = features(&site(&["typo"]), &declared)
            .expect_err("unknown")
            .to_string();
        assert!(
            err.contains("`typo` is not a feature of site-template-blog/v1.0.0"),
            "{err}"
        );
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(content_type("a/b.CSS"), "text/css; charset=utf-8");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    const POST: &str = "+++\ntitle = \"A\"\ndate = 2015-02-15T20:00:00+00:00\n+++\nBody.\n";

    #[test]
    fn posts_are_named_by_their_date() {
        assert_eq!(check_post("2015-02-15-hello-medium.md", POST), Ok(()));
        let string_date = "+++\ndate = \"2015-02-15\"\n+++\n";
        assert_eq!(check_post("2015-02-15-a.md", string_date), Ok(()));
        for (name, text, why) in [
            ("0001-hello.md", POST, "is not named"),
            ("2015-02-15-Hello.md", POST, "is not named"),
            ("2026é-01-01-a.md", POST, "is not named"),
            ("2015-02-15-.md", POST, "is not named"),
            ("2015-02-15.md", POST, "is not named"),
            (
                "2015-02-16-hello.md",
                POST,
                "but its date is 2015-02-15T20:00:00+00:00",
            ),
            (
                "2015-02-15-hello.md",
                "+++\ndate = 2015-02-15\nslug = \"x\"\n+++\n",
                "sets slug",
            ),
            (
                "2015-02-15-hello.md",
                "+++\ntitle = \"A\"\n+++\n",
                "has no date",
            ),
            (
                "2015-02-15-hello.md",
                "---\ndate: 2015-02-15\n---\n",
                "front matter",
            ),
        ] {
            let err = check_post(name, text).expect_err(name);
            assert!(err.contains(why), "{name}: {err}");
        }
    }

    #[test]
    fn site_holds_only_posts_pages_and_assets() {
        let dir = tempfile::tempdir().expect("tmp");
        let src = dir.path();
        for d in ["posts", "pages", "assets/images"] {
            std::fs::create_dir_all(src.join(d)).expect("dir");
        }
        std::fs::write(src.join("posts/2015-02-15-a.md"), POST).expect("post");
        assert_eq!(check_layout(src, true), Vec::<String>::new());

        std::fs::write(src.join("config.toml"), "").expect("config");
        std::fs::create_dir_all(src.join("content")).expect("content");
        std::fs::write(src.join("posts/_index.md"), "").expect("index");
        std::fs::create_dir_all(src.join("pages/team")).expect("nested");
        std::fs::write(src.join("pages/team/_index.md"), "").expect("nested index");
        std::fs::write(src.join("posts/0002-b.md"), POST).expect("post");
        let problems = check_layout(src, true).join("\n");
        for want in [
            "`config.toml`",
            "`content`",
            "`posts/_index.md`",
            "posts/0002-b.md",
        ] {
            assert!(problems.contains(want), "{want}: {problems}");
        }
        assert!(!check_layout(src, false).join("\n").contains("0002-b.md"));
    }

    #[test]
    fn settings_land_at_each_target() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title"],
            "properties": {
                "title": { "type": "string", "x-site": [
                    { "file": "config.toml", "key": "title" },
                    { "file": "static/manifest.json", "key": "name" }
                ] },
                "theme_color": {
                    "type": "object",
                    "properties": { "light": { "type": "string", "x-site": [
                        { "file": "static/manifest.json", "key": "theme_color" }
                    ] } },
                    "x-site": [{ "file": "config.toml", "key": "extra.theme_color" }]
                }
            }
        });
        let settings = json!({ "title": "T", "theme_color": { "light": "#fff" } });
        assert_eq!(check_settings(&schema, &settings), Ok(()));
        let err = check_settings(&schema, &json!({ "typo": 1 })).expect_err("bad");
        assert!(err.contains("static_site.site"), "{err}");

        let mut out = Vec::new();
        site_targets(&schema, &settings, &mut out);
        let got: Vec<(String, String)> =
            out.iter().map(|(f, k, _)| (f.clone(), k.clone())).collect();
        assert_eq!(
            got,
            [
                ("config.toml".to_string(), "extra.theme_color".to_string()),
                ("static/manifest.json".into(), "theme_color".into()),
                ("config.toml".into(), "title".into()),
                ("static/manifest.json".into(), "name".into()),
            ]
        );

        let mut config: toml::Value =
            toml::from_str("theme = \"t\"\n[extra]\nintro = \"\"\n").expect("toml");
        set_toml(
            &mut config,
            "extra.author.page",
            toml::Value::String("/a".into()),
        )
        .expect("set");
        set_toml(&mut config, "title", toml::Value::String("T".into())).expect("set");
        assert_eq!(config["extra"]["author"]["page"].as_str(), Some("/a"));
        assert_eq!(config["extra"]["intro"].as_str(), Some(""));
        assert!(set_toml(&mut config, "theme.x", toml::Value::Boolean(true)).is_err());

        let mut manifest = json!({ "display": "standalone" });
        set_json(&mut manifest, "name", json!("T")).expect("set");
        assert_eq!(manifest, json!({ "display": "standalone", "name": "T" }));
    }
}
