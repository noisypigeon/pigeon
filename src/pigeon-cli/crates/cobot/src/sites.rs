//! `pigeon cobot sites`: build each static site with its pinned template, and
//! publish it to its R2 bucket; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §5 and §7.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
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

/// The template project, inside a tag's tree.
pub const TEMPLATE: &str = "src/static-site-template";
/// The template's theme name, under a build's `themes/`.
pub const THEME: &str = "static-site-template";

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
    /// The pinned template version.
    pub version: String,
    /// The features it lists, `no-<feature>` turning a default off.
    pub features: Vec<String>,
    /// Its hosts' fully qualified names, as the apex assigns them.
    pub hosts: Vec<String>,
}

impl Site {
    /// The template tag it pins.
    #[must_use]
    pub fn tag(&self) -> String {
        format!("{THEME}/v{}", self.version)
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
            version: template["version"].as_str().unwrap_or_default().to_string(),
            features: template["features"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            hosts,
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

/// The tag's features and their defaults, from its `template.yaml`.
fn declared(root: &Path, tag: &str) -> Result<BTreeMap<String, bool>, Error> {
    let text = git(root, &["show", &format!("{tag}:{TEMPLATE}/template.yaml")])?;
    let manifest: Manifest =
        serde_saphyr::from_str(&String::from_utf8_lossy(&text)).map_err(|e| Error::Io {
            path: PathBuf::from(format!("{tag}:{TEMPLATE}/template.yaml")),
            message: e.to_string(),
        })?;
    Ok(manifest
        .features
        .into_iter()
        .map(|(k, f)| (k, f.default))
        .collect())
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

/// The site's `config.toml` with `base_url` first and `[extra.features]` last.
///
/// # Errors
///
/// A message when the site sets either itself.
pub fn site_config(
    text: &str,
    base_url: &str,
    features: &BTreeMap<String, bool>,
) -> Result<String, String> {
    let key = |line: &str, k: &str| {
        line.strip_prefix(k)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_string();
        }
        if section.is_empty() && key(line, "base_url") {
            return Err("sets base_url; cobot writes it from the apex's hosts".into());
        }
        if section == "extra.features" || (section == "extra" && key(line, "features")) {
            return Err("sets extra.features; cobot writes them from static_site".into());
        }
    }
    let mut out = format!("base_url = \"{base_url}\"\n");
    out.push_str(text);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n[extra.features]\n");
    for (k, v) in features {
        let _ = writeln!(out, "{k} = {v}");
    }
    Ok(out)
}

/// Builds each site into `.cobot/<unit>/site/public`.
///
/// # Errors
///
/// [`Error::Git`] for a template tag that does not resolve, [`Error::Io`] for
/// an unknown feature or an unreadable site, [`Error::Tool`] if Zola fails.
pub fn build(root: &Path, sites: &[Site]) -> Result<Vec<Built>, Error> {
    let commit = String::from_utf8_lossy(&git(root, &["rev-parse", "--short", "HEAD"])?)
        .trim()
        .to_string();
    let mut out = Vec::new();
    for site in sites {
        let src = site.dir.join("site");
        let config_path = src.join("config.toml");
        let text = fs::read_to_string(&config_path).map_err(io(&config_path))?;
        let declared = declared(root, &site.tag())?;
        let features = features(site, &declared)?;
        let base = site.canonical().ok_or_else(|| Error::Io {
            path: PathBuf::from(&site.unit),
            message: "the apex assigns this static site no host".into(),
        })?;
        let config =
            site_config(&text, &format!("https://{base}"), &features).map_err(|message| {
                Error::Io {
                    path: PathBuf::from(&site.unit).join("site/config.toml"),
                    message,
                }
            })?;
        let work = root.join(WORK_DIR).join(&site.unit).join("site");
        if work.exists() {
            fs::remove_dir_all(&work).map_err(io(&work))?;
        }
        copy_dir(&src, &work)?;
        fs::write(work.join("config.toml"), config).map_err(io(&work))?;
        let theme = work.join("themes").join(THEME);
        fs::create_dir_all(&theme).map_err(io(&theme))?;
        let tar = git(
            root,
            &[
                "archive",
                "--format=tar",
                &format!("{}:{TEMPLATE}", site.tag()),
            ],
        )?;
        let archive = work.join("themes").join(format!("{THEME}.tar"));
        fs::write(&archive, tar).map_err(io(&archive))?;
        run(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&theme),
            "tar -xf",
        )?;
        fs::remove_file(&archive).map_err(io(&archive))?;
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
/// [`Error::Io`] for an unreadable file.
pub fn expected(public: &Path) -> Result<BTreeMap<String, Upload>, Error> {
    let mut out = BTreeMap::new();
    walk(public, "", &mut out)?;
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

/// The key holding each object's `Content-Type` as last written, which an
/// `ETag` cannot show.
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

    use super::{Site, content_type, features, site_config};

    fn site(features: &[&str]) -> Site {
        Site {
            unit: "workloads/z.com/blog".into(),
            dir: "workloads/z.com/blog".into(),
            zone: "z.com".into(),
            workload: "blog".into(),
            version: "1.0.0".into(),
            features: features.iter().map(|f| (*f).to_string()).collect(),
            hosts: vec!["z.com".into(), "www.z.com".into()],
        }
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
            err.contains("`typo` is not a feature of static-site-template/v1.0.0"),
            "{err}"
        );
    }

    #[test]
    fn the_config_gains_base_url_and_features() {
        let f: BTreeMap<String, bool> = [("feed".to_string(), true)].into();
        let got =
            site_config("title = \"T\"\n\n[extra]\na = 1\n", "https://z.com", &f).expect("ok");
        assert_eq!(
            got,
            "base_url = \"https://z.com\"\ntitle = \"T\"\n\n[extra]\na = 1\n\n[extra.features]\nfeed = true\n"
        );
        assert!(site_config("base_url = \"x\"\n", "https://z.com", &f).is_err());
        assert!(
            site_config(
                "base_url_label = \"x\"\n[extra]\nfeatures_intro = true\n",
                "https://z.com",
                &f
            )
            .is_ok()
        );
        assert!(site_config("[extra]\nfeatures = { feed = true }\n", "https://z.com", &f).is_err());
        assert!(site_config("[extra.features]\nfeed = false\n", "https://z.com", &f).is_err());
        assert_eq!(content_type("a/b.CSS"), "text/css; charset=utf-8");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }
}
