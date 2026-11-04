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
const APEX: &str = "workloads/noisypigeon.com/apex";
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

/// A repository with the real Cloudflare catalog and `body` as `noisypigeon.com`'s apex.
fn repo(body: &str) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    for rel in [
        ".config/providers.yaml",
        ".config/providers.lock.hcl",
        "catalog/cloudflare",
        "catalog/agnostic",
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
    write(root, &format!("{APEX}/config.yaml"), body);
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
    render_of(root, APEX)
}

fn render_of(root: &Path, unit: &str) -> Value {
    let units = cobot::select(root, &root.join(".config"), &[unit.to_string()]).expect("valid");
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
        "catalog/cloudflare/presets/other.yaml",
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
        &PROTON.replace("services:", "hosts: { blog: [journal] }\nservices:"),
    );
    let got = records(repo.path());
    assert!(rec(&got, "host-blog-journal").is_object() && rec(&got, "host-blog-www").is_null());

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &PROTON.replace("services:", "hosts: { blog: [a.b, a-b] }\nservices:"),
    );
    assert_fails(
        repo.path(),
        "`a.b` and `a-b` would share the record key `host-blog-a-b`",
    );

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &PROTON.replace("services:", "hosts: { ghost: [g] }\nservices:"),
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
                    "workloads/example.org/apex/config.yaml",
                    "services: {}\nrecords: {}\n",
                );
            }),
            "`example.org` is not a zone in infra/cloudflare",
        ),
        (
            "the old all-zones apex",
            Box::new(|r| write(r, "workloads/cloudflare/apex/config.yaml", "zones: {}\n")),
            "`cloudflare` is not a zone in infra/cloudflare",
        ),
        (
            "file in a zone",
            Box::new(|r| write(r, "workloads/noisypigeon.com/notes.md", "x\n")),
            "a zone holds only workload directories",
        ),
        (
            "apex without config",
            Box::new(|r| {
                fs::rename(
                    r.join(APEX).join("config.yaml"),
                    r.join(APEX).join("c.yaml"),
                )
                .expect("rename");
            }),
            "an apex holds config.yaml",
        ),
        (
            "apex with its own schema",
            Box::new(|r| write(r, &format!("{APEX}/schema.json"), "{}\n")),
            "never its own",
        ),
        (
            "apex with its own unit.yaml",
            Box::new(|r| write(r, &format!("{APEX}/unit.yaml"), "x: 1\n")),
            "never its own",
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
fn each_zone_is_its_own_apex_with_its_own_provider_and_keys() {
    let repo = repo(PROTON);
    write(
        repo.path(),
        "workloads/pigeon.dev/apex/config.yaml",
        "services: {}\nrecords:\n  site: { name: \"@\", type: A, content: 192.0.2.1 }\n",
    );
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let json = render_of(repo.path(), "workloads/pigeon.dev/apex");
    let provider = &json["provider"]["cloudflare"][0];
    assert_eq!(provider["alias"], "zone");
    assert_eq!(provider["for_each"], "${toset([\"pigeon.dev\"])}");
    assert_eq!(provider["api_token"], "${var.handoff[each.key].api_token}");
    let m = &json["module"]["dns_records"];
    assert_eq!(
        m["providers"]["cloudflare"],
        "cloudflare.zone[each.value.zone]"
    );
    assert_eq!(m["zone_id"], "${var.handoff[each.value.zone].zone_id}");
    assert_eq!(m["for_each"]["pigeon.dev:site"]["zone"], "pigeon.dev");
    assert_eq!(m["for_each"].as_object().expect("map").len(), 1);
    assert_eq!(records(repo.path()).len(), 9);
}

#[test]
fn optional_booleans_unset_on_false() {
    let repo = repo(&PROTON.replace("services:", "services:\n  flag: { enable: false }"));
    write(
        repo.path(),
        "catalog/cloudflare/presets/flag.yaml",
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
    write(
        root,
        "infra/cloudflare/presets/proton.yaml",
        &fs::read_to_string(repo_root().join("catalog/cloudflare/presets/proton.yaml"))
            .expect("preset"),
    );
    let real: Value = serde_json::from_str(
        &fs::read_to_string(repo_root().join("catalog/cloudflare/schema.json")).expect("schema"),
    )
    .expect("json");
    let mut apex_x = real["x-cobot"]["apex"].clone();
    apex_x["providers"] = json!({});
    let mut workload_x = real["x-cobot"]["workload"].clone();
    workload_x["providers"] = json!({});
    for (module, vars) in [
        ("ruleset", &["zone_id", "name", "phase", "rules"][..]),
        ("r2-bucket", &["account_id", "name", "storage_class"][..]),
        (
            "r2-custom-domain",
            &["account_id", "bucket_name", "domain", "zone_id"][..],
        ),
    ] {
        data_module(
            root,
            &format!("infra/cloudflare/modules/{module}"),
            vars,
            "output \"name\" {\n  value = terraform_data.this.id\n}\n",
        );
    }
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
            "workload": workload_x,
            "secrets": ["account_id"],
            "outputs": {
                "zone_ids": "$module.zones[*].id",
                "tokens": "$module.tokens[*].value",
                "token_ids": "$module.tokens[*].id",
                "account_id": "$secret.account_id"
            }
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
    secrets_file(
        &root.join("infra/cloudflare/secrets.config.yaml"),
        &["account_id"],
        |_| "v".into(),
        SECRETS_PASS,
    );
    data_module(root, "infra/cloudflare/modules/zone", &["name"], "");
    data_module(
        root,
        "infra/cloudflare/modules/token",
        &["name", "not_before"],
        "output \"value\" {\n  value     = \"tok-${var.name}-${var.not_before}\"\n  sensitive = true\n}\n",
    );
    write(root, &format!("{APEX}/config.yaml"), PROTON);
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
    let config = PROTON.replace(
        "records: {}",
        "records:\n  extra: { name: \"@\", type: TXT, content: { secret: extra_spf } }",
    );
    let repo = repo(&config);
    set_secrets(
        repo.path(),
        &["protonmail_verification", "pm_verification", "extra_spf"],
    );
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let parsed =
        json!({ "zones": { ZONE: serde_saphyr::from_str::<Value>(&config).expect("yaml") } });
    let site = cobot::apex::Site {
        root: repo.path().to_path_buf(),
        zone: ZONE.into(),
        workload: None,
    };
    let home = cobot::catalog::home(repo.path(), "cloudflare");
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
        write(
            repo.path(),
            ".config/moves.yaml",
            &format!("description: Test moves.\nmoves:\n  - {moves}\n"),
        );
        assert_fails(repo.path(), want);
    }
}

const OLD: &str = "workloads/noisypigeon.com/old";
const NEW: &str = "workloads/noisypigeon.com/new";

/// A workload with its own schema and one data module per item.
fn items_unit(root: &Path, rel: &str, items: &str) {
    write(
        root,
        &format!("{rel}/schema.json"),
        r#"{ "type": "object", "x-cobot": { "version": "0.1.0" }, "properties": { "items": { "type": "array", "x-cobot": { "module": "item", "for_each": "$item", "inputs": { "name": "$item" } } } } }"#,
    );
    data_module(root, &format!("{rel}/modules/item"), &["name"], "");
    write(
        root,
        &format!("{rel}/config.yaml"),
        &format!("items: [{items}]\n"),
    );
}

fn the(plans: &[cobot::UnitPlan], unit: &str) -> (usize, usize, usize) {
    plans
        .iter()
        .find(|p| p.unit == unit)
        .map(cobot::UnitPlan::counts)
        .expect("planned")
}

#[test]
fn a_moved_unit_keeps_its_state_and_never_loses_a_retry() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = builtin_repo();
    let root = repo.path();
    let state = TempDir::new().expect("state");
    items_unit(root, OLD, "a");
    let select =
        |unit: &str| cobot::select(root, &root.join(".config"), &[unit.into()]).expect("valid");
    cobot::apply(&ctx(root, state.path()), &select(OLD)).expect("apply");
    let from = state.path().join(OLD).join("terraform.tfstate");
    let to = state.path().join(NEW).join("terraform.tfstate");
    let original = fs::read(&from).expect("from state");

    fs::rename(root.join(OLD), root.join(NEW)).expect("move");
    write(
        root,
        ".config/moves.yaml",
        &format!("description: Test moves.\nmoves:\n  - {{ from: {OLD}, to: {NEW} }}\n"),
    );
    let plans = cobot::plan(&ctx(root, state.path()), &select(NEW)).expect("plan");
    let summary = cobot::summary(&plans);
    assert!(
        summary.contains(&format!("State moves from `{OLD}`")),
        "{summary}"
    );
    assert!(plans.iter().all(|p| !p.has_changes()), "{summary}");

    items_unit(root, NEW, "a, b");
    let applied = cobot::apply(&ctx(root, state.path()), &select(NEW)).expect("apply moved");
    assert_eq!(
        the(&applied, NEW),
        (1, 0, 0),
        "{}",
        cobot::summary(&applied)
    );
    assert!(to.exists() && !from.exists());

    fs::write(&from, &original).expect("restore from");
    items_unit(root, NEW, "a");
    let retry = cobot::plan(&ctx(root, state.path()), &select(NEW)).expect("retry");
    assert_eq!(the(&retry, NEW), (0, 0, 1), "{}", cobot::summary(&retry));

    cobot::apply(&ctx(root, state.path()), &select(NEW)).expect("apply retry");
    assert!(!from.exists());
    let again = cobot::plan(&ctx(root, state.path()), &select(NEW)).expect("replan");
    assert!(
        again.iter().all(|p| !p.has_changes() && p.note.is_none()),
        "{}",
        cobot::summary(&again)
    );
}

#[test]
fn secrets_under_the_wrong_passphrase_say_so_without_the_value() {
    let repo = builtin_repo();
    let root = repo.path();
    let path = root.join(APEX).join("secrets.config.yaml");
    secrets_file(
        &path,
        &["protonmail_verification", "pm_verification"],
        |k| format!("value-of-{k}"),
        SECRETS_PASS,
    );
    let state = TempDir::new().expect("state");
    let units = cobot::select(root, &root.join(".config"), &[APEX.into()]).expect("valid");
    let err = cobot::plan(&ctx(root, state.path()), &units)
        .expect_err("wrong passphrase")
        .to_string();
    assert!(
        err.starts_with(&format!(
            "{APEX}/secrets.config.yaml: `pm_verification` cannot be decrypted with COBOT_WORKLOAD_SECRETS_PASSPHRASE; it decrypts with COBOT_SECRETS_PASSPHRASE"
        )) && !err.contains("value-of-"),
        "{err}"
    );
}

const BLOG: &str = "workloads/noisypigeon.com/blog";

const SITE: &str = "static_site:\n  template: { version: 1.0.0, features: [math] }\n  site: { title: Blog, description: A blog. }\n";

/// `CLOUDFLARE` with the blog's site token and the apex token widened for its rulesets.
fn cloudflare_with_sites() -> String {
    CLOUDFLARE.replace(
        "      - { permission: DNS Write, zones: [noisypigeon.com] }\n",
        "      - { permission: DNS Write, zones: [noisypigeon.com] }
      - { permission: Zone Transform Rules Write, zones: [noisypigeon.com] }
      - { permission: Dynamic URL Redirects Write, zones: [noisypigeon.com] }
  - name: site-noisypigeon.com-blog
    capabilities:
      - { permission: Workers R2 Storage Write, account: true }
      - { permission: Zone Read, zones: [noisypigeon.com] }
      - { permission: DNS Write, zones: [noisypigeon.com] }
",
    )
}

/// A repository serving the blog as a static site at the apex's `@` and `www`.
fn site_repo() -> TempDir {
    let repo = repo(PROTON);
    let root = repo.path();
    write(
        root,
        "infra/cloudflare/config.yaml",
        &cloudflare_with_sites(),
    );
    write(root, &format!("{BLOG}/config.yaml"), SITE);
    repo
}

#[test]
fn a_static_site_renders_its_bucket_and_a_domain_per_host() {
    let repo = site_repo();
    assert_eq!(problems(repo.path()), Vec::<String>::new());
    let json = render_of(repo.path(), BLOG);
    assert_eq!(
        json["provider"]["cloudflare"]["api_token"],
        "${var.handoff.api_token}"
    );
    assert_eq!(
        json["variable"]["handoff"]["type"],
        "object({ api_token = string, zone_id = string, account_id = string })"
    );
    let bucket = &json["module"]["bucket"];
    assert_eq!(
        bucket["for_each"],
        json!({ "site": { "name": "blog-noisypigeon-com", "storage_class": "Standard" } })
    );
    assert_eq!(bucket["account_id"], "${var.handoff.account_id}");
    let domains = &json["module"]["domains"];
    assert_eq!(
        domains["for_each"],
        json!({
            "apex": { "bucket": "site", "domain": "noisypigeon.com" },
            "www": { "bucket": "site", "domain": "www.noisypigeon.com" }
        })
    );
    assert_eq!(
        domains["bucket_name"],
        "${module.bucket[each.value.bucket].name}"
    );
    assert_eq!(domains["zone_id"], "${var.handoff.zone_id}");

    write(
        repo.path(),
        &format!("{BLOG}/config.yaml"),
        &format!("{SITE}  storage_class: InfrequentAccess\n"),
    );
    assert_eq!(
        render_of(repo.path(), BLOG)["module"]["bucket"]["for_each"]["site"]["storage_class"],
        "InfrequentAccess"
    );
}

#[test]
fn the_apex_rewrites_and_redirects_for_its_static_sites_and_renders_no_records_for_them() {
    let repo = site_repo();
    let json = render(repo.path());
    let rulesets = &json["module"]["zone_rulesets"]["for_each"];
    let root = &rulesets["noisypigeon.com:sites-root"];
    assert_eq!(root["phase"], "http_request_transform");
    assert_eq!(root["zone"], ZONE);
    let rules = root["rules"].as_array().expect("rules");
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[1]["expression"],
        "(http.host eq \"www.noisypigeon.com\" and http.request.uri.path eq \"/\")"
    );
    assert_eq!(
        rules[0]["action_parameters"]["uri"]["path"]["value"],
        "/index.html"
    );
    let www = &rulesets["noisypigeon.com:sites-www"];
    assert_eq!(www["phase"], "http_request_dynamic_redirect");
    let redirect = &www["rules"][0];
    assert_eq!(
        redirect["expression"],
        "(http.host eq \"www.noisypigeon.com\")"
    );
    assert_eq!(
        redirect["action_parameters"]["from_value"]["status_code"],
        301
    );
    assert_eq!(
        redirect["action_parameters"]["from_value"]["target_url"]["expression"],
        "concat(\"https://noisypigeon.com\", http.request.uri.path)"
    );
    assert!(
        !records(repo.path())
            .keys()
            .any(|k| k.contains(":host-blog"))
    );

    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &PROTON.replace("services:", "hosts: { blog: [journal] }\nservices:"),
    );
    let rulesets = render(repo.path())["module"]["zone_rulesets"]["for_each"].clone();
    assert_eq!(
        rulesets
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["noisypigeon.com:sites-root".to_string()])
    );

    let repo = self::repo(PROTON);
    assert_eq!(
        render(repo.path())["module"]["zone_rulesets"]["for_each"],
        json!({})
    );
}

#[test]
fn a_static_site_needs_its_tokens() {
    let repo = site_repo();
    write(repo.path(), "infra/cloudflare/config.yaml", CLOUDFLARE);
    assert_fails(
        repo.path(),
        "infra/cloudflare/config.yaml needs a `site-noisypigeon.com-blog` token",
    );
    assert_fails(
        repo.path(),
        "`apex-noisypigeon.com` holds exactly `DNS Write` on `noisypigeon.com`, `Dynamic URL Redirects Write` on `noisypigeon.com`, `Zone Transform Rules Write` on `noisypigeon.com`, nothing else",
    );
    write(
        repo.path(),
        "infra/cloudflare/config.yaml",
        &cloudflare_with_sites().replace(
            "      - { permission: Zone Read, zones: [noisypigeon.com] }\n",
            "",
        ),
    );
    assert_fails(
        repo.path(),
        "`site-noisypigeon.com-blog` holds exactly `DNS Write` on `noisypigeon.com`, `Workers R2 Storage Write` on the account, `Zone Read` on `noisypigeon.com`, nothing else",
    );
}

#[test]
fn bad_static_sites_fail_for_their_own_reason() {
    for (config, want) in [
        ("static_site: {}\n", "\"template\" is a required property"),
        (
            "static_site:\n  template: { version: \"1.0\" }\n",
            "does not match",
        ),
        (
            "static_site:\n  template: { version: 1.0.0 }\n  storage_class: Cold\n",
            "storage_class",
        ),
        (
            "static_site:\n  template: { version: 1.0.0, features: [Math] }\n",
            "does not match",
        ),
        (
            "static_site:\n  template: { version: 1.0.0 }\n  extra: 1\n",
            "Additional properties",
        ),
        (
            "ingress: { type: CNAME, content: b.example }\n",
            "Additional properties",
        ),
        ("{}\n", "\"static_site\" is a required property"),
    ] {
        let repo = site_repo();
        write(repo.path(), &format!("{BLOG}/config.yaml"), config);
        assert_fails(repo.path(), want);
    }

    let repo = site_repo();
    write(
        repo.path(),
        &format!("{APEX}/config.yaml"),
        &PROTON.replace(
            "services:",
            "hosts: { blog: [blog.noisypigeon.com] }\nservices:",
        ),
    );
    assert_fails(repo.path(), "a static site needs a host");
    assert_fails(repo.path(), "assigns this workload no host");

    let repo = site_repo();
    write(
        repo.path(),
        "infra/cloudflare/config.yaml",
        &cloudflare_with_sites().replace(
            "zones: [noisypigeon.com, pigeon.dev]",
            "zones: [noisypigeon.com, pigeon.dev, noisy-pigeon.com, noisy.pigeon-com]",
        ),
    );
    for zone in ["noisy-pigeon.com", "noisy.pigeon-com"] {
        write(
            repo.path(),
            &format!("workloads/{zone}/blog/config.yaml"),
            SITE,
        );
    }
    assert_fails(
        repo.path(),
        "workloads/noisy.pigeon-com/blog: bucket `blog-noisy-pigeon-com` is also workloads/noisy-pigeon.com/blog's",
    );
    let long = "a".repeat(60);
    write(
        repo.path(),
        &format!("workloads/noisypigeon.com/{long}/config.yaml"),
        SITE,
    );
    assert_fails(repo.path(), "is longer than R2's 63 characters");

    let repo = site_repo();
    workload(
        repo.path(),
        "shop",
        &format!("ingress: {{ type: CNAME, content: s.example }}\n{SITE}"),
    );
    assert_fails(
        repo.path(),
        "workloads/noisypigeon.com/shop: ingress and static_site cannot both serve a workload",
    );
    write(
        repo.path(),
        "workloads/noisypigeon.com/shop/schema.json",
        r#"{ "type": "object", "x-cobot": { "version": "0.1.0", "workload": {} } }"#,
    );
    write(
        repo.path(),
        "workloads/noisypigeon.com/shop/config.yaml",
        "{}\n",
    );
    assert_fails(
        repo.path(),
        "`x-cobot.workload` belongs only in infra/cloudflare",
    );
}

#[test]
fn a_site_token_is_handed_off_after_infra_applies() {
    if !tofu_available() {
        eprintln!("skipped: tofu is not on PATH");
        return;
    }
    let repo = builtin_repo();
    let root = repo.path();
    write(
        root,
        "infra/cloudflare/config.yaml",
        &cloudflare_with_sites(),
    );
    write(root, &format!("{BLOG}/config.yaml"), SITE);
    let state = TempDir::new().expect("state");
    let blog = cobot::select(root, &root.join(".config"), &[BLOG.into()]).expect("valid");

    let pending = cobot::plan(&ctx(root, state.path()), &blog).expect("plan");
    let note = pending[0].note.clone().unwrap_or_default();
    assert!(note.contains("site-noisypigeon.com-blog"), "{note}");
    assert_eq!(pending[0].counts(), (3, 0, 0));
    assert!(matches!(
        cobot::apply(&ctx(root, state.path()), &blog),
        Err(cobot::Error::Pending { .. })
    ));

    let infra =
        cobot::select(root, &root.join(".config"), &["infra/cloudflare".into()]).expect("valid");
    cobot::apply(&ctx(root, state.path()), &infra).expect("apply infra");
    let applied = cobot::apply(&ctx(root, state.path()), &blog).expect("apply blog");
    assert!(applied[0].note.is_none(), "{}", cobot::summary(&applied));
    assert_eq!(applied[0].counts(), (3, 0, 0));
    let again = cobot::plan(&ctx(root, state.path()), &blog).expect("replan");
    assert!(!again[0].has_changes(), "{}", cobot::summary(&again));
    for dir in [root.join(".cobot"), state.path().to_path_buf()] {
        assert_eq!(
            grep(&dir, "tok-site-noisypigeon.com-blog"),
            Vec::<PathBuf>::new()
        );
    }
}

fn zola_available() -> bool {
    Command::new("zola")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn git_in(root: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_static_site_takes_its_template_features_and_needs_its_settings() {
    if !zola_available() {
        eprintln!("skipped: zola is not on PATH");
        return;
    }
    let repo = contract_repo();
    let root = repo.path();
    let sites = cobot::sites::find(root, &[]).expect("find");
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].hosts, ["noisypigeon.com", "www.noisypigeon.com"]);
    let built = cobot::sites::build(root, &sites).expect("build");
    assert!(built[0].features["math"] && built[0].features["feed"] && !built[0].features["tags"]);
    let home = fs::read_to_string(built[0].public.join("index.html")).expect("index");
    assert!(
        home.contains("https://noisypigeon.com/") && home.contains("katex.min.js"),
        "{home}"
    );

    write(
        root,
        &format!("{BLOG}/config.yaml"),
        &CONTRACT.replace("features: [math]", "features: [typo]"),
    );
    let sites = cobot::sites::find(root, &[]).expect("find");
    let err = cobot::sites::build(root, &sites)
        .expect_err("unknown")
        .to_string();
    assert!(
        err.contains("`typo` is not a feature of static-site-template/v1.0.0"),
        "{err}"
    );
    write(
        root,
        &format!("{BLOG}/config.yaml"),
        &CONTRACT.replace("1.0.0", "9.9.9"),
    );
    let sites = cobot::sites::find(root, &[]).expect("find");
    assert!(cobot::sites::build(root, &sites).is_err());

    write(
        root,
        &format!("{BLOG}/config.yaml"),
        "static_site:\n  template: { version: 1.0.0 }\n",
    );
    let sites = cobot::sites::find(root, &[]).expect("find");
    let err = cobot::sites::build(root, &sites)
        .expect_err("no settings")
        .to_string();
    assert!(err.contains("static_site.site is required"), "{err}");

    write(root, &format!("{BLOG}/config.yaml"), "static_site: [\n");
    assert!(matches!(
        cobot::sites::find(root, &[]),
        Err(cobot::Error::Invalid(_))
    ));
}

const CONTRACT: &str = "static_site:
  template: { version: 1.0.0, features: [math] }
  site:
    title: Blog
    short_name: blog
    description: A blog.
    intro: Hi, *you*.
    avatar: /assets/images/a.svg
    social_preview: /assets/images/a.svg
    author: { page: /pages/about }
    nav: [{ title: About, link: /pages/about }]
    theme_color: { light: \"#eee\", dark: \"#111\" }
";

/// A repo whose blog is a contract site: one post, one page, one asset.
fn contract_repo() -> TempDir {
    let repo = site_repo();
    let root = repo.path();
    copy(root, "src/static-site-template");
    write(root, &format!("{BLOG}/config.yaml"), CONTRACT);
    write(
        root,
        &format!("{BLOG}/site/posts/2015-02-15-hello.md"),
        "+++\ntitle = \"Hello\"\ndate = 2015-02-15T20:00:00+00:00\naliases = [\"changelog/hello\"]\n+++\nHello.\n",
    );
    write(
        root,
        &format!("{BLOG}/site/pages/about.md"),
        "+++\ntitle = \"About\"\n+++\nAbout.\n",
    );
    write(root, &format!("{BLOG}/site/assets/images/a.svg"), "<svg/>");
    git_in(root, &["init", "-q"]);
    git_in(root, &["add", "-A"]);
    git_in(root, &["commit", "-q", "-m", "init"]);
    git_in(root, &["tag", "static-site-template/v1.0.0"]);
    repo
}

#[test]
fn a_contract_site_builds_from_its_config_yaml() {
    if !zola_available() {
        eprintln!("skipped: zola is not on PATH");
        return;
    }
    let repo = contract_repo();
    let root = repo.path();
    let sites = cobot::sites::find(root, &[]).expect("find");
    let built = cobot::sites::build(root, &sites).expect("build");
    let public = &built[0].public;
    let read =
        |rel: &str| fs::read_to_string(public.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let toml = fs::read_to_string(public.with_file_name("config.toml")).expect("config");
    for want in [
        "base_url = \"https://noisypigeon.com\"",
        "title = \"Blog\"",
        "theme = \"static-site-template\"",
        "math = true",
    ] {
        assert!(toml.contains(want), "{want}: {toml}");
    }
    let manifest: Value = serde_json::from_str(&read("manifest.json")).expect("manifest");
    assert_eq!(
        (
            &manifest["name"],
            &manifest["short_name"],
            &manifest["theme_color"],
            &manifest["display"]
        ),
        (
            &json!("Blog"),
            &json!("blog"),
            &json!("#eee"),
            &json!("standalone")
        )
    );
    assert!(read("robots.txt").contains("Sitemap: https://noisypigeon.com/sitemap.xml"));
    let home = read("index.html");
    assert!(
        home.contains("Hi, <em>you</em>.") && home.contains("/posts/hello/"),
        "{home}"
    );
    assert!(read("posts/hello/index.html").contains("Hello."));
    assert!(read("changelog/hello/index.html").contains("/posts/hello/"));
    assert!(read("pages/about/index.html").contains("About."));
    assert_eq!(read("assets/images/a.svg"), "<svg/>");
    assert!(!public.join("posts/index.html").exists());
}

#[test]
fn a_contract_site_rejects_what_breaks_the_contract() {
    let repo = contract_repo();
    let root = repo.path();
    let build = |config: &str| {
        write(root, &format!("{BLOG}/config.yaml"), config);
        let sites = cobot::sites::find(root, &[]).expect("find");
        cobot::sites::build(root, &sites)
            .expect_err("rejected")
            .to_string()
    };

    write(
        root,
        &format!("{BLOG}/site/posts/0001-hello.md"),
        "+++\ndate = 2015-02-15\n+++\n",
    );
    let err = build(CONTRACT);
    assert!(err.contains("posts/0001-hello.md: is not named"), "{err}");
    fs::remove_file(root.join(BLOG).join("site/posts/0001-hello.md")).expect("rm");

    write(root, &format!("{BLOG}/site/config.toml"), "");
    let err = build(CONTRACT);
    assert!(
        err.contains("`config.toml` does not belong in site/"),
        "{err}"
    );
    fs::remove_file(root.join(BLOG).join("site/config.toml")).expect("rm");

    let err = build(&CONTRACT.replace("short_name: blog", "short_name: 7"));
    assert!(err.contains("static_site.site.short_name"), "{err}");

    fs::remove_file(root.join("src/static-site-template/schema.json")).expect("rm");
    git_in(root, &["commit", "-qam", "no contract"]);
    git_in(root, &["tag", "static-site-template/v0.9.0"]);
    let err = build(&CONTRACT.replace("1.0.0", "0.9.0"));
    assert!(err.contains("predates the config contract"), "{err}");
}
