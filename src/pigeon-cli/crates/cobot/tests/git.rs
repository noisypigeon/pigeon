//! Git repositories on Cloudflare Artifacts, against a fake API and local remotes; see
//! `.claude/decisions/2026-11-05-development-git-hosting-cloudflare-artifacts.md`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use cobot::artifacts::{Client, Scope};
use cobot::cloudflare::Api;
use cobot::git::{self, Outcome};
use serde_json::{Value, json};
use tempfile::TempDir;

const TOKEN: &str = "run-token-0123456789";
const GIT_TOKEN: &str = "git-token-abcdef0123";

/// What the fake API holds and saw.
#[derive(Default)]
struct Fake {
    account: String,
    remote: String,
    source: Option<PathBuf>,
    namespaces: Vec<String>,
    repos: BTreeMap<String, Value>,
    calls: Vec<String>,
}

/// A fake Cloudflare API for one account. Every repository's remote is the bare repository
/// `remote`, and an import fills it from `source`.
fn serve(account: &str, remote: &Path, source: Option<&Path>) -> (String, Arc<Mutex<Fake>>) {
    let fake = Arc::new(Mutex::new(Fake {
        account: account.into(),
        remote: format!("file://{}", remote.display()),
        source: source.map(Path::to_path_buf),
        ..Fake::default()
    }));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shared = fake.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or_default().to_string(),
                parts.next().unwrap_or_default().to_string(),
            );
            let (mut authorized, mut length) = (false, 0usize);
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).expect("header");
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("authorization") {
                        authorized = v.trim() == format!("Bearer {TOKEN}");
                    }
                    if k.eq_ignore_ascii_case("content-length") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let (status, result) = if authorized {
                route(&mut shared.lock().expect("lock"), &method, &path, &body)
            } else {
                (403, Value::Null)
            };
            let text = json!({ "success": status == 200, "result": result }).to_string();
            let mut out = stream;
            let _ = write!(
                out,
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    (format!("http://{addr}"), fake)
}

fn route(f: &mut Fake, method: &str, path: &str, body: &Value) -> (u16, Value) {
    f.calls.push(format!("{method} {path}"));
    if path == "/accounts?per_page=2" {
        return (200, json!([{ "id": f.account }]));
    }
    let base = format!("/accounts/{}/artifacts/namespaces", f.account);
    let Some(rest) = path.strip_prefix(&base) else {
        return (404, Value::Null);
    };
    let parts: Vec<&str> = rest
        .trim_start_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    match (method, parts.as_slice()) {
        ("POST", []) => {
            let Some(ns) = body["namespace"].as_str() else {
                return (400, Value::Null);
            };
            f.namespaces.push(ns.into());
            (200, json!({ "namespace": ns }))
        }
        ("GET", [ns]) if f.namespaces.iter().any(|n| n == ns) => (200, json!({ "name": ns })),
        ("POST", [ns, "repos"]) => {
            let name = body["name"].as_str().unwrap_or_default();
            let repo = json!({ "name": name, "remote": f.remote });
            f.repos.insert(format!("{ns}/{name}"), repo.clone());
            (200, repo)
        }
        ("DELETE", [ns, "repos", name]) => f
            .repos
            .remove(&format!("{ns}/{name}"))
            .map_or((404, Value::Null), |_| (200, Value::Null)),
        ("GET", [ns, "repos", name]) => f
            .repos
            .get(&format!("{ns}/{name}"))
            .map_or((404, Value::Null), |r| (200, r.clone())),
        ("POST", [ns, "repos", name, "import"]) => {
            let key = format!("{ns}/{name}");
            if body.get("source").is_some() {
                return (400, Value::Null);
            }
            if f.repos.contains_key(&key) {
                return (409, Value::Null);
            }
            if body["url"].as_str().is_some_and(|u| u.contains("broken")) {
                return (502, Value::Null);
            }
            let repo = json!({ "name": name, "remote": f.remote, "imported_from": body["url"] });
            f.repos.insert(key, repo.clone());
            if let Some(src) = &f.source {
                let into = f.remote.trim_start_matches("file://");
                run_git(src, &["push", "-q", "--mirror", into]);
            }
            (200, repo)
        }
        ("POST", [ns, "repos", name, "tokens"])
            if f.repos.contains_key(&format!("{ns}/{name}")) =>
        {
            (200, json!({ "token": GIT_TOKEN, "scope": body["scope"] }))
        }
        _ => (404, Value::Null),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn copy(root: &Path, rel: &str) {
    let from = repo_root().join(rel);
    if from.is_dir() {
        for e in fs::read_dir(&from).expect("list").flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name != "CHANGELOG.md" {
                copy(root, &format!("{rel}/{name}"));
            }
        }
    } else {
        write(root, rel, &fs::read_to_string(from).expect("read"));
    }
}

fn platform() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    for rel in [
        ".config/providers.yaml",
        ".config/providers.lock.hcl",
        "catalog/cloudflare",
    ] {
        copy(dir.path(), rel);
    }
    dir
}

fn instance(name: &str, units: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    write(
        dir.path(),
        "config.yaml",
        &format!(
            "schema: v1\nname: {name}\ndescription: A test instance.\nrepository: https://example.com/{name}\nunits:\n{units}"
        ),
    );
    dir
}

const PIGEON_UNITS: &str = "  repos/pigeon:
    kind: cloudflare/git-repo
    mirror: https://github.com/noisypigeon/pigeon
  repos/scratch:
    kind: cloudflare/git-repo
    description: An empty one.
";

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare repository with `main`, a tag `v1`, and a `change/wip` branch; its `main` SHA.
fn source(dir: &Path) -> (PathBuf, String) {
    let work = dir.join("work");
    fs::create_dir_all(&work).expect("mkdir");
    run_git(&work, &["init", "-q"]);
    fs::write(work.join("a.txt"), "a").expect("write");
    run_git(&work, &["add", "."]);
    run_git(&work, &["commit", "-q", "-m", "first"]);
    run_git(&work, &["tag", "v1"]);
    run_git(&work, &["checkout", "-q", "-b", "change/wip"]);
    fs::write(work.join("b.txt"), "b").expect("write");
    run_git(&work, &["add", "."]);
    run_git(&work, &["commit", "-q", "-m", "wip"]);
    run_git(&work, &["checkout", "-q", "main"]);
    let bare = dir.join("artifacts.git");
    run_git(
        dir,
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().expect("utf8"),
            bare.to_str().expect("utf8"),
        ],
    );
    let main = run_git(&bare, &["rev-parse", "refs/heads/main"]);
    (bare, main)
}

/// An empty bare repository standing in for Artifacts, and a source to import from.
fn remotes() -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let (src, _) = source(dir.path());
    let remote = dir.path().join("artifacts-empty.git");
    run_git(
        dir.path(),
        &["init", "-q", "--bare", remote.to_str().expect("utf8")],
    );
    (dir, remote, src)
}

#[test]
fn apply_creates_imports_and_then_changes_nothing() {
    let (platform, inst) = (platform(), instance("pigeon", PIGEON_UNITS));
    let (_dir, remote, src) = remotes();
    let (base, fake) = serve("acct-platform-1234", &remote, Some(&src));
    let api = Api::new(Some(&base), TOKEN.into());
    let client = Client::new(&api).expect("one account");
    assert_eq!(client.account_hint(), "…1234");
    let declared = git::declared(inst.path(), platform.path()).expect("declared");
    assert_eq!(declared.namespace, "pigeon");
    let first = git::apply(&client, &declared).expect("apply");
    assert_eq!(
        first
            .iter()
            .map(|a| (a.name.as_str(), a.outcome))
            .collect::<Vec<_>>(),
        [("pigeon", Outcome::Imported), ("scratch", Outcome::Created)]
    );
    {
        let f = fake.lock().expect("lock");
        assert_eq!(f.namespaces, ["pigeon"]);
        assert_eq!(
            f.repos["pigeon/pigeon"]["imported_from"],
            "https://github.com/noisypigeon/pigeon"
        );
        let creates = f
            .calls
            .iter()
            .filter(|c| c.starts_with("POST") && c.ends_with("/namespaces/pigeon/repos"))
            .count();
        assert_eq!(
            creates, 1,
            "only `scratch` is created; `pigeon` is created by its import: {:?}",
            f.calls
        );
        assert!(f.calls.iter().all(|c| c.starts_with("GET /accounts?")
            || c.contains("/accounts/acct-platform-1234/artifacts/namespaces")));
    }
    let posts = |f: &Fake| {
        f.calls
            .iter()
            .filter(|c| c.starts_with("POST") && !c.ends_with("/tokens"))
            .count()
    };
    let before = posts(&fake.lock().expect("lock"));
    let second = git::apply(&client, &declared).expect("apply again");
    assert!(
        second.iter().all(|a| a.outcome == Outcome::Unchanged),
        "{second:?}"
    );
    assert_eq!(
        posts(&fake.lock().expect("lock")),
        before,
        "a second apply creates nothing"
    );
}

#[test]
fn an_import_that_never_landed_is_retried() {
    let (platform, inst) = (platform(), instance("pigeon", PIGEON_UNITS));
    let (_dir, remote, src) = remotes();
    let (base, fake) = serve("acct-platform-1234", &remote, Some(&src));
    {
        let mut f = fake.lock().expect("lock");
        f.namespaces.push("pigeon".into());
        let r = f.remote.clone();
        f.repos.insert(
            "pigeon/pigeon".into(),
            json!({ "name": "pigeon", "remote": r }),
        );
    }
    let api = Api::new(Some(&base), TOKEN.into());
    let client = Client::new(&api).expect("account");
    let declared = git::declared(inst.path(), platform.path()).expect("declared");
    let got = git::apply(&client, &declared).expect("apply");
    assert_eq!(
        got[0].outcome,
        Outcome::Imported,
        "an empty repository with a mirror is replaced by its import"
    );
    assert!(
        fake.lock()
            .expect("lock")
            .calls
            .iter()
            .any(|c| c.starts_with("DELETE") && c.ends_with("/repos/pigeon")),
        "the empty repository is deleted first"
    );

    let tags_only = TempDir::new().expect("t");
    let (full, _) = source(tags_only.path());
    let tagged = tags_only.path().join("tags-only.git");
    run_git(
        tags_only.path(),
        &["init", "-q", "--bare", tagged.to_str().expect("utf8")],
    );
    run_git(
        &full,
        &["push", "-q", tagged.to_str().expect("utf8"), "refs/tags/v1"],
    );
    let (base, fake) = serve("acct-platform-1234", &tagged, Some(&src));
    {
        let mut f = fake.lock().expect("lock");
        f.namespaces.push("pigeon".into());
        let r = f.remote.clone();
        f.repos.insert(
            "pigeon/pigeon".into(),
            json!({ "name": "pigeon", "remote": r }),
        );
    }
    let api = Api::new(Some(&base), TOKEN.into());
    let client = Client::new(&api).expect("account");
    let got = git::apply(&client, &declared).expect("apply");
    assert_eq!(
        got[0].outcome,
        Outcome::Unchanged,
        "a repository holding only tags is never re-imported"
    );
}

#[test]
fn a_failed_replacement_import_restores_the_empty_repository() {
    let platform = platform();
    let inst = instance(
        "pigeon",
        "  repos/pigeon:\n    kind: cloudflare/git-repo\n    mirror: https://github.com/noisypigeon/broken\n",
    );
    let (_dir, remote, src) = remotes();
    let (base, fake) = serve("acct-platform-1234", &remote, Some(&src));
    {
        let mut f = fake.lock().expect("lock");
        f.namespaces.push("pigeon".into());
        let r = f.remote.clone();
        f.repos.insert(
            "pigeon/pigeon".into(),
            json!({ "name": "pigeon", "remote": r }),
        );
    }
    let api = Api::new(Some(&base), TOKEN.into());
    let client = Client::new(&api).expect("account");
    let declared = git::declared(inst.path(), platform.path()).expect("declared");
    let err = git::apply(&client, &declared)
        .expect_err("the import fails")
        .to_string();
    assert!(err.contains("the empty repository is restored"), "{err}");
    assert!(
        fake.lock()
            .expect("lock")
            .repos
            .contains_key("pigeon/pigeon")
    );
}

#[test]
fn each_instance_uses_its_own_namespace_in_the_account_its_token_sees() {
    let platform = platform();
    let (_dir, remote, src) = remotes();
    let np = instance(
        "noisypigeon",
        "  repos/noisypigeon:\n    kind: cloudflare/git-repo\n    mirror: https://github.com/noisypigeon/noisypigeon\n",
    );
    let (base, fake) = serve("acct-other-9876", &remote, Some(&src));
    let api = Api::new(Some(&base), TOKEN.into());
    let client = Client::new(&api).expect("account");
    let declared = git::declared(np.path(), platform.path()).expect("declared");
    git::apply(&client, &declared).expect("apply");
    let f = fake.lock().expect("lock");
    assert_eq!(f.namespaces, ["noisypigeon"]);
    assert!(f.repos.contains_key("noisypigeon/noisypigeon"));
    assert!(
        f.calls.iter().any(|c| c.contains("/accounts/acct-other-9876/artifacts/namespaces/noisypigeon/repos")),
        "the account comes from /accounts, never from config: {:?}",
        f.calls
    );
}

#[test]
fn a_git_repo_unit_never_plans_and_its_settings_are_checked() {
    let platform = platform();
    let inst = instance("pigeon", PIGEON_UNITS);
    let (units, problems) = cobot::unit::discover_in(inst.path(), platform.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert!(
        units.is_empty(),
        "an API unit is no tofu unit: {:?}",
        units.iter().map(|u| &u.path).collect::<Vec<_>>()
    );
    for (bad, want) in [
        ("mirror: http://github.com/x/y", "repos/bad"),
        (
            "mirror: https://github.com/x/y\n    tofu: true",
            "repos/bad",
        ),
    ] {
        let inst = instance(
            "pigeon",
            &format!("  repos/bad:\n    kind: cloudflare/git-repo\n    {bad}\n"),
        );
        let allow =
            cobot::providers::Allowlist::load(&platform.path().join(".config")).expect("allow");
        let got: Vec<String> = cobot::unit::validate_in(inst.path(), platform.path(), &allow)
            .into_iter()
            .map(|p| format!("{}: {}", p.path, p.message))
            .collect();
        assert!(got.iter().any(|p| p.contains(want)), "{bad}: {got:?}");
        assert!(git::declared(inst.path(), platform.path()).is_err());
    }
}

#[test]
fn the_git_token_travels_only_in_the_environment() {
    let c = git::command("https://artifacts.example/r.git", GIT_TOKEN);
    let argv: Vec<String> = c
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(argv.iter().all(|a| !a.contains(GIT_TOKEN)), "{argv:?}");
    let envs: Vec<String> = c
        .get_envs()
        .filter_map(|(k, v)| Some(format!("{}={}", k.to_string_lossy(), v?.to_string_lossy())))
        .collect();
    assert!(
        envs.contains(
            &"GIT_CONFIG_KEY_0=http.https://artifacts.example/r.git.extraHeader".to_string()
        ),
        "{envs:?}"
    );
    assert!(envs.contains(&format!(
        "GIT_CONFIG_VALUE_0=Authorization: Bearer {GIT_TOKEN}"
    )));

    let (platform, inst, remotes) = (
        platform(),
        instance("pigeon", PIGEON_UNITS),
        TempDir::new().expect("t"),
    );
    let (artifacts, _) = source(remotes.path());
    let (url, _) = serve("acct-platform-1234", &artifacts, None);
    let api = Api::new(Some(&url), TOKEN.into());
    let client = Client::new(&api).expect("account");
    let declared = git::declared(inst.path(), platform.path()).expect("declared");
    git::apply(&client, &declared).expect("apply");
    let into = TempDir::new().expect("into");
    let clone = [
        "clone".to_string(),
        "--quiet".into(),
        "{remote}".into(),
        "copy".into(),
    ];
    let status = git::run(
        &client,
        &declared,
        "pigeon",
        Scope::Read,
        &clone,
        into.path(),
    )
    .expect("runs");
    assert!(status.success());
    let config = fs::read_to_string(into.path().join("copy/.git/config")).expect("config");
    assert!(
        !config.contains(GIT_TOKEN) && !config.contains(TOKEN),
        "{config}"
    );
    assert!(!format!("{client:?}{api:?}").contains(TOKEN));
}

/// A source as Artifacts, and a mirror cloned from it as GitHub, with the work tree that made
/// them; the mirror holds `main` and `v1`, plus a branch `keep` of its own.
fn pair(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (artifacts, _) = source(dir);
    let github = dir.join("github.git");
    run_git(
        dir,
        &[
            "clone",
            "-q",
            "--bare",
            artifacts.to_str().expect("utf8"),
            github.to_str().expect("utf8"),
        ],
    );
    run_git(&github, &["branch", "-q", "keep", "main"]);
    run_git(&github, &["branch", "-q", "-D", "change/wip"]);
    (artifacts, github, dir.join("work"))
}

fn stage(dir: &Path, name: &str) -> PathBuf {
    let s = dir.join(name);
    run_git(dir, &["init", "-q", "--bare", s.to_str().expect("utf8")]);
    s
}

fn url(p: &Path) -> String {
    format!("file://{}", p.display())
}

/// Commits on the work tree's `main` and pushes it, with a tag, to `to`.
fn commit_to(work: &Path, to: &Path, file: &str, tag: &str) -> String {
    fs::write(work.join(file), file).expect("write");
    run_git(work, &["add", "."]);
    run_git(work, &["commit", "-q", "-m", file]);
    run_git(work, &["tag", tag]);
    run_git(
        work,
        &["push", "-q", to.to_str().expect("utf8"), "main", tag],
    );
    run_git(work, &["rev-parse", "HEAD"])
}

#[test]
fn a_mirror_fast_forwards_adds_tags_and_never_pushes_changes() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, work) = pair(dir.path());
    run_git(&github, &["tag", "release-only", "main"]);
    let head = commit_to(&work, &artifacts, "next.txt", "v2");
    let done = git::mirror_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        None,
        &stage(dir.path(), "m.git"),
    )
    .expect("mirror");
    assert!(done.refs.iter().any(|(r, _)| r == "refs/tags/v2"));
    assert_eq!(run_git(&github, &["rev-parse", "refs/heads/main"]), head);
    let tags = run_git(
        &github,
        &["for-each-ref", "--format=%(refname)", "refs/tags"],
    );
    assert_eq!(
        tags, "refs/tags/release-only\nrefs/tags/v1\nrefs/tags/v2",
        "tags are only added; a tag Artifacts lacks stays on the mirror"
    );
    let branches = run_git(
        &github,
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
    );
    assert_eq!(
        branches, "refs/heads/keep\nrefs/heads/main",
        "change/* never reaches the mirror"
    );
}

#[test]
fn a_mirror_with_a_token_names_it_nowhere() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, work) = pair(dir.path());
    let head = commit_to(&work, &artifacts, "next.txt", "v2");
    let token = "ghs_mirror-token-never-shown";
    let staged = dir.path().join("job").join("m.git");
    git::bare(&staged).expect("bare");
    let done = git::mirror_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        Some(token),
        &staged,
    )
    .expect("mirror");
    assert_eq!(run_git(&github, &["rev-parse", "refs/heads/main"]), head);
    assert!(!format!("{done:?}").contains(token));
    let config = fs::read_to_string(staged.join("config")).expect("config");
    assert!(!config.contains(token), "the token never reaches a file");
    let gone = dir.path().join("gone.git");
    let err = git::mirror_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&gone),
        Some(token),
        &staged,
    )
    .expect_err("no such mirror")
    .to_string();
    assert!(!err.contains(token), "{err}");
}

#[test]
fn a_mirror_that_moved_ahead_is_refused_until_synced() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, _) = pair(dir.path());
    let release = dir.path().join("release");
    run_git(
        dir.path(),
        &[
            "clone",
            "-q",
            github.to_str().expect("utf8"),
            release.to_str().expect("utf8"),
        ],
    );
    let released = commit_to(&release, &github, "CHANGELOG.md", "cobot/v9.9.9");
    let stage_m = stage(dir.path(), "m.git");
    let err = git::mirror_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        None,
        &stage_m,
    )
    .expect_err("refused")
    .to_string();
    assert!(
        err.contains("run `pigeon cobot git sync pigeon` first"),
        "{err}"
    );
    assert_eq!(
        run_git(&github, &["rev-parse", "refs/heads/main"]),
        released,
        "the mirror is untouched"
    );

    let synced = git::sync_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        &stage(dir.path(), "s.git"),
    )
    .expect("sync");
    assert_eq!(
        synced.advanced.as_ref().map(|(_, to)| to.as_str()),
        Some(released.as_str())
    );
    assert_eq!(synced.tags, 1);
    assert_eq!(
        run_git(&artifacts, &["rev-parse", "refs/heads/main"]),
        released
    );
    assert_eq!(
        run_git(
            &artifacts,
            &["rev-parse", "refs/tags/cobot/v9.9.9^{commit}"]
        ),
        released
    );
    git::mirror_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        None,
        &stage_m,
    )
    .expect("a synced mirror takes the next push");
}

#[test]
fn a_sync_refuses_histories_that_diverged() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, work) = pair(dir.path());
    let release = dir.path().join("release");
    run_git(
        dir.path(),
        &[
            "clone",
            "-q",
            github.to_str().expect("utf8"),
            release.to_str().expect("utf8"),
        ],
    );
    commit_to(&release, &github, "released.txt", "r1");
    let ours = commit_to(&work, &artifacts, "ours.txt", "o1");
    let err = git::sync_from(
        &url(&artifacts),
        GIT_TOKEN,
        "main",
        "pigeon",
        &url(&github),
        &stage(dir.path(), "s.git"),
    )
    .expect_err("diverged")
    .to_string();
    assert!(err.contains("has diverged"), "{err}");
    assert_eq!(
        run_git(&artifacts, &["rev-parse", "refs/heads/main"]),
        ours,
        "Artifacts is untouched"
    );
}

#[test]
fn a_change_head_is_force_pushed_to_its_branch_and_deleted_after() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, work) = pair(dir.path());
    let first = run_git(&artifacts, &["rev-parse", "refs/heads/change/wip"]);
    let token = "ghs_change-token-never-shown";
    let staged = dir.path().join("job").join("c.git");
    git::bare(&staged).expect("bare");
    let push = |sha: &str| {
        git::push_change(
            &url(&artifacts),
            GIT_TOKEN,
            "change/wip",
            sha,
            &url(&github),
            Some(token),
            &staged,
        )
    };
    push(&first).expect("first head");
    assert_eq!(
        run_git(&github, &["rev-parse", "refs/heads/change/wip"]),
        first
    );
    run_git(&work, &["checkout", "-q", "-B", "change/wip", "main"]);
    fs::write(work.join("c.txt"), "c").expect("write");
    run_git(&work, &["add", "."]);
    run_git(&work, &["commit", "-q", "-m", "rewritten"]);
    run_git(
        &work,
        &[
            "push",
            "-q",
            "-f",
            artifacts.to_str().expect("utf8"),
            "change/wip",
        ],
    );
    let second = run_git(&work, &["rev-parse", "HEAD"]);
    push(&second).expect("a new head replaces the old");
    assert_eq!(
        run_git(&github, &["rev-parse", "refs/heads/change/wip"]),
        second
    );
    let err = push(&first).expect_err("not the branch's").to_string();
    assert!(err.contains("no longer holds"), "{err}");
    let config = fs::read_to_string(staged.join("config")).expect("config");
    assert!(!config.contains(token), "the token never reaches a file");
    git::delete_change("change/wip", &url(&github), Some(token), &staged).expect("delete");
    git::delete_change("change/wip", &url(&github), Some(token), &staged)
        .expect("a branch already gone is not an error");
    let branches = run_git(
        &github,
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
    );
    assert_eq!(branches, "refs/heads/keep\nrefs/heads/main");
}

#[test]
fn only_a_change_branch_is_pushed_or_deleted_this_way() {
    let dir = TempDir::new().expect("tempdir");
    let (artifacts, github, _) = pair(dir.path());
    let main = run_git(&artifacts, &["rev-parse", "refs/heads/main"]);
    let staged = stage(dir.path(), "c.git");
    for branch in [
        "main",
        "keep",
        "change/",
        "change/../main",
        "refs/heads/main",
    ] {
        let err = git::push_change(
            &url(&artifacts),
            GIT_TOKEN,
            branch,
            &main,
            &url(&github),
            None,
            &staged,
        )
        .expect_err(branch)
        .to_string();
        assert!(err.contains("not a change's branch"), "{branch}: {err}");
        assert!(git::delete_change(branch, &url(&github), None, &staged).is_err());
    }
    let branches = run_git(
        &github,
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
    );
    assert_eq!(branches, "refs/heads/keep\nrefs/heads/main");
}
