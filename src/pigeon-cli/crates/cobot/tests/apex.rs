//! Zone apexes: layout, presets, claims, workload secrets and the token handoff.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};
use tempfile::TempDir;

const STATE_PASS: &str = "state-passphrase-0123456789";
const SECRETS_PASS: &str = "secrets-passphrase-0123456789";
const WORKLOAD_PASS: &str = "workload-passphrase-0123456789";
const APEX: &str = "workloads/cloudflare/apex";
const ZONE: &str = "noisypigeon.com";

const PROTON: &str = "apex: blog
services:
  proton:
    verification:
      protonmail: { secret: protonmail_verification }
      pm: { secret: pm_verification }
    dmarc: default
records: {}
";

const CLOUDFLARE: &str = "zones: [noisypigeon.com, pigeon.dev]
rotation: { period_days: 90, overlap_days: 7 }
tokens:
  - name: apex-noisypigeon.com
    capabilities:
      - { permission: DNS Write, zones: [noisypigeon.com] }
  - name: apex-pigeon.dev
    capabilities:
      - { permission: DNS Write, zones: [pigeon.dev] }
";

/// The apex config with `body` as `noisypigeon.com`'s apex.
fn apex(body: &str) -> String {
    zones(&[(ZONE, body)])
}

fn zones(each: &[(&str, &str)]) -> String {
    let mut out = String::from("zones:\n");
    for (zone, body) in each {
        let _ = writeln!(out, "  {zone}:");
        for line in body.lines() {
            let _ = writeln!(out, "    {line}");
        }
    }
    out
}

/// One zone's record by key.
fn rec<'a>(got: &'a serde_json::Map<String, Value>, key: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    got.get(&format!("{ZONE}:{key}")).unwrap_or(&NULL)
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
            if name != "CHANGELOG.md" && name != "secrets.config.yaml" {
                copy(root, &format!("{rel}/{name}"));
            }
        }
    } else {
        write(root, rel, &fs::read_to_string(from).expect("read"));
    }
}

/// A repository with the real `infra/cloudflare` and an apex for `noisypigeon.com`.
fn repo(body: &str) -> TempDir {
    repo_with(&apex(body))
}

fn repo_with(config: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    for rel in [
        ".config/providers.yaml",
        ".config/providers.lock.hcl",
        "infra/cloudflare",
    ] {
        copy(root, rel);
    }
    write(root, "infra/cloudflare/config.yaml", CLOUDFLARE);
    let cf = root.join("infra/cloudflare/secrets.config.yaml");
    secrets_file(
        &cf,
        &["account_id", "provisioner_token"],
        |_| "v".into(),
        SECRETS_PASS,
    );
    write(root, &format!("{APEX}/config.yaml"), config);
    set_secrets(root, &["protonmail_verification", "pm_verification"]);
    dir
}

/// Encrypted values by key and passphrase; scrypt is slow, so each is made once.
static ENCRYPTED: LazyLock<Mutex<BTreeMap<(String, String), String>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn secrets_file(path: &Path, keys: &[&str], value: impl Fn(&str) -> String, pass: &str) {
    let mut cache = ENCRYPTED.lock().expect("lock");
    let mut text = String::new();
    for k in keys {
        let enc = cache
            .entry(((*k).to_string(), pass.to_string()))
            .or_insert_with(|| cobot::secrets::encrypt(&value(k), pass).expect("encrypt"));
        let _ = writeln!(text, "{k}: {enc}");
    }
    if keys.is_empty() {
        let _ = fs::remove_file(path);
    } else {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }
}

fn set_secrets(root: &Path, keys: &[&str]) {
    let path = root.join(APEX).join("secrets.config.yaml");
    secrets_file(&path, keys, |k| format!("value-of-{k}"), WORKLOAD_PASS);
}

fn problems(root: &Path) -> Vec<String> {
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist loads");
    cobot::unit::validate(root, &allow)
        .into_iter()
        .map(|p| format!("{}: {}", p.path, p.message))
        .collect()
}

fn assert_fails(root: &Path, want: &str) {
    let got = problems(root);
    assert!(
        got.iter().any(|p| p.contains(want)),
        "want {want:?}: {got:?}"
    );
}

fn render(root: &Path) -> Value {
    let units = cobot::select(root, &root.join(".config"), &[APEX.to_string()]).expect("valid");
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    cobot::render_json(
        &units[0],
        Path::new("/state"),
        &allow,
        0,
        &cobot::config::Existing::new(),
    )
}

fn records(root: &Path) -> serde_json::Map<String, Value> {
    render(root)["module"]["dns_records"]["for_each"]
        .as_object()
        .cloned()
        .expect("records")
}

#[test]
fn proton_expands_to_its_nine_records() {
    let repo = repo(PROTON);
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let got = records(repo.path());
    let keys: Vec<&str> = got
        .keys()
        .map(|k| k.strip_prefix("noisypigeon.com:").expect("zone key"))
        .collect();
    assert_eq!(
        keys,
        [
            "proton-dkim",
            "proton-dkim02",
            "proton-dkim03",
            "proton-dmarc",
            "proton-mx1",
            "proton-mx2",
            "proton-spf",
            "proton-verification-pm",
            "proton-verification-protonmail",
        ]
    );
    assert_eq!(
        rec(&got, "proton-mx1"),
        &json!({ "zone": ZONE, "name": "noisypigeon.com", "type": "MX", "content": "mx1.alias.proton.me",
                "ttl": 1, "priority": 10, "proxied": false })
    );
    assert_eq!(
        rec(&got, "proton-dkim02")["name"],
        "dkim02._domainkey.noisypigeon.com"
    );
    assert_eq!(
        rec(&got, "proton-verification-pm")["content"],
        "pm-verification=${var.secrets[\"pm_verification\"]}"
    );
    assert_eq!(
        rec(&got, "proton-dmarc")["content"],
        "v=DMARC1; p=quarantine; pct=100; adkim=s; aspf=s"
    );
    let text = serde_json::to_string(&got).expect("json");
    assert!(!text.contains("value-of-"), "{text}");
}

#[test]
fn unset_inputs_skip_records_and_overrides_change_only_their_fields() {
    let repo = repo(&PROTON.replace("dmarc: default", "dmarc: false").replace(
        "records: {}",
        "records:\n  proton-spf: { content: \"v=spf1 include:alias.proton.me include:x.example ~all\" }\n  proton-mx2: false\n  own-dmarc: { name: _dmarc, type: TXT, content: \"v=DMARC1; p=none\" }",
    ));
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let got = records(repo.path());
    assert!(rec(&got, "proton-dmarc").is_null() && rec(&got, "proton-mx2").is_null());
    assert_eq!(
        rec(&got, "proton-spf"),
        &json!({ "zone": ZONE, "name": "noisypigeon.com", "type": "TXT",
                "content": "v=spf1 include:alias.proton.me include:x.example ~all",
                "ttl": 1, "priority": null, "proxied": false })
    );
    assert_eq!(rec(&got, "own-dmarc")["name"], "_dmarc.noisypigeon.com");

    let repo = self::repo(&PROTON.replace(
        "records: {}",
        "records:\n  proton-mx1: { type: TXT, content: maintenance }",
    ));
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let got = records(repo.path());
    assert_eq!(rec(&got, "proton-mx1")["type"], "TXT");
    assert!(rec(&got, "proton-mx1")["priority"].is_null());

    let repo = self::repo(
        &PROTON
            .replace("    verification:\n      protonmail: { secret: protonmail_verification }\n      pm: { secret: pm_verification }\n", ""),
    );
    set_secrets(repo.path(), &[]);
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    assert_eq!(records(repo.path()).len(), 7);
}

#[test]
fn unknown_keys_and_bad_records_fail() {
    for (records, want) in [
        ("proton-typo: false", "drops no preset or host record"),
        (
            "proton-typo: { content: x }",
            "`proton` names no such record",
        ),
        ("host-typo: { content: x }", "`host` names no such record"),
        (
            "half: { name: x, type: TXT }",
            "needs name, type and content",
        ),
        (
            "mx: { name: mail, type: MX, content: m.example }",
            "needs a priority",
        ),
        (
            "t: { name: t, type: TXT, content: x, proxied: true }",
            "can be proxied",
        ),
        (
            "t: { name: t, type: TXT, content: x, priority: 1 }",
            "only MX records",
        ),
        (
            "t: { name: t.noisypigeon.com, type: TXT, content: x }",
            "relative to the zone",
        ),
    ] {
        let repo = repo(&PROTON.replace("records: {}", &format!("records:\n  {records}")));
        assert_fails(repo.path(), want);
    }
    let repo = repo(&PROTON.replace("dmarc: default", "dmarc: 7"));
    assert_fails(repo.path(), "must be a string");
    let repo = self::repo(&PROTON.replace("dmarc: default", "dmarc: default\n    extra: 1"));
    assert_fails(repo.path(), "unknown input `extra`");
    let repo = self::repo(&PROTON.replace("proton:", "nothing: {}\n  proton:"));
    assert_fails(repo.path(), "no preset");
}

#[test]
fn claims_conflict_naming_both_owners() {
    let repo = repo(&PROTON.replace(
        "records: {}",
        "records:\n  spf: { name: \"@\", type: TXT, content: \"v=spf1 -all\" }",
    ));
    assert_fails(repo.path(), "@ SPF is claimed by proton and records.spf");

    let repo = self::repo(&PROTON.replace("services:", "services:\n  other: {}"));
    write(
        repo.path(),
        "infra/cloudflare/presets/other.yaml",
        "description: Another mail service.\nclaims:\n  - { name: \"@\", type: MX }\nrecords:\n  mx: { name: \"@\", type: MX, content: m.example, priority: 5 }\n",
    );
    assert_fails(repo.path(), "@ MX is claimed by other and proton");

    let repo = self::repo(PROTON);
    write(
        repo.path(),
        "workloads/noisypigeon.com/www/unit.yaml",
        "schema: 0\nname: www\nkind: workload\nmodules: []\n",
    );
    assert_fails(
        repo.path(),
        "www ADDRESS is claimed by workload blog and workload www",
    );

    let repo =
        self::repo(&PROTON.replace("records: {}", "records:\n  host-blog-www: { name: shop }"));
    workload(
        repo.path(),
        "blog",
        "ingress: { type: CNAME, content: b.example }\n",
    );
    workload(repo.path(), "shop", "{}\n");
    assert_fails(
        repo.path(),
        "shop ADDRESS is claimed by workload shop and workload blog",
    );

    let repo = self::repo(&PROTON.replace(
        "records: {}",
        "records:\n  backup-mx: { name: mail, type: MX, content: m.example, priority: 5 }",
    ));
    workload(
        repo.path(),
        "mail",
        "ingress: { type: CNAME, content: m.example }\n",
    );
    assert_fails(
        repo.path(),
        "mail holds a CNAME (host-mail-mail), which shares its name with no other record: backup-mx",
    );
}

fn workload(root: &Path, name: &str, config: &str) {
    write(
        root,
        &format!("workloads/noisypigeon.com/{name}/schema.json"),
        r#"{ "type": "object", "x-cobot": { "version": "0.1.0" }, "properties": { "ingress": { "type": "object" } } }"#,
    );
    write(
        root,
        &format!("workloads/noisypigeon.com/{name}/config.yaml"),
        config,
    );
}

#[test]
fn hosts_render_only_for_workloads_that_exist_with_ingress() {
    let repo = repo(PROTON);
    assert!(!records(repo.path()).keys().any(|k| k.contains(":host-")));

    workload(
        repo.path(),
        "blog",
        "ingress: { type: CNAME, content: blog.pages.example }\n",
    );
    workload(repo.path(), "docs", "{}\n");
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let got = records(repo.path());
    assert_eq!(
        rec(&got, "host-blog-apex"),
        &json!({ "zone": ZONE, "name": "noisypigeon.com", "type": "CNAME", "content": "blog.pages.example",
                "ttl": 1, "priority": null, "proxied": true })
    );
    assert_eq!(rec(&got, "host-blog-www")["name"], "www.noisypigeon.com");
    assert!(!got.keys().any(|k| k.contains(":host-docs")));

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &apex(&PROTON.replace("services:", "hosts: { blog: [journal] }\nservices:")),
    );
    let got = records(repo.path());
    assert!(rec(&got, "host-blog-journal").is_object() && rec(&got, "host-blog-www").is_null());

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &apex(&PROTON.replace("services:", "hosts: { blog: [a.b, a-b] }\nservices:")),
    );
    assert_fails(
        repo.path(),
        "`a.b` and `a-b` would share the record key `host-blog-a-b`",
    );

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &apex(&PROTON.replace("services:", "hosts: { ghost: [g] }\nservices:")),
    );
    assert_fails(repo.path(), "hosts.ghost: no workload");
    workload(repo.path(), "blog", "ingress: { type: MX, content: x }\n");
    assert_fails(repo.path(), "ingress `type` must be");
}

#[test]
fn the_layout_follows_the_zones() {
    type Edit = Box<dyn Fn(&Path)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "unlisted zone directory",
            Box::new(|r| write(r, "workloads/example.org/blog/unit.yaml", "x: 1\n")),
            "`example.org` is not a zone in infra/cloudflare",
        ),
        (
            "unlisted apex zone",
            Box::new(|r| {
                write(
                    r,
                    &format!("{APEX}/config.yaml"),
                    &zones(&[("example.org", "services: {}\nrecords: {}")]),
                );
            }),
            "zones.example.org: not a zone in infra/cloudflare",
        ),
        (
            "file in a zone",
            Box::new(|r| write(r, "workloads/noisypigeon.com/notes.md", "x\n")),
            "a zone holds only workload directories",
        ),
        (
            "apex without config",
            Box::new(|r| write(r, "workloads/noisypigeon.com/apex/unit.yaml", "x: 1\n")),
            "an apex holds config.yaml",
        ),
        (
            "apex with its own schema",
            Box::new(|r| write(r, &format!("{APEX}/schema.json"), "{}\n")),
            "never its own",
        ),
        (
            "more than apex under cloudflare",
            Box::new(|r| write(r, "workloads/cloudflare/other/x.md", "x\n")),
            "holds only apex/",
        ),
        (
            "missing token",
            Box::new(|r| {
                write(
                    r,
                    "infra/cloudflare/config.yaml",
                    &CLOUDFLARE.replace("apex-noisypigeon", "other-noisypigeon"),
                );
            }),
            "needs a `apex-noisypigeon.com` token",
        ),
        (
            "broad token",
            Box::new(|r| {
                write(
                    r,
                    "infra/cloudflare/config.yaml",
                    &CLOUDFLARE.replacen(
                        "zones: [noisypigeon.com] }",
                        "zones: [noisypigeon.com, pigeon.dev] }",
                        1,
                    ),
                );
            }),
            "exactly `DNS Write` on `noisypigeon.com`",
        ),
    ];
    for (what, edit, want) in cases {
        let repo = repo(PROTON);
        edit(repo.path());
        let got = problems(repo.path());
        assert!(got.iter().any(|p| p.contains(want)), "{what}: {got:?}");
    }
}

#[test]
fn two_zones_get_their_own_providers_and_keys() {
    let repo = repo_with(&zones(&[
        (ZONE, PROTON),
        (
            "pigeon.dev",
            "services: {}\nrecords:\n  site: { name: \"@\", type: A, content: 192.0.2.1 }",
        ),
    ]));
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let json = render(repo.path());
    let provider = &json["provider"]["cloudflare"][0];
    assert_eq!(provider["alias"], "zone");
    assert_eq!(
        provider["for_each"],
        "${toset([\"noisypigeon.com\",\"pigeon.dev\"])}"
    );
    assert_eq!(provider["api_token"], "${var.handoff[each.key].api_token}");
    let m = &json["module"]["dns_records"];
    assert_eq!(
        m["providers"]["cloudflare"],
        "cloudflare.zone[each.value.zone]"
    );
    assert_eq!(m["zone_id"], "${var.handoff[each.value.zone].zone_id}");
    assert_eq!(m["for_each"]["pigeon.dev:site"]["zone"], "pigeon.dev");
    assert_eq!(m["for_each"].as_object().expect("map").len(), 10);
}

#[test]
fn optional_booleans_unset_on_false() {
    let repo = repo(&PROTON.replace("services:", "services:\n  flag: { enable: false }"));
    write(
        repo.path(),
        "infra/cloudflare/presets/flag.yaml",
        "description: A flag.\ninputs:\n  enable: { type: boolean, optional: true }\nclaims:\n  - { name: flag, type: ADDRESS, when: enable }\nrecords:\n  on: { name: flag, type: TXT, content: \"on=${enable}\" }\n",
    );
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    assert!(rec(&records(repo.path()), "flag-on").is_null());
}

#[test]
fn injected_names_and_secret_keys_are_rejected() {
    let repo = repo(&PROTON.replace(
        "services:",
        "hosts: { blog: [\"x${file(\\\"/etc/passwd\\\")}\"] }\nservices:",
    ));
    assert_fails(repo.path(), "is not a host name");
    let repo =
        self::repo(&PROTON.replace("{ secret: pm_verification }", "{ secret: \"a\\\"]}${x\" }"));
    assert_fails(repo.path(), "a secret is `{ secret: <key> }`");
}

#[test]
fn workload_secrets_are_their_own_and_never_break_glass() {
    let repo = repo(&PROTON.replace("pm_verification }", "account_id }"));
    set_secrets(repo.path(), &["protonmail_verification", "account_id"]);
    assert_fails(repo.path(), "`account_id` is an infra/ key");

    let repo = self::repo(PROTON);
    set_secrets(repo.path(), &["protonmail_verification"]);
    assert_fails(repo.path(), "differ from x-cobot.secrets");

    let w = Some(WORKLOAD_PASS.to_string());
    let i = Some(SECRETS_PASS.to_string());
    assert_eq!(
        cobot::secrets_passphrase_for(APEX, i.clone(), w.clone()).expect("workload"),
        WORKLOAD_PASS
    );
    assert_eq!(
        cobot::secrets_passphrase_for("infra/cloudflare", i.clone(), w).expect("infra"),
        SECRETS_PASS
    );
    assert!(matches!(
        cobot::secrets_passphrase_for(APEX, i.clone(), None),
        Err(cobot::Error::WorkloadSecretsPassphrase)
    ));
    assert!(matches!(
        cobot::secrets_passphrase_for(APEX, i.clone(), i),
        Err(cobot::Error::SamePassphrase)
    ));
}

fn tofu_available() -> bool {
    Command::new("tofu")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn data_module(root: &Path, rel: &str, vars: &[&str], output: &str) {
    let mut tf = String::new();
    for v in vars {
        let _ = write!(
            tf,
            "variable \"{v}\" {{\n  type    = any\n  default = null\n}}\n"
        );
    }
    let input = vars
        .iter()
        .map(|v| format!("{v} = var.{v}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        tf,
        "resource \"terraform_data\" \"this\" {{\n  input = {{ {input} }}\n}}\n\noutput \"id\" {{\n  value = terraform_data.this.id\n}}\n{output}"
    );
    write(root, &format!("{rel}/main.tf"), &tf);
    write(
        root,
        &format!("{rel}/module.yaml"),
        "version: 0.1.0\ndescription: A test module.\n",
    );
}

/// A built-in-only `infra/cloudflare`, shaped like the real one, and the apex.
fn builtin_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    copy(root, "infra/cloudflare/presets/proton.yaml");
    let real: Value = serde_json::from_str(
        &fs::read_to_string(repo_root().join("infra/cloudflare/schema.json")).expect("schema"),
    )
    .expect("json");
    let mut apex_x = real["x-cobot"]["apex"].clone();
    apex_x["providers"] = json!({});
    data_module(
        root,
        "infra/cloudflare/modules/dns-record",
        &[
            "zone_id", "name", "type", "content", "ttl", "priority", "proxied",
        ],
        "",
    );
    let cf = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "$defs": real["$defs"],
        "x-cobot": {
            "version": "0.1.0",
            "apex": apex_x,
            "outputs": { "zone_ids": "$module.zones[*].id", "tokens": "$module.tokens[*].value" }
        },
        "properties": {
            "zones": { "type": "array", "x-cobot": { "module": "zone", "for_each": "$item", "inputs": { "name": "$item" } } },
            "rotation": { "type": "object" },
            "tokens": {
                "type": "array",
                "x-cobot": {
                    "module": "token",
                    "for_each": "$item.name",
                    "rotate": { "period_days": "$config.rotation.period_days", "overlap_days": "$config.rotation.overlap_days" },
                    "inputs": { "name": "$item.name", "not_before": "$generation.not_before" }
                }
            }
        }
    });
    write(root, "infra/cloudflare/schema.json", &cf.to_string());
    write(root, "infra/cloudflare/config.yaml", CLOUDFLARE);
    data_module(root, "infra/cloudflare/modules/zone", &["name"], "");
    data_module(
        root,
        "infra/cloudflare/modules/token",
        &["name", "not_before"],
        "output \"value\" {\n  value     = \"tok-${var.name}-${var.not_before}\"\n  sensitive = true\n}\n",
    );
    write(root, &format!("{APEX}/config.yaml"), &apex(PROTON));
    set_secrets(root, &["protonmail_verification", "pm_verification"]);
    dir
}

fn ctx(root: &Path, state: &Path) -> cobot::Context {
    ctx_on(root, state, "2026-11-05")
}

fn ctx_on(root: &Path, state: &Path, day: &str) -> cobot::Context {
    cobot::Context {
        root: root.to_path_buf(),
        providers: root.join(".config"),
        state: state.to_path_buf(),
        state_passphrase: STATE_PASS.into(),
        secrets_passphrase: Some(SECRETS_PASS.into()),
        workload_secrets_passphrase: Some(WORKLOAD_PASS.into()),
        today: cobot::generations::day(day).expect("day"),
    }
}

fn grep(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with(".terraform") {
                    stack.push(p);
                }
            } else if fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle)) {
                hits.push(p);
            }
        }
    }
    hits
}

#[test]
fn the_apex_token_is_handed_off_after_infra_applies() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = builtin_repo();
    let root = repo.path();
    let state = TempDir::new().expect("state");
    let units = cobot::select(root, &root.join(".config"), &[]).expect("valid");
    assert_eq!(
        units.iter().map(|u| u.path.as_str()).collect::<Vec<_>>(),
        ["infra/cloudflare", APEX]
    );
    let apex_only = cobot::select(root, &root.join(".config"), &[APEX.into()]).expect("valid");

    let plans = cobot::plan(&ctx(root, state.path()), &units).expect("plan");
    let summary = cobot::summary(&plans);
    assert!(summary.contains("create `output.tokens`"), "{summary}");
    assert_eq!(plans[1].counts(), (9, 0, 0), "{summary}");
    assert!(summary.contains("Credentials pending"), "{summary}");
    assert!(matches!(
        cobot::apply(&ctx(root, state.path()), &apex_only),
        Err(cobot::Error::Pending { .. })
    ));

    let applied = cobot::apply(&ctx(root, state.path()), &units).expect("apply");
    assert!(applied[1].note.is_none(), "{}", cobot::summary(&applied));
    assert_eq!(applied[1].counts(), (9, 0, 0));

    let again = cobot::plan(&ctx(root, state.path()), &units).expect("replan");
    assert!(
        again.iter().all(|p| !p.has_changes() && p.note.is_none()),
        "{}",
        cobot::summary(&again)
    );
    let expired =
        cobot::plan(&ctx_on(root, state.path(), "2027-06-01"), &apex_only).expect("expired");
    let note = expired[0].note.clone().unwrap_or_default();
    assert!(
        note.contains("refresh skipped") && note.contains("apex-noisypigeon.com"),
        "{note}"
    );

    for secret in ["tok-apex-noisypigeon.com", "value-of-pm_verification"] {
        assert_eq!(
            grep(&root.join(".cobot"), secret),
            Vec::<PathBuf>::new(),
            "{secret}"
        );
        assert_eq!(
            grep(state.path(), secret),
            Vec::<PathBuf>::new(),
            "{secret}"
        );
    }
}

#[test]
fn absolute_hosts_and_orphan_workloads_are_rejected() {
    let repo = repo(&PROTON.replace(
        "services:",
        "hosts: { blog: [www.noisypigeon.com] }\nservices:",
    ));
    assert_fails(
        repo.path(),
        "`www.noisypigeon.com` must be relative to the zone",
    );
    let repo = self::repo(PROTON);
    write(
        repo.path(),
        "workloads/noisypigeon.com/blog/README.md",
        "x\n",
    );
    assert_fails(
        repo.path(),
        "a workload directory holds config.yaml or unit.yaml",
    );
}

#[test]
fn secret_mail_policies_are_claimed_once_decrypted() {
    let config = apex(&PROTON.replace(
        "records: {}",
        "records:\n  extra: { name: \"@\", type: TXT, content: { secret: extra_spf } }",
    ));
    let repo = repo_with(&config);
    set_secrets(
        repo.path(),
        &["protonmail_verification", "pm_verification", "extra_spf"],
    );
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let parsed: Value = serde_saphyr::from_str(&config).expect("yaml");
    let site = cobot::apex::Site {
        root: repo.path().to_path_buf(),
        zone: None,
    };
    let home = repo.path().join("infra/cloudflare");
    let mut secrets = BTreeMap::new();
    secrets.insert("extra_spf".to_string(), "v=spf1 -all".to_string());
    let found = cobot::apex::check_decrypted(&parsed, &home, &site, &secrets);
    assert!(
        found
            .iter()
            .any(|p| p.contains("@ SPF is claimed by proton and records.extra")),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|p| p.contains("v=spf1 -all")),
        "{found:?}"
    );
    secrets.insert("extra_spf".to_string(), "site-verification=x".to_string());
    assert_eq!(
        cobot::apex::check_decrypted(&parsed, &home, &site, &secrets),
        Vec::<String>::new()
    );
}

const PER_ZONE: &str = "workloads/noisypigeon.com/apex";

/// Moves the all-zones apex's `noisypigeon.com` entry to `workloads/noisypigeon.com/apex`.
fn to_per_zone(root: &Path, body: &str) {
    let secrets = fs::read(root.join(APEX).join("secrets.config.yaml")).expect("secrets");
    fs::remove_dir_all(root.join("workloads/cloudflare")).expect("rm");
    write(root, &format!("{PER_ZONE}/config.yaml"), body);
    fs::write(root.join(PER_ZONE).join("secrets.config.yaml"), secrets).expect("secrets");
}

fn render_of(root: &Path, unit: &str) -> Value {
    let units = cobot::select(root, &root.join(".config"), &[unit.to_string()]).expect("valid");
    let allow = cobot::providers::Allowlist::load(&root.join(".config")).expect("allowlist");
    let mut json = cobot::render_json(
        &units[0],
        Path::new("/state"),
        &allow,
        0,
        &cobot::config::Existing::new(),
    );
    json["terraform"]["backend"] = Value::Null;
    json
}

#[test]
fn a_per_zone_apex_renders_as_its_zone_in_the_all_zones_apex() {
    let repo = repo(PROTON);
    let all = render_of(repo.path(), APEX);
    to_per_zone(repo.path(), PROTON);
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    assert_eq!(render_of(repo.path(), PER_ZONE), all);
}

#[test]
fn per_zone_apexes_follow_the_layout() {
    let repo = repo(PROTON);
    write(repo.path(), &format!("{PER_ZONE}/config.yaml"), PROTON);
    assert_fails(repo.path(), "`noisypigeon.com` already has an apex");

    let repo = self::repo(PROTON);
    to_per_zone(repo.path(), PROTON);
    write(repo.path(), &format!("{PER_ZONE}/schema.json"), "{}\n");
    assert_fails(repo.path(), "never its own");

    let repo = self::repo(PROTON);
    to_per_zone(repo.path(), PROTON);
    write(
        repo.path(),
        "infra/cloudflare/config.yaml",
        &CLOUDFLARE.replace("apex-noisypigeon", "other-noisypigeon"),
    );
    assert_fails(repo.path(), "needs a `apex-noisypigeon.com` token");
}

#[test]
fn moves_are_validated() {
    for (moves, want) in [
        (
            "{ from: infra/x, to: workloads/noisypigeon.com/apex }",
            "within infra/ or within workloads/",
        ),
        (
            "{ from: workloads/gone, to: workloads/nowhere }",
            "`to` must be a unit",
        ),
        (
            "{ from: workloads/noisypigeon.com/apex, to: workloads/noisypigeon.com/apex }",
            "`from` must no longer be a unit",
        ),
        (
            "{ from: workloads/gone, to: workloads/noisypigeon.com/apex }\n  - { from: workloads/gone, to: workloads/noisypigeon.com/apex }",
            "is in two moves",
        ),
    ] {
        let repo = repo(PROTON);
        to_per_zone(repo.path(), PROTON);
        write(
            repo.path(),
            ".config/moves.yaml",
            &format!("description: Test moves.\nmoves:\n  - {moves}\n"),
        );
        assert_fails(repo.path(), want);
    }
}

const MOVES: &str = "description: Test moves.\nmoves:\n  - { from: workloads/cloudflare/apex, to: workloads/noisypigeon.com/apex }\n";

#[test]
fn a_moved_apex_keeps_its_state_and_never_loses_a_retry() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = builtin_repo();
    let root = repo.path();
    let state = TempDir::new().expect("state");
    let all = cobot::select(root, &root.join(".config"), &[]).expect("valid");
    cobot::apply(&ctx(root, state.path()), &all).expect("apply");
    let from = state.path().join(APEX).join("terraform.tfstate");
    let to = state.path().join(PER_ZONE).join("terraform.tfstate");
    let original = fs::read(&from).expect("from state");

    to_per_zone(root, PROTON);
    write(root, ".config/moves.yaml", MOVES);
    let units = cobot::select(root, &root.join(".config"), &[]).expect("valid");
    let plans = cobot::plan(&ctx(root, state.path()), &units).expect("plan");
    let summary = cobot::summary(&plans);
    assert!(
        summary.contains("State moves from `workloads/cloudflare/apex`"),
        "{summary}"
    );
    assert!(plans.iter().all(|p| !p.has_changes()), "{summary}");

    let extra = PROTON.replace(
        "records: {}",
        "records:\n  extra: { name: extra, type: TXT, content: x }",
    );
    write(root, &format!("{PER_ZONE}/config.yaml"), &extra);
    let units = cobot::select(root, &root.join(".config"), &[]).expect("valid");
    let applied = cobot::apply(&ctx(root, state.path()), &units).expect("apply moved");
    assert_eq!(
        applied[1].counts(),
        (1, 0, 0),
        "{}",
        cobot::summary(&applied)
    );
    assert!(to.exists() && !from.exists());

    fs::write(&from, &original).expect("restore from");
    write(root, &format!("{PER_ZONE}/config.yaml"), PROTON);
    let units = cobot::select(root, &root.join(".config"), &[]).expect("valid");
    let retry = cobot::plan(&ctx(root, state.path()), &units).expect("retry");
    assert_eq!(retry[1].counts(), (0, 0, 1), "{}", cobot::summary(&retry));

    cobot::apply(&ctx(root, state.path()), &units).expect("apply retry");
    assert!(!from.exists());
    let again = cobot::plan(&ctx(root, state.path()), &units).expect("replan");
    assert!(
        again.iter().all(|p| !p.has_changes() && p.note.is_none()),
        "{}",
        cobot::summary(&again)
    );
}
