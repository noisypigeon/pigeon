//! `pigeon cobot sites`: build each static site with its pinned template, and
//! publish it to its R2 bucket; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §5 and §7,
//! `.claude/decisions/2026-11-03-runtime-static-site-config-contract.md`, and
//! `.claude/decisions/2026-11-05-runtime-architecture-site-through-backplane.md` §1.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::Value;

use crate::apex;
use crate::s3::{self, Bucket, Credentials};
use crate::static_site;
use crate::{Error, WORK_DIR, unit};

/// Where templates live, as `<TEMPLATES>/<name>`.
pub const TEMPLATES: &str = "src/site-templates";
/// A template's theme and tag prefix, as `<PREFIX>-<name>`.
pub const PREFIX: &str = "site-template";
/// The post naming rule `YYYY-MM-DD-<slug>.md`.
pub const DATE_SLUG: &str = "date-slug";
/// What a contract site's `site/` may hold.
pub const SITE_DIRS: [&str; 3] = ["posts", "pages", "assets"];

/// A static site, as found under `workloads/`.
#[derive(Debug, Clone)]
pub struct Site {
    /// The unit path, `workloads/<zone>/<workload>`.
    pub unit: String,
    /// The unit's directory.
    pub dir: PathBuf,
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
}

impl Site {
    /// The template tag it pins.
    #[must_use]
    pub fn tag(&self) -> String {
        format!("{}/v{}", self.theme(), self.version)
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
    let mut out = Vec::new();
    let (units, problems) = unit::discover(root);
    if !problems.is_empty() {
        return Err(Error::Invalid(problems));
    }
    for u in units {
        let Some(c) = &u.config else { continue };
        let Some(site) = &c.site else { continue };
        let Some(w) = &site.workload else { continue };
        if !static_site::is_static(&c.config) {
            continue;
        }
        let template = &c.config["static_site"]["template"];
        let Some(name) = template["name"].as_str() else {
            return Err(Error::Io {
                path: PathBuf::from(&u.path).join(crate::config::CONFIG),
                message: "static_site.template.name is required".into(),
            });
        };
        let hosts = apex::assignments(
            &apex::workload_config(root, &site.zone, apex::APEX).unwrap_or(Value::Null),
            &apex::workloads(root, &site.zone),
            &site.zone,
        )
        .remove(w)
        .unwrap_or_default()
        .iter()
        .map(|h| apex::fqdn(h, &site.zone))
        .collect();
        out.push(Site {
            unit: u.path.clone(),
            dir: u.dir.clone(),
            zone: site.zone.clone(),
            workload: w.clone(),
            template: name.to_string(),
            version: template["version"].as_str().unwrap_or_default().to_string(),
            features: template["features"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            hosts,
            settings: c.config["static_site"].get("site").cloned(),
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
                path: PathBuf::from(&site.unit).join(crate::config::CONFIG),
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
    let at = format!("{}:{}/template.yaml", site.tag(), site.project());
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
        if e.file_type().map_err(io(&e.path()))?.is_dir() {
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
    if let Some(bad) = data.keys().find(|k| !is_data_name(k)) {
        return Err(Error::Io {
            path: PathBuf::from(format!("data/{bad}.json")),
            message: "a data name is lowercase letters, digits and dashes".into(),
        });
    }
    let commit = String::from_utf8_lossy(&git(root, &["rev-parse", "--short", "HEAD"])?)
        .trim()
        .to_string();
    let mut out = Vec::new();
    for site in sites {
        let declared = declared(root, site)?;
        let features = features(site, &declared.features)?;
        let base = format!(
            "https://{}",
            site.canonical().ok_or_else(|| Error::Io {
                path: PathBuf::from(&site.unit),
                message: "the apex assigns this static site no host".into(),
            })?
        );
        let work = root.join(WORK_DIR).join(&site.unit).join("site");
        if work.exists() {
            fs::remove_dir_all(&work).map_err(io(&work))?;
        }
        let settings = site.settings.as_ref().ok_or_else(|| Error::Io {
            path: PathBuf::from(&site.unit).join(crate::config::CONFIG),
            message: "static_site.site is required".into(),
        })?;
        assemble(root, site, settings, &declared, &base, &features, &work)?;
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
            &format!("{}:{}", site.tag(), site.project()),
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
        path: PathBuf::from(&site.unit).join(crate::config::CONFIG),
        message,
    };
    let schema_text = git(
        root,
        &[
            "show",
            &format!("{}:{}/schema.json", site.tag(), site.project()),
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
    let src = site.dir.join("site");
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
    let mut config: toml::Value =
        toml::from_str(&fs::read_to_string(&config_path).map_err(io(&config_path))?)
            .map_err(|e| fail(format!("scaffold/config.toml: {e}")))?;
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(io(&manifest_path))?)
            .map_err(|e| fail(format!("scaffold/static/manifest.json: {e}")))?;
    let mut targets = Vec::new();
    site_targets(&schema, settings, &mut targets);
    for (file, key, value) in targets {
        match file.as_str() {
            "config.toml" => {
                let v = toml::Value::try_from(&value).map_err(|e| fail(format!("{key}: {e}")))?;
                set_toml(&mut config, &key, v).map_err(fail)?;
            }
            "static/manifest.json" => set_json(&mut manifest, &key, value).map_err(fail)?,
            other => return Err(fail(format!("x-site: unknown file `{other}`"))),
        }
    }
    set_toml(&mut config, "theme", toml::Value::String(site.theme())).map_err(fail)?;
    set_toml(
        &mut config,
        "base_url",
        toml::Value::String(base.to_string()),
    )
    .map_err(fail)?;
    let flags = features
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::Boolean(*v)))
        .collect();
    set_toml(&mut config, "extra.features", toml::Value::Table(flags)).map_err(fail)?;
    fs::write(
        &config_path,
        toml::to_string(&config).map_err(|e| fail(format!("config.toml: {e}")))?,
    )
    .map_err(io(&config_path))?;
    let mut json =
        serde_json::to_string_pretty(&manifest).map_err(|e| fail(format!("manifest.json: {e}")))?;
    json.push('\n');
    fs::write(&manifest_path, json).map_err(io(&manifest_path))?;

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

/// `settings` against the tag's schema, with every error.
fn check_settings(schema: &Value, settings: &Value) -> Result<(), String> {
    let validator = jsonschema::validator_for(schema).map_err(|e| format!("schema.json: {e}"))?;
    let errors: Vec<String> = validator
        .iter_errors(settings)
        .map(|e| {
            let at = e.instance_path().to_string();
            format!("static_site.site{}: {e}", at.replace('/', "."))
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Each `(file, key, value)` the settings write: every set property whose
/// schema carries `x-site`, at any depth.
fn site_targets(schema: &Value, value: &Value, out: &mut Vec<(String, String, Value)>) {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, prop) in props {
        let Some(v) = value.get(name) else { continue };
        for t in prop
            .get("x-site")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(file), Some(key)) = (t["file"].as_str(), t["key"].as_str()) {
                out.push((file.to_string(), key.to_string(), v.clone()));
            }
        }
        site_targets(prop, v, out);
    }
}

/// Sets the dotted `key` in a TOML table, creating tables on the way.
fn set_toml(doc: &mut toml::Value, key: &str, value: toml::Value) -> Result<(), String> {
    let mut at = doc;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let table = at
            .as_table_mut()
            .ok_or_else(|| format!("config.toml: `{key}` crosses a non-table"))?;
        if i + 1 == parts.len() {
            table.insert((*part).to_string(), value);
            return Ok(());
        }
        at = table
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    }
    Ok(())
}

/// Sets the dotted `key` in a JSON object, creating objects on the way.
fn set_json(doc: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let mut at = doc;
    let parts: Vec<&str> = key.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let map = at
            .as_object_mut()
            .ok_or_else(|| format!("manifest.json: `{key}` crosses a non-object"))?;
        if i + 1 == parts.len() {
            map.insert((*part).to_string(), value);
            return Ok(());
        }
        at = map
            .entry((*part).to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    Ok(())
}

/// Whether `name` may name a data file: `[a-z0-9-]+`.
#[must_use]
pub fn is_data_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A site whose template has no content must have no `site/`.
#[must_use]
pub fn check_no_content(src: &Path) -> Vec<String> {
    if src.exists() {
        vec![format!(
            "{} must not exist: the site's template has no content",
            src.display()
        )]
    } else {
        Vec::new()
    }
}

/// Every way a contract site's `site/` breaks the layout: anything but
/// [`SITE_DIRS`], an `_index.md`, and, with `date_slug`, a misnamed post.
#[must_use]
pub fn check_layout(src: &Path, date_slug: bool) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(src) else {
        return vec![format!("{} is missing", src.display())];
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir || !SITE_DIRS.contains(&name.as_str()) {
            out.push(format!(
                "`{name}` does not belong in site/, which holds only posts/, pages/ and assets/"
            ));
        }
    }
    for dir in ["posts", "pages"] {
        indexes(&src.join(dir), dir, &mut out);
    }
    if date_slug && let Ok(posts) = fs::read_dir(src.join("posts")) {
        let mut names: Vec<_> = posts.flatten().collect();
        names.sort_by_key(fs::DirEntry::file_name);
        for e in names {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "_index.md" {
                continue;
            }
            if let Err(m) = check_post(&name, &fs::read_to_string(e.path()).unwrap_or_default()) {
                out.push(format!("posts/{name}: {m}"));
            }
        }
    }
    out
}

/// The `YYYY-MM-DD` of a name `YYYY-MM-DD-<slug>.md`, its slug lowercase
/// words of `[a-z0-9]` joined by dashes.
fn date_prefix(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let bytes = stem.as_bytes();
    let dated = bytes.len() > 11
        && bytes[..10].iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        && bytes[10] == b'-';
    if !dated {
        return None;
    }
    let slugged = stem[11..].split('-').all(|w| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    slugged.then(|| &stem[..10])
}

/// Each `_index.md` under `dir`, at any depth: sections are the scaffold's.
fn indexes(dir: &Path, at: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = format!("{at}/{name}");
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            indexes(&e.path(), &rel, out);
        } else if name == "_index.md" {
            out.push(format!("`{rel}`: sections are the template's scaffold"));
        }
    }
}

/// A post's name against `date-slug`: `YYYY-MM-DD-<slug>.md`, its prefix the
/// date of its `date`, and no `slug` of its own.
///
/// # Errors
///
/// What is wrong, in a few words.
pub fn check_post(name: &str, text: &str) -> Result<(), String> {
    let Some(prefix) = date_prefix(name) else {
        return Err("is not named YYYY-MM-DD-<slug>.md".into());
    };
    let front = text
        .strip_prefix("+++")
        .and_then(|rest| rest.split_once("\n+++"))
        .map(|(f, _)| f)
        .ok_or("has no +++ TOML front matter")?;
    let front: toml::Value = toml::from_str(front).map_err(|e| format!("front matter: {e}"))?;
    if front.get("slug").is_some() {
        return Err("sets slug; its name after the date is the slug".into());
    }
    let date = front
        .get("date")
        .map(|d| match d {
            toml::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .ok_or("has no date")?;
    if date.get(..10) != Some(prefix) {
        return Err(format!("is prefixed {prefix}, but its date is {date}"));
    }
    Ok(())
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

/// A file's `Content-Type`, from its extension.
#[must_use]
pub fn content_type(key: &str) -> &'static str {
    let ext = key.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// One object to upload: its body and type.
#[derive(Debug, Clone)]
pub struct Upload {
    /// The body.
    pub body: Vec<u8>,
    /// Its `Content-Type`.
    pub content_type: &'static str,
}

fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Upload>) -> Result<(), Error> {
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
            let body = fs::read(e.path()).map_err(io(&e.path()))?;
            out.insert(
                key.clone(),
                Upload {
                    body,
                    content_type: content_type(&key),
                },
            );
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
    let mut out = BTreeMap::new();
    walk(public, "", &mut out)?;
    if out.contains_key(TYPES) {
        return Err(Error::Io {
            path: public.join(TYPES),
            message: format!(
                "`{TYPES}` is cobot's own key in the bucket; a site cannot publish it"
            ),
        });
    }
    let pages: Vec<(String, Upload)> = out
        .iter()
        .filter_map(|(k, v)| {
            k.strip_suffix("/index.html")
                .map(|dir| (dir.to_string(), v.clone()))
        })
        .collect();
    for (dir, page) in pages {
        let html = Upload {
            body: page.body,
            content_type: content_type("index.html"),
        };
        out.insert(format!("{dir}/"), html.clone());
        out.insert(dir, html);
    }
    Ok(out)
}

/// The key holding each object's last written `Content-Type`; a site may not use it.
pub const TYPES: &str = ".cobot/types.json";

/// What a sync did, by key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Synced {
    /// Keys written, new or changed.
    pub put: Vec<String>,
    /// Keys left as they were.
    pub unchanged: usize,
    /// Keys deleted, no longer expected.
    pub deleted: Vec<String>,
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
    let types: BTreeMap<String, String> = expected
        .iter()
        .map(|(k, u)| (k.clone(), u.content_type.to_string()))
        .collect();
    let mut done = Synced::default();
    for (key, up) in expected {
        if have.get(key).is_some_and(|e| *e == s3::md5_hex(&up.body))
            && written.get(key).is_some_and(|t| t == up.content_type)
        {
            done.unchanged += 1;
            continue;
        }
        bucket
            .put(key, &up.body, up.content_type, now())
            .map_err(fail)?;
        done.put.push(key.clone());
    }
    if written != types {
        let body = serde_json::to_vec(&types).unwrap_or_default();
        bucket
            .put(TYPES, &body, content_type(TYPES), now())
            .map_err(fail)?;
    }
    let keep: BTreeSet<&str> = expected.keys().map(String::as_str).chain([TYPES]).collect();
    for key in have.keys().filter(|k| !keep.contains(k.as_str())) {
        bucket.delete(key, now()).map_err(fail)?;
        done.deleted.push(key.clone());
    }
    Ok(done)
}

/// The site token's credentials and the account id, from `infra/cloudflare`'s
/// `tofu show -json` state: the live generation's id as the access key, and
/// the SHA-256 of its value as the secret.
#[must_use]
pub fn credentials(
    shown: &str,
    site: &Site,
    today: crate::generations::Day,
) -> Option<(Credentials, String)> {
    let state: Value = serde_json::from_str(shown).ok()?;
    let outputs = &state["values"]["outputs"];
    let token = static_site::token_name(&site.zone, &site.workload);
    let (g, value) = apex::live_token(outputs, "tokens", &token, today)?;
    let id = outputs["token_ids"]["value"][g.key(&token)].as_str()?;
    let account = outputs["account_id"]["value"].as_str()?;
    Some((
        Credentials {
            access_key: id.to_string(),
            secret_key: s3::sha256_hex(value.as_bytes()),
        },
        account.to_string(),
    ))
}

/// R2's S3 endpoint for an account, in the EU jurisdiction.
#[must_use]
pub fn endpoint(account: &str) -> String {
    format!("https://{account}.eu.r2.cloudflarestorage.com")
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
    let Some(apex_host) = built.site.canonical() else {
        return Vec::new();
    };
    let base = format!("https://{apex_host}");
    let mut list: Vec<(String, Box<dyn Fn() -> bool>)> = Vec::new();
    let root = base.clone();
    list.push((
        format!("{root}/ returns 200 with <html"),
        Box::new(move || {
            fetch(&format!("{root}/")).is_some_and(|(s, _, b)| s == 200 && b.contains("<html"))
        }),
    ));
    if let Some(page) = keys
        .keys()
        .find_map(|k| k.strip_suffix("/index.html").map(str::to_string))
    {
        for url in [format!("{base}/{page}"), format!("{base}/{page}/")] {
            list.push((
                format!("{url} returns 200"),
                Box::new(move || fetch(&url).is_some_and(|(s, _, _)| s == 200)),
            ));
        }
    }
    let missing = format!("{base}/cobot-missing-{}/", now());
    list.push((
        format!("{missing} returns 404"),
        Box::new(move || fetch(&missing).is_some_and(|(s, _, _)| s == 404)),
    ));
    let www = format!("www.{}", built.site.zone);
    if built.site.hosts.contains(&built.site.zone) && built.site.hosts.contains(&www) {
        let from = format!("https://{www}/");
        let to = format!("https://{}/", built.site.zone);
        list.push((
            format!("{from} redirects 301 to {to}"),
            Box::new(move || {
                fetch(&from).is_some_and(|(s, l, _)| s == 301 && l.as_deref() == Some(to.as_str()))
            }),
        ));
    }
    list.into_iter()
        .map(|(what, check)| {
            let passed = (0..tries).any(|i| {
                if i > 0 {
                    std::thread::sleep(wait);
                }
                check()
            });
            Check { what, passed }
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
            zone: "z.com".into(),
            workload: "blog".into(),
            template: "blog".into(),
            version: "1.0.0".into(),
            features: features.iter().map(|f| (*f).to_string()).collect(),
            hosts: vec!["z.com".into(), "www.z.com".into()],
            settings: None,
        }
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
