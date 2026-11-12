//! A config unit's blocks, their modules and their outputs, checked.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::super::render::{item_keys, lookup, rotation};
use super::super::{
    BLOCK, Block, CONFIG, ConfigUnit, Disk, Expr, MODULE_VERSION, Map, ModuleInfo, SCHEMA, blocks,
    declared_secrets, exprs_in, has_handoff, module_named_well, parse, x_cobot,
};
use crate::cobot::providers::Allowlist;

/// Checks one block's name and module; the module's info if it exists.
pub(crate) fn check_module(
    b: &Block,
    disk: &Disk,
    add: &mut impl FnMut(&str, String),
) -> Option<ModuleInfo> {
    if !BLOCK.is_match(&b.name) {
        add(
            SCHEMA,
            format!("block name `{}` must be lowercase with underscores", b.name),
        );
    }
    if !module_named_well(&b.module) {
        add(
            SCHEMA,
            format!(
                "block `{}` names module `{}`, which is not a lowercase-dash name under modules/ or <catalog>/",
                b.name, b.module
            ),
        );
        return None;
    }
    let m = disk.modules.get(&b.module).cloned().unwrap_or_default();
    if !m.inside {
        add(
            SCHEMA,
            format!(
                "block `{}` names missing module `{}`, looked for {}",
                b.name, b.module, m.display
            ),
        );
        return None;
    }
    if !MODULE_VERSION.is_match(&m.manifest) {
        add(
            &format!("{}/module.yaml", m.display),
            "needs `version: X.Y.Z`".into(),
        );
    }
    let provider_resources = m
        .info
        .resources
        .iter()
        .filter(|r| *r != "terraform_data")
        .count();
    let triggers = m.info.resources.len() - provider_resources;
    if provider_resources > 1 || triggers > 1 || m.info.resources.is_empty() {
        add(
            &m.display,
            "a module holds one resource, plus at most one terraform_data trigger".into(),
        );
    }
    Some(m.info)
}

/// Checks `x-cobot.each`: blocks over another array property's items.
pub(crate) fn check_each(schema: &Value, add: &mut impl FnMut(&str, String)) {
    let Some(each) = x_cobot(schema).get("each") else {
        return;
    };
    let Some(each) = each.as_object() else {
        add(SCHEMA, "`each` must be a map".into());
        return;
    };
    let x = x_cobot(schema);
    let taken = |name: &str| {
        schema["properties"].get(name).is_some()
            || x["singletons"].get(name).is_some()
            || x["derived"].get(name).is_some()
    };
    for (name, b) in each {
        if taken(name) {
            add(
                SCHEMA,
                format!("each block `{name}` reuses a property's or singleton's name"),
            );
        }
        let over = b["over"].as_str().unwrap_or_default();
        if schema["properties"][over]["type"] != "array" {
            add(
                SCHEMA,
                format!("each block `{name}`: `over` must name an array property"),
            );
        }
        if b.get("for_each").is_none() {
            add(SCHEMA, format!("each block `{name}` needs `for_each`"));
        }
        if b.get("rotate").is_some() {
            add(SCHEMA, format!("each block `{name}` cannot rotate"));
        }
    }
}

pub(crate) fn check_blocks(
    unit: &ConfigUnit,
    allow: &Allowlist,
    disk: &Disk,
    problems: &mut Vec<(String, String)>,
) {
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    check_each(&unit.schema, &mut add);
    let x = x_cobot(&unit.schema);
    let declared = declared_secrets(unit);
    let handoff = has_handoff(unit);
    let all = blocks(&unit.schema);
    let mut infos: BTreeMap<String, ModuleInfo> = BTreeMap::new();
    for b in &all {
        if let Some(info) = check_module(b, disk, &mut add) {
            infos.insert(b.name.clone(), info);
        }
    }
    for b in &all {
        let Some(info) = infos.get(&b.name) else {
            continue;
        };
        for (input, value) in &b.inputs {
            if !info.variables.contains(input) {
                add(
                    SCHEMA,
                    format!("block `{}` sets undeclared input `{input}`", b.name),
                );
            }
            check_exprs(
                value,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
        }
        if b.for_each.is_some()
            && let Err(e) = item_keys(b, &unit.config)
        {
            add(CONFIG, e);
        }
        if let Some((p, o)) = &b.rotate {
            check_exprs(
                p,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
            check_exprs(
                o,
                b,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
            if let Err(e) = rotation(b, &unit.config) {
                add(CONFIG, e);
            }
        }
    }
    if let Some(providers) = x["providers"].as_object() {
        for (name, cfg) in providers {
            if allow.by_name(name).is_none() {
                add(SCHEMA, format!("provider `{name}` is not in the allowlist"));
            }
            let pseudo = Block {
                name: format!("provider.{name}"),
                module: String::new(),
                over: String::new(),
                for_each: None,
                derived: None,
                rotate: None,
                inputs: Map::new(),
            };
            check_exprs(
                cfg,
                &pseudo,
                &all,
                &infos,
                &declared,
                handoff,
                &unit.config,
                &mut add,
            );
        }
    }
}

/// Checks `x-cobot.outputs`: each a `$module` expression.
pub(crate) fn check_outputs(unit: &ConfigUnit, disk: &Disk, problems: &mut Vec<(String, String)>) {
    let Some(outputs) = x_cobot(&unit.schema).get("outputs") else {
        return;
    };
    let Some(map) = outputs.as_object() else {
        problems.push((SCHEMA.into(), "`outputs` must be a map".into()));
        return;
    };
    let all = blocks(&unit.schema);
    let infos: BTreeMap<String, ModuleInfo> = all
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                disk.modules
                    .get(&b.module)
                    .map(|m| m.info.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();
    let pseudo = Block {
        name: "outputs".into(),
        module: String::new(),
        over: String::new(),
        for_each: None,
        derived: None,
        rotate: None,
        inputs: Map::new(),
    };
    let mut add = |file: &str, m: String| problems.push((file.to_string(), m));
    for (name, v) in map {
        if !BLOCK.is_match(name)
            || !v
                .as_str()
                .and_then(parse)
                .is_some_and(|e| matches!(e, Expr::Module { .. } | Expr::Secret(_)))
        {
            add(
                SCHEMA,
                format!("output `{name}` must be a `$module` or `$secret` expression"),
            );
            continue;
        }
        check_exprs(
            v,
            &pseudo,
            &all,
            &infos,
            &declared_secrets(unit),
            false,
            &unit.config,
            &mut add,
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one validation pass over shared context"
)]
pub(crate) fn check_exprs(
    value: &Value,
    block: &Block,
    all: &[Block],
    infos: &BTreeMap<String, ModuleInfo>,
    declared: &BTreeSet<String>,
    handoff: bool,
    config: &Value,
    add: &mut impl FnMut(&str, String),
) {
    for text in exprs_in(value) {
        match parse(text) {
            None => add(SCHEMA, format!("unknown expression `{text}`")),
            Some(Expr::Item(_) | Expr::Generation(_)) if !block.is_list() => {
                add(SCHEMA, format!("`{text}` needs a list block with for_each"));
            }
            Some(Expr::Generation(_)) if block.rotate.is_none() => {
                add(SCHEMA, format!("`{text}` needs a rotating block"));
            }
            Some(Expr::Secret(k)) if !declared.contains(&k) => {
                add(SCHEMA, format!("`{text}` is not in x-cobot.secrets"));
            }
            Some(Expr::Handoff(_)) if !handoff => {
                add(SCHEMA, format!("`{text}` needs `handoff`"));
            }
            Some(Expr::Config(p)) if lookup(config, &p).is_none() => {
                add(
                    SCHEMA,
                    format!("`{text}` names a path absent from config.yaml"),
                );
            }
            Some(Expr::Module {
                block: b,
                output,
                index,
            }) => match infos.get(&b) {
                _ if !all.iter().any(|x| x.name == b) => {
                    add(SCHEMA, format!("`{text}` names an unknown block"));
                }
                _ if all
                    .iter()
                    .any(|x| x.name == b && x.is_list() != index.is_some()) =>
                {
                    add(
                        SCHEMA,
                        format!(
                            "`{text}`: a list block needs `[*]` or `[$item.…]`, a singleton takes no index"
                        ),
                    );
                }
                Some(info) if !info.outputs.contains(&output) => {
                    add(SCHEMA, format!("`{text}`: module has no output `{output}`"));
                }
                _ => {}
            },
            Some(_) => {}
        }
    }
}
