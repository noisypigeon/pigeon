//! `x-cobot` expressions: `$item`, `$config`, `$secret`, `$generation` and `$module`.

use super::MODULE_REF;

/// One `x-cobot` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `$item` or `$item.<path>`.
    Item(Vec<String>),
    /// `$config.<path>`.
    Config(Vec<String>),
    /// `$secret.<key>`.
    Secret(String),
    /// `$generation.not_before` or `$generation.expires_on`.
    Generation(String),
    /// `$handoff.api_token` or `$handoff.zone_id`.
    Handoff(String),
    /// `$credential.<name>.<field>`, a field of a credential the runner supplies.
    Credential {
        /// The credential, named in `x-cobot.credentials`.
        name: String,
        /// Its field, such as `api_token`.
        field: String,
    },
    /// `$module.<block>.<out>`, `$module.<block>[*].<out>` or `$module.<block>[$item.<path>].<out>`.
    Module {
        /// The referenced block.
        block: String,
        /// `None`, `Some(None)` for `[*]`, or `Some(Some(path))` for `[$item.<path>]`.
        index: Option<Option<Vec<String>>>,
        /// The output.
        output: String,
    },
}

fn path(rest: &str) -> Option<Vec<String>> {
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let rest = rest.strip_prefix('.')?;
    let parts: Vec<String> = rest.split('.').map(str::to_string).collect();
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then_some(parts)
}

/// Parses an expression; `None` for an unknown form.
#[must_use]
pub fn parse(text: &str) -> Option<Expr> {
    if let Some(c) = MODULE_REF.captures(text) {
        let index = match c.name("index").map(|i| i.as_str()) {
            None => None,
            Some("*") => Some(None),
            Some(i) => Some(Some(
                path(i.trim_start_matches("$item")).filter(|p| !p.is_empty())?,
            )),
        };
        return Some(Expr::Module {
            block: c["block"].to_string(),
            index,
            output: c["out"].to_string(),
        });
    }
    if let Some(rest) = text.strip_prefix("$item") {
        return path(rest).map(Expr::Item);
    }
    if let Some(rest) = text.strip_prefix("$config") {
        return path(rest).filter(|p| !p.is_empty()).map(Expr::Config);
    }
    if let Some(key) = text.strip_prefix("$secret.") {
        return path(&format!(".{key}"))
            .filter(|p| p.len() == 1)
            .map(|p| Expr::Secret(p[0].clone()));
    }
    if let Some(rest) = text.strip_prefix("$credential") {
        return path(rest)
            .filter(|p| p.len() == 2)
            .map(|p| Expr::Credential {
                name: p[0].clone(),
                field: p[1].clone(),
            });
    }
    if let Some(key) = text.strip_prefix("$handoff.")
        && crate::cobot::static_site::HANDOFF_KEYS.contains(&key)
    {
        return Some(Expr::Handoff(key.to_string()));
    }
    match text {
        "$generation.not_before" => Some(Expr::Generation("not_before".into())),
        "$generation.expires_on" => Some(Expr::Generation("expires_on".into())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Expr, parse};

    #[test]
    fn expressions_parse() {
        assert_eq!(parse("$item"), Some(Expr::Item(vec![])));
        assert_eq!(parse("$item.name"), Some(Expr::Item(vec!["name".into()])));
        assert_eq!(
            parse("$config.rotation.period_days"),
            Some(Expr::Config(vec!["rotation".into(), "period_days".into()]))
        );
        assert_eq!(
            parse("$secret.account_id"),
            Some(Expr::Secret("account_id".into()))
        );
        assert_eq!(
            parse("$generation.expires_on"),
            Some(Expr::Generation("expires_on".into()))
        );
        assert_eq!(
            parse("$module.zones[*].id"),
            Some(Expr::Module {
                block: "zones".into(),
                index: Some(None),
                output: "id".into()
            })
        );
        assert_eq!(
            parse("$module.zones[$item.zone].id"),
            Some(Expr::Module {
                block: "zones".into(),
                index: Some(Some(vec!["zone".into()])),
                output: "id".into()
            })
        );
        assert_eq!(
            parse("$module.permission_groups.ids"),
            Some(Expr::Module {
                block: "permission_groups".into(),
                index: None,
                output: "ids".into()
            })
        );
        for bad in [
            "$items",
            "$config",
            "$secret.a.b",
            "$generation.x",
            "$module.Zones.id",
            "$module.z[x].id",
            "$env.HOME",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
