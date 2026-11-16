//! Each template builds with Zola from its scaffold and a site's settings, as `sites build`
//! assembles it; see `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §5.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use include_dir::Dir;
use serde_json::Value;

/// Settings at their `x-site` targets, as the builder writes them for one site.
const BLOG: &str = r##"
[extra]
intro = "Hi."
avatar = "/a.svg"
social_preview = "/a.svg"
author = { page = "/pages/about" }
nav = [{ title = "About", link = "/pages/about" }]
theme_color = { light = "#eeeeee", dark = "#111111" }
"##;

/// Whether `zola` runs; a missing one fails under `PIGEON_REQUIRE_TOOLS`, per
/// `.claude/decisions/2026-11-16-development-one-test-binary-per-crate.md` §4.
fn zola() -> bool {
    let ok = Command::new("zola")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        ok || std::env::var_os("PIGEON_REQUIRE_TOOLS").is_none(),
        "zola is not on PATH and PIGEON_REQUIRE_TOOLS is set"
    );
    if !ok {
        eprintln!("skipped: zola is not on PATH");
    }
    ok
}

/// `[extra.features]` from the template's declared defaults.
fn features(name: &str) -> String {
    let manifest: Value =
        serde_saphyr::from_str(site_templates::text(name, "template.yaml").expect("template.yaml"))
            .expect("template.yaml");
    let mut out = String::from("\n[extra.features]\n");
    for (f, decl) in manifest["features"].as_object().expect("features") {
        writeln!(out, "{f} = {}", decl["default"].as_bool().expect("default")).expect("write");
    }
    out
}

/// The site, built: the theme under `themes/`, the scaffold at the root, then `settings`.
fn build(name: &str, settings: &str, content: &[(&str, &str)]) -> tempfile::TempDir {
    let work = tempfile::TempDir::new().expect("work");
    let root = work.path();
    let dir = site_templates::get(name).expect("template");
    let theme = root.join("themes").join(format!("site-template-{name}"));
    fs::create_dir_all(&theme).expect("themes");
    write(dir, &theme);
    write(dir.get_dir("scaffold").expect("scaffold"), root);
    let config = fs::read_to_string(root.join("config.toml")).expect("config.toml");
    let head =
        "base_url = \"https://example.com/docs\"\ntitle = \"Site\"\ndescription = \"A site.\"\n";
    fs::write(
        root.join("config.toml"),
        format!("{head}{config}{settings}{}", features(name)),
    )
    .expect("config.toml");
    for (path, text) in content {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(path, text).expect("content");
    }
    let out = Command::new("zola")
        .arg("build")
        .current_dir(root)
        .env(
            "SITE_COMMIT_SHA",
            "0000000000000000000000000000000000000000",
        )
        .output()
        .expect("zola build");
    assert!(
        out.status.success(),
        "{name}: zola build failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    work
}

fn write(dir: &Dir<'_>, to: &Path) {
    for file in dir.files() {
        let rel = file.path().strip_prefix(dir.path()).expect("under its dir");
        fs::write(to.join(rel), file.contents()).expect("file");
    }
    for sub in dir.dirs() {
        let rel = sub.path().strip_prefix(dir.path()).expect("under its dir");
        fs::create_dir_all(to.join(rel)).expect("dir");
        write(sub, &to.join(rel));
    }
}

#[test]
fn the_blog_builds_with_zola() {
    if !zola() {
        return;
    }
    let site = build(
        "blog",
        BLOG,
        &[(
            "content/posts/2026-11-16-hello.md",
            "+++\ntitle = \"Hello\"\ndate = 2026-11-16\n+++\nHello.\n",
        )],
    );
    let index = fs::read_to_string(site.path().join("public/index.html")).expect("index.html");
    assert!(index.contains("Hello"), "the post is listed");
    assert!(site.path().join("public/posts/hello/index.html").is_file());
}

#[test]
fn the_architecture_template_builds_with_zola() {
    if !zola() {
        return;
    }
    let site = build("architecture", "", &[]);
    assert!(site.path().join("public/index.html").is_file());
    assert!(site.path().join("public/assets/js/rail.js").is_file());
}
