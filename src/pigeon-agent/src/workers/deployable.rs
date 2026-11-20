//! Whether every Worker spec deploys, on this build and on main's stored build; see .claude/decisions/2026-11-19-development-workers-deployable-before-merge.md.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{
    DEPLOYMENT_PREFIX, DEPLOYMENT_VAR, Found, Item, SPEC, Spec, Tools, bound, discover, image_key,
    reference, references, render_with, scrub, stacks, text, validate,
};
use crate::infra::unit::Problem;

/// The stack the deploy binds every spec to, through `bound`'s colour fallback.
pub const DEPLOY_STACK: &str = "platform/blue";
/// What an unresolved output reference starts with.
pub const OUTPUT_PREFIX: &str = "$output.";
/// The variable holding the sha256 of main's stored build on a Checker.
pub const BUILD_VAR: &str = "PIGEON_BUILD_SHA256";
/// main's stored build, under `$TMPDIR`.
pub const BUILD_FILE: &str = "pigeon-build";
/// Where the dry runs write, three levels deep so the rendered `../../..` paths reach the root.
pub const DRY_DIR: &str = ".pigeon/deployable";
/// Why main's build failing a field is a rule, not a bug.
pub const PARSER_RULE: &str = "a spec field ships one change after its parser (.claude/decisions/2026-11-19-development-workers-deployable-before-merge.md §1)";

/// The outputs `found`'s references name, stubbed in the shape each asks for, and what is wrong.
#[must_use]
pub fn stub_outputs(
    root: &Path,
    found: &Found,
) -> (BTreeMap<String, Map<String, Value>>, Vec<String>) {
    let mut out: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut problems = Vec::new();
    let Some((manifest, _)) = crate::infra::manifest::load(root, root) else {
        return (out, problems);
    };
    let mut listed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut plain: BTreeSet<(String, String)> = BTreeSet::new();
    for r in references(&found.spec) {
        let Some(x) = reference(&r) else { continue };
        let declared = manifest.units.get(&x.unit).is_some_and(|e| {
            e.schema["x-pigeon"]["outputs"]
                .as_object()
                .is_some_and(|o| o.contains_key(&x.output))
                || (e.kind == "cloudflare/static-site" && x.output == "bucket")
        });
        if !declared {
            problems.push(format!(
                "`{r}`: `{}` declares no output `{}`",
                x.unit, x.output
            ));
            continue;
        }
        let key = (x.unit.clone(), x.output.clone());
        let slot = out.entry(x.unit.clone()).or_default();
        match x.index {
            None => {
                plain.insert(key.clone());
                let v = if x.output.ends_with("_id") {
                    json!("0".repeat(32))
                } else {
                    json!(format!("stub-{}", x.output.replace('_', "-")))
                };
                slot.insert(x.output.clone(), v);
            }
            Some(item) => {
                listed.insert(key.clone());
                let entry = slot
                    .entry(x.output.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Some(list) = entry.as_array_mut() {
                    match item {
                        Item::At(i) => {
                            while list.len() < i + 1 {
                                let k = list.len();
                                list.push(json!(format!(
                                    "stub-{}-{k}",
                                    x.output.replace('_', "-")
                                )));
                            }
                        }
                        Item::Named(n) => {
                            if !list.contains(&json!(n)) {
                                list.push(json!(n));
                            }
                        }
                    }
                }
            }
        }
    }
    for (unit, output) in listed.intersection(&plain) {
        problems.push(format!(
            "`{unit}.{output}` is used both as a list and as a value"
        ));
    }
    (out, problems)
}

/// Each container's image, a stub registry reference, keyed as `render_with` asks.
#[must_use]
pub fn stub_images(spec: &Spec) -> BTreeMap<String, String> {
    spec.containers
        .iter()
        .map(|c| {
            (
                image_key(spec, &c.class),
                format!(
                    "registry.cloudflare.com/stub-{}:stub",
                    c.class.to_lowercase()
                ),
            )
        })
        .collect()
}

/// Every string in `rendered` still holding a `$deployment.` or `$output.` reference.
#[must_use]
pub fn unresolved(rendered: &Value) -> Vec<String> {
    fn walk(v: &Value, pointer: &mut String, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                if s.contains(DEPLOYMENT_PREFIX) || s.contains(OUTPUT_PREFIX) {
                    let at = if pointer.is_empty() { "/" } else { pointer };
                    out.push(format!("{at}: `{s}` is left unresolved"));
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    let n = pointer.len();
                    let _ = write!(pointer, "/{i}");
                    walk(x, pointer, out);
                    pointer.truncate(n);
                }
            }
            Value::Object(o) => {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                for k in keys {
                    let n = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&k.replace('~', "~0").replace('/', "~1"));
                    walk(&o[k], pointer, out);
                    pointer.truncate(n);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(rendered, &mut String::new(), &mut out);
    out
}

/// The config a dry run deploys: `rendered` with a stub entry and no build, and the stub module.
#[must_use]
pub fn dry_config(rendered: &Value, spec: &Spec) -> (Value, String) {
    let mut config = rendered.clone();
    if let Some(o) = config.as_object_mut() {
        o.remove("build");
        o.remove("$schema");
        o.insert("main".into(), json!("stub.mjs"));
    }
    let classes: BTreeSet<&str> = spec
        .durable_objects
        .classes
        .iter()
        .map(String::as_str)
        .chain(spec.containers.iter().map(|c| c.class.as_str()))
        .collect();
    let mut module =
        String::from("export default { async fetch() { return new Response(null); } };\n");
    for c in classes {
        let _ = writeln!(module, "export class {c} {{}}");
    }
    (config, module)
}

/// Runs `wrangler deploy --dry-run` on `rendered` with a stub entry.
///
/// # Errors
///
/// The last lines of wrangler's output when it can't be run or fails.
pub fn dry_run(
    root: &Path,
    spec: &Spec,
    stack: &str,
    rendered: &Value,
    tools: &Tools,
) -> Result<(), String> {
    let dir = root
        .join(DRY_DIR)
        .join(format!("{}-{}", spec.name, stack.replace('/', "-")));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (config, module) = dry_config(rendered, spec);
    std::fs::write(dir.join("wrangler.jsonc"), text(&config)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("stub.mjs"), module).map_err(|e| e.to_string())?;
    let out = Command::new(&tools.wrangler)
        .args([
            "deploy",
            "-c",
            "wrangler.jsonc",
            "--dry-run",
            "--outdir",
            "dist",
        ])
        .current_dir(&dir)
        .envs(tools.env.iter().map(|(k, v)| (k, v)))
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let all = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let lines: Vec<&str> = all.lines().collect();
    let tail = &lines[lines.len().saturating_sub(20)..];
    Err(tail
        .iter()
        .map(|l| scrub(l, &[]))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Whether wrangler runs.
///
/// # Errors
///
/// A message when it doesn't and `on_checker`.
pub fn wrangler(tools: &Tools, on_checker: bool) -> Result<bool, String> {
    let ok = Command::new(&tools.wrangler)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if ok {
        Ok(true)
    } else if on_checker {
        Err("wrangler is needed on a Checker".into())
    } else {
        Ok(false)
    }
}

/// Binds every spec to every stack and the deploy's, renders with stubs and dry-runs when `tools`
/// is given; the number of spec and stack pairs checked.
///
/// # Errors
///
/// Every problem found.
pub fn head_pass(root: &Path, tools: Option<&Tools>) -> Result<usize, Vec<Problem>> {
    let found = discover(root).map_err(|e| {
        vec![Problem {
            path: SPEC.into(),
            message: e.to_string(),
        }]
    })?;
    let mut problems = Vec::new();
    let mut pairs = 0;
    for f in &found {
        let path = format!("{}/{SPEC}", f.dir);
        let mut local: Vec<String> = Vec::new();
        problems.extend(validate(root, f));
        let mut targets: Vec<(String, Found)> = Vec::new();
        if let Some(kind) = f.spec.deployment.as_deref() {
            let units: Vec<String> = stacks(root, kind)
                .into_iter()
                .chain([DEPLOY_STACK.to_string()])
                .collect();
            let mut seen: BTreeSet<(String, Option<String>)> = BTreeSet::new();
            for u in units {
                match bound(root, f, Some(&u)) {
                    Ok(b) => {
                        let key = (
                            b.spec.name.clone(),
                            b.spec.bindings.vars.get(DEPLOYMENT_VAR).cloned(),
                        );
                        if seen.insert(key) {
                            targets.push((u, b));
                        }
                    }
                    Err(e) => local.push(format!("[{u}] {e}")),
                }
            }
        } else {
            targets.push(("-".into(), f.clone()));
        }
        for (stack, b) in targets {
            pairs += 1;
            let mut mine: Vec<String> = Vec::new();
            let (outputs, stub_problems) = stub_outputs(root, &b);
            mine.extend(stub_problems);
            match render_with(&b, &outputs, Some(&stub_images(&b.spec))) {
                Ok(rendered) => {
                    mine.extend(unresolved(&rendered));
                    if mine.is_empty()
                        && let Some(t) = tools
                        && let Err(why) = dry_run(root, &b.spec, &stack, &rendered, t)
                    {
                        mine.push(format!("wrangler --dry-run: {why}"));
                    }
                }
                Err(e) => mine.extend(e),
            }
            local.extend(mine.into_iter().map(|m| format!("[{stack}] {m}")));
        }
        problems.extend(local.into_iter().map(|message| Problem {
            path: path.clone(),
            message,
        }));
    }
    if problems.is_empty() {
        Ok(pairs)
    } else {
        Err(problems)
    }
}

/// main's stored build: `given`, else `$TMPDIR/pigeon-build` when [`BUILD_VAR`] is set, else the
/// session's.
///
/// # Errors
///
/// A message when the stored build is missing or its sha256 isn't [`BUILD_VAR`].
pub fn main_build(
    var: impl Fn(&str) -> Option<String>,
    given: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    if given.is_some() {
        return Ok(given);
    }
    if let Some(sum) = var(BUILD_VAR) {
        let p = PathBuf::from(var("TMPDIR").unwrap_or_else(|| "/tmp".into())).join(BUILD_FILE);
        let bad = || {
            format!(
                "main's build `{}` is missing or isn't {BUILD_VAR}",
                p.display()
            )
        };
        let bytes = std::fs::read(&p).map_err(|_| bad())?;
        let hex = Sha256::digest(&bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
        return if hex == sum { Ok(Some(p)) } else { Err(bad()) };
    }
    Ok(var("HOME")
        .map(|h| PathBuf::from(h).join(".cache/pigeon/bin/pigeon"))
        .filter(|p| p.is_file()))
}

/// Runs main's build on `root`; the form it ran.
///
/// # Errors
///
/// A line each for why main's build can't deploy this tree.
pub fn main_pass(main: &Path, root: &Path) -> Result<&'static str, Vec<String>> {
    let has = Command::new(main)
        .args(["workers", "deployable", "--help"])
        .output()
        .is_ok_and(|o| o.status.success());
    let (form, mut cmd) = if has {
        let mut c = Command::new(main);
        c.args(["workers", "deployable", "--root"])
            .arg(root)
            .args(["--pass", "head"]);
        ("deployable", c)
    } else {
        let mut c = Command::new(main);
        c.args(["workers", "validate", "--root"])
            .arg(root)
            .args(["--deployment", DEPLOY_STACK]);
        ("validate", c)
    };
    // Main's build checks parse, bind and render only; the head pass runs the dry-run.
    cmd.env(super::WRANGLER_VAR, "/nonexistent/wrangler")
        .env_remove(BUILD_VAR);
    let out = cmd.output().map_err(|e| {
        vec![format!(
            "main's build `{}` can't deploy this tree ({form}); {e}",
            main.display()
        )]
    })?;
    if out.status.success() {
        return Ok(form);
    }
    let mut lines = vec![format!(
        "main's build `{}` can't deploy this tree ({form}); {PARSER_RULE}",
        main.display()
    )];
    for stream in [&out.stdout, &out.stderr] {
        for l in String::from_utf8_lossy(stream).lines() {
            if !l.trim().is_empty() {
                lines.push(scrub(l, &[]));
            }
        }
    }
    Err(lines)
}
