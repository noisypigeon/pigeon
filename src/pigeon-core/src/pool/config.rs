//! `.config/pool.yaml`: the pools, their prices and the task budgets.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::Micros;
use crate::sessions::Class;

/// The committed `.config/pool.yaml`.
pub const CONFIG: &str = include_str!("../../../../.config/pool.yaml");
/// The gateways a pool may name.
pub const GATEWAYS: [&str; 1] = ["anthropic-messages"];
/// The harnesses a pool may name.
pub const HARNESSES: [&str; 1] = ["claude-code"];

/// Which capacity a task takes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Prefer {
    /// The subscription's slots.
    Subscription,
    /// The pools.
    Pool,
}

/// Micro-dollars per million tokens, by kind of token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    /// Input tokens.
    pub input: Micros,
    /// Cache writes, 5 minute.
    pub write_5m: Micros,
    /// Cache writes, 1 hour.
    pub write_1h: Micros,
    /// Cache reads.
    pub read: Micros,
    /// Output tokens.
    pub output: Micros,
}

/// A price that applies above some prompt size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Long {
    /// The prompt tokens it applies above.
    pub over: u64,
    /// The price.
    pub price: Price,
}

/// What a task of a class may spend by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Dollars, in micro-dollars.
    pub usd: Micros,
    /// Fresh tokens.
    pub fresh_tokens: u64,
    /// Requests.
    pub requests: u32,
    /// Minutes.
    pub minutes: u32,
}

/// The most a task of a class may be given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Max {
    /// Dollars, in micro-dollars.
    pub usd: Micros,
    /// Minutes.
    pub minutes: u32,
}

/// One capacity pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pool {
    /// Its slug.
    pub name: String,
    /// Whether tasks may route to it.
    pub enabled: bool,
    /// Its gateway.
    pub gateway: String,
    /// Its harness.
    pub harness: String,
    /// Its credit.
    pub credit: Micros,
    /// Its monthly limit.
    pub month_limit: Micros,
    /// The day its cycle starts, if known.
    pub cycle_day: Option<u64>,
    /// Its concurrent tasks.
    pub concurrency: u32,
    /// Its prices, by model.
    pub prices: BTreeMap<String, Price>,
    /// Its long-prompt prices, by model.
    pub long: BTreeMap<String, Long>,
}

/// `.config/pool.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Pools, first first.
    pub order: Vec<String>,
    /// What a task takes first.
    pub prefer: Prefer,
    /// The subscription's slots.
    pub subscription_slots: u32,
    /// Days a pool drains before its cycle ends.
    pub drain_days: u32,
    /// The pools, by name.
    pub pools: BTreeMap<String, Pool>,
    /// Default budgets, by class name.
    pub budgets: BTreeMap<String, Budget>,
    /// Maxima, by class name.
    pub maxima: BTreeMap<String, Max>,
}

fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn list<'a>(key: &str, value: &'a str, n: Option<usize>) -> Result<Vec<&'a str>, String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(|| format!("`{key}` is not a [list]"))?;
    let items: Vec<&str> = inner.split(',').map(str::trim).collect();
    if n.is_some_and(|n| items.len() != n) {
        return Err(format!("`{key}` needs exactly {} items", n.unwrap_or(0)));
    }
    Ok(items)
}

fn whole(key: &str, value: &str, min: u64, max: u64) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("`{key}` is not a whole number from {min} to {max}"))
}

fn usd(key: &str, value: &str) -> Result<Micros, String> {
    super::micros(value)
        .filter(|m| *m > 0)
        .ok_or_else(|| format!("`{key}` is not dollars above 0"))
}

fn price(key: &str, items: &[&str]) -> Result<Price, String> {
    let m: Vec<Micros> = items
        .iter()
        .map(|i| usd(key, i))
        .collect::<Result<_, _>>()?;
    Ok(Price {
        input: m[0],
        write_5m: m[1],
        write_1h: m[2],
        read: m[3],
        output: m[4],
    })
}

fn small(key: &str, value: &str, min: u64) -> Result<u32, String> {
    let n = whole(key, value, min, u64::MAX)?;
    u32::try_from(n).map_err(|_| format!("`{key}` is too large"))
}

#[derive(Default)]
struct Raw {
    scalars: BTreeMap<String, String>,
    prices: BTreeMap<String, Price>,
    long: BTreeMap<String, Long>,
}

/// `yaml` as a config.
///
/// # Errors
///
/// When a key is unknown, missing or repeated, or a value is not what it should be.
#[allow(clippy::too_many_lines)]
pub fn parse(yaml: &str) -> Result<Config, String> {
    let mut seen = BTreeSet::new();
    let mut order: Option<Vec<String>> = None;
    let mut prefer = None;
    let mut slots = None;
    let mut drain = None;
    let mut pools: BTreeMap<String, Raw> = BTreeMap::new();
    let mut budgets = BTreeMap::new();
    let mut maxima = BTreeMap::new();
    for line in yaml.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .map(|(k, v)| (k.trim(), v.trim()))
            .ok_or_else(|| format!("`{line}` is not `key: value`"))?;
        if !seen.insert(key.to_string()) {
            return Err(format!("`{key}` is repeated"));
        }
        match key {
            "order" => {
                let items = list(key, value, None)?;
                let mut names = Vec::new();
                for i in items {
                    if !slug(i) {
                        return Err(format!("`order` has `{i}`, which is not a slug"));
                    }
                    if names.iter().any(|n| n == i) {
                        return Err(format!("`{i}` is in `order` twice"));
                    }
                    names.push(i.to_string());
                }
                order = Some(names);
            }
            "prefer" => {
                prefer = Some(match value {
                    "subscription" => Prefer::Subscription,
                    "pool" => Prefer::Pool,
                    _ => return Err(format!("`{key}` is not subscription or pool")),
                });
            }
            "subscription_slots" => slots = Some(small(key, value, 1)?),
            "drain_days" => drain = Some(whole(key, value, 0, 31)?),
            _ => {
                let (head, rest) = key
                    .split_once('.')
                    .ok_or_else(|| format!("unknown key `{key}`"))?;
                let class = |c: &str| match c {
                    "haiku" | "sonnet" => Ok(c.to_string()),
                    _ => Err(format!("unknown key `{key}`")),
                };
                if head == "budget" {
                    let c = class(rest)?;
                    let items = list(key, value, Some(4))?;
                    let b = Budget {
                        usd: usd(key, items[0])?,
                        fresh_tokens: whole(key, items[1], 1, u64::MAX)?,
                        requests: small(key, items[2], 1)?,
                        minutes: small(key, items[3], 1)?,
                    };
                    budgets.insert(c, b);
                } else if head == "max" {
                    let c = class(rest)?;
                    let items = list(key, value, Some(2))?;
                    let m = Max {
                        usd: usd(key, items[0])?,
                        minutes: small(key, items[1], 1)?,
                    };
                    maxima.insert(c, m);
                } else {
                    let raw = pools.entry(head.to_string()).or_default();
                    if let Some(model) = rest.strip_prefix("price.") {
                        let items = list(key, value, Some(5))?;
                        raw.prices.insert(model.to_string(), price(key, &items)?);
                    } else if let Some(model) = rest.strip_prefix("long.") {
                        let items = list(key, value, Some(6))?;
                        let over = whole(key, items[0], 1, u64::MAX)?;
                        raw.long.insert(
                            model.to_string(),
                            Long {
                                over,
                                price: price(key, &items[1..])?,
                            },
                        );
                    } else if POOL_KEYS.contains(&rest) {
                        raw.scalars.insert(rest.to_string(), value.to_string());
                    } else {
                        return Err(format!("unknown key `{key}`"));
                    }
                }
            }
        }
    }
    let order = order
        .filter(|o| !o.is_empty())
        .ok_or("`order` is missing or empty")?;
    let prefer = prefer.ok_or("`prefer` is missing")?;
    let subscription_slots = slots.ok_or("`subscription_slots` is missing")?;
    let drain_days = u32::try_from(drain.ok_or("`drain_days` is missing")?)
        .map_err(|_| "`drain_days` is too large".to_string())?;
    if let Some(p) = pools.keys().find(|p| !order.contains(p)) {
        return Err(format!("`{p}` is not in `order`"));
    }
    let mut out = BTreeMap::new();
    for name in &order {
        let raw = pools.remove(name).unwrap_or_default();
        out.insert(name.clone(), pool(name, raw)?);
    }
    for class in ["haiku", "sonnet"] {
        let b = budgets
            .get(class)
            .ok_or_else(|| format!("`budget.{class}` is missing"))?;
        let m = maxima
            .get(class)
            .ok_or_else(|| format!("`max.{class}` is missing"))?;
        if b.usd > m.usd || b.minutes > m.minutes {
            return Err(format!("`budget.{class}` exceeds `max.{class}`"));
        }
    }
    Ok(Config {
        order,
        prefer,
        subscription_slots,
        drain_days,
        pools: out,
        budgets,
        maxima,
    })
}

const POOL_KEYS: [&str; 7] = [
    "enabled",
    "gateway",
    "harness",
    "credit_usd",
    "month_limit_usd",
    "cycle_day",
    "concurrency",
];

fn pool(name: &str, raw: Raw) -> Result<Pool, String> {
    let get = |k: &str| {
        raw.scalars
            .get(k)
            .map(String::as_str)
            .ok_or_else(|| format!("`{name}.{k}` is missing"))
    };
    let key = |k: &str| format!("{name}.{k}");
    let enabled = match get("enabled")? {
        "true" => true,
        "false" => false,
        _ => return Err(format!("`{}` is not true or false", key("enabled"))),
    };
    let gateway = get("gateway")?;
    if !GATEWAYS.contains(&gateway) {
        return Err(format!("`{}` is not a known gateway", key("gateway")));
    }
    let harness = get("harness")?;
    if !HARNESSES.contains(&harness) {
        return Err(format!("`{}` is not a known harness", key("harness")));
    }
    let credit = usd(&key("credit_usd"), get("credit_usd")?)?;
    let month_limit = usd(&key("month_limit_usd"), get("month_limit_usd")?)?;
    let cycle = get("cycle_day")?;
    let cycle_day = if cycle == "null" {
        None
    } else {
        Some(whole(&key("cycle_day"), cycle, 1, 31)?)
    };
    let concurrency = small(&key("concurrency"), get("concurrency")?, 1)?;
    if concurrency > 8 {
        return Err(format!("`{}` is not from 1 to 8", key("concurrency")));
    }
    if raw.prices.is_empty() {
        return Err(format!("`{name}` has no `price`"));
    }
    if let Some(m) = raw.long.keys().find(|m| !raw.prices.contains_key(*m)) {
        return Err(format!("`{name}.long.{m}` has no `{name}.price.{m}`"));
    }
    if month_limit > credit {
        return Err(format!(
            "`{}` is above `{}`",
            key("month_limit_usd"),
            key("credit_usd")
        ));
    }
    if enabled && cycle_day.is_none() {
        return Err(format!("`{}` is null on an enabled pool", key("cycle_day")));
    }
    Ok(Pool {
        name: name.to_string(),
        enabled,
        gateway: gateway.to_string(),
        harness: harness.to_string(),
        credit,
        month_limit,
        cycle_day,
        concurrency,
        prices: raw.prices,
        long: raw.long,
    })
}

/// The committed config.
///
/// # Errors
///
/// When `.config/pool.yaml` does not parse.
pub fn config() -> Result<Config, String> {
    parse(CONFIG)
}

impl Config {
    /// The pool called `name`.
    #[must_use]
    pub fn pool(&self, name: &str) -> Option<&Pool> {
        self.pools.get(name)
    }

    /// `class`'s budget, with `usd` and `minutes` in place of its defaults.
    ///
    /// # Errors
    ///
    /// When `class` is opus, or an override is above its class's maximum.
    pub fn budget(
        &self,
        class: Class,
        usd: Option<Micros>,
        minutes: Option<u32>,
    ) -> Result<Budget, String> {
        if class == Class::Opus {
            return Err("opus never runs on a pool".into());
        }
        let name = class.name();
        let mut b = *self
            .budgets
            .get(name)
            .ok_or_else(|| format!("no budget for `{name}`"))?;
        let max = self
            .maxima
            .get(name)
            .ok_or_else(|| format!("no max for `{name}`"))?;
        if let Some(u) = usd {
            if u > max.usd {
                return Err(format!(
                    "${} is above {name}'s maximum of ${}",
                    super::dollars(u),
                    super::dollars(max.usd)
                ));
            }
            b.usd = u;
        }
        if let Some(m) = minutes {
            if m > max.minutes {
                return Err(format!(
                    "{m} minutes is above {name}'s maximum of {}",
                    max.minutes
                ));
            }
            b.minutes = m;
        }
        Ok(b)
    }
}

impl Pool {
    /// The price of `model` for a prompt of `prompt_tokens`.
    #[must_use]
    pub fn price(&self, model: &str, prompt_tokens: u64) -> Option<Price> {
        match self.long.get(model) {
            Some(l) if prompt_tokens > l.over => Some(l.price),
            _ => self.prices.get(model).copied(),
        }
    }

    /// The highest of each kind of price over every model.
    #[must_use]
    pub fn priciest(&self) -> Option<Price> {
        let all = self
            .prices
            .values()
            .copied()
            .chain(self.long.values().map(|l| l.price));
        all.reduce(|a, b| Price {
            input: a.input.max(b.input),
            write_5m: a.write_5m.max(b.write_5m),
            write_1h: a.write_1h.max(b.write_1h),
            read: a.read.max(b.read),
            output: a.output.max(b.output),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(edit: impl Fn(&str) -> String) -> Result<Config, String> {
        parse(&edit(CONFIG))
    }

    fn refuses(edit: impl Fn(&str) -> String, needle: &str) {
        let e = with(edit).expect_err(needle);
        assert!(e.contains(needle), "{e} lacks {needle}");
    }

    #[test]
    fn committed_config_parses() {
        let c = config().unwrap();
        let p = c.pool("anthropic").unwrap();
        assert!(!p.enabled);
        assert_eq!(p.credit, 100_000_000);
        assert_eq!(p.month_limit, 50_000_000);
        assert_eq!(p.prices["claude-sonnet-5-5"].output, 10_000_000);
        assert_eq!(p.long["claude-haiku-5-5"].over, 100_000);
        assert_eq!(c.prefer, Prefer::Subscription);
        assert_eq!(c.subscription_slots, 3);
        assert_eq!(c.drain_days, 7);
        assert_eq!(c.budgets["haiku"].usd, 750_000);
        assert_eq!(c.maxima["sonnet"].minutes, 60);
        assert!(c.pool("nope").is_none());
    }

    macro_rules! refusals {
        ($($name:ident: $edit:expr => $needle:expr;)*) => {
            $(#[test]
            fn $name() {
                refuses($edit, $needle);
            })*
        };
    }

    refusals! {
        line_without_colon: |c| format!("{c}nonsense\n") => "nonsense";
        unknown_top_key: |c| format!("{c}wat: 1\n") => "`wat`";
        unknown_pool_key: |c| format!("{c}anthropic.wat: 1\n") => "`anthropic.wat`";
        unknown_class: |c| format!("{c}budget.opus: [1, 1, 1, 1]\n") => "`budget.opus`";
        repeated_key: |c| format!("{c}drain_days: 3\n") => "`drain_days` is repeated";
        bad_prefer: |c| c.replace("prefer: subscription", "prefer: both") => "`prefer`";
        bad_slots: |c| c.replace("subscription_slots: 3", "subscription_slots: 0") => "`subscription_slots`";
        bad_drain: |c| c.replace("drain_days: 7", "drain_days: 32") => "`drain_days`";
        bad_enabled: |c| c.replace("anthropic.enabled: false", "anthropic.enabled: no") => "`anthropic.enabled`";
        bad_gateway: |c| c.replace("anthropic-messages", "other") => "`anthropic.gateway`";
        bad_harness: |c| c.replace("claude-code", "other") => "`anthropic.harness`";
        bad_credit: |c| c.replace("credit_usd: 100", "credit_usd: 0") => "`anthropic.credit_usd`";
        bad_concurrency: |c| c.replace("concurrency: 2", "concurrency: 9") => "`anthropic.concurrency`";
        bad_cycle_day: |c| c.replace("cycle_day: null", "cycle_day: 32") => "`anthropic.cycle_day`";
        bad_price_list: |c| c.replace("[2, 2.5, 4, 0.1, 10]", "[2, 2.5, 4, 0.1]") => "`anthropic.price.claude-sonnet-5-5`";
        bad_long_list: |c| c.replace("[100000, 0.5,", "[0, 0.5,") => "`anthropic.long.claude-haiku-5-5`";
        bad_budget_list: |c| c.replace("[0.75, 3000000, 120, 30]", "[0.75, 3000000, 120]") => "`budget.haiku`";
        bad_max_list: |c| c.replace("[2, 45]", "[2]") => "`max.haiku`";
        bad_order_slug: |c| c.replace("order: [anthropic]", "order: [Anthropic]") => "`order`";
        order_twice: |c| c.replace("order: [anthropic]", "order: [anthropic, anthropic]") => "twice";
        missing_order: |c| c.replace("order: [anthropic]\n", "") => "`order`";
        empty_order: |c| c.replace("order: [anthropic]", "order: []") => "`order`";
        missing_prefer: |c| c.replace("prefer: subscription\n", "") => "`prefer`";
        missing_slots: |c| c.replace("subscription_slots: 3\n", "") => "`subscription_slots`";
        missing_drain: |c| c.replace("drain_days: 7\n", "") => "`drain_days`";
        missing_pool_key: |c| c.replace("anthropic.concurrency: 2\n", "") => "`anthropic.concurrency`";
        missing_budget: |c| c.replace("budget.sonnet: [4, 3000000, 200, 40]\n", "") => "`budget.sonnet`";
        missing_max: |c| c.replace("max.haiku: [2, 45]\n", "") => "`max.haiku`";
        pool_not_in_order: |c| format!("{c}other.enabled: true\n") => "`other` is not in `order`";
        pool_without_price: |c| c.lines().filter(|l| !l.contains(".price.")).collect::<Vec<_>>().join("\n") => "no `price`";
        long_without_price: |c| format!("{c}anthropic.long.claude-opus-9: [1, 1, 1, 1, 1, 1]\n") => "`anthropic.long.claude-opus-9`";
        month_limit_above_credit: |c| c.replace("month_limit_usd: 50", "month_limit_usd: 150") => "`anthropic.month_limit_usd`";
        enabled_without_cycle_day: |c| c.replace("enabled: false", "enabled: true") => "`anthropic.cycle_day`";
        budget_dollars_over_max: |c| c.replace("[0.75, 3000000, 120, 30]", "[3, 3000000, 120, 30]") => "`budget.haiku`";
        budget_minutes_over_max: |c| c.replace("[0.75, 3000000, 120, 30]", "[0.75, 3000000, 120, 46]") => "`budget.haiku`";
    }

    #[test]
    fn enabled_with_cycle_day_parses() {
        let c = with(|c| {
            c.replace("enabled: false", "enabled: true")
                .replace("cycle_day: null", "cycle_day: 5")
        })
        .unwrap();
        assert_eq!(c.pools["anthropic"].cycle_day, Some(5));
    }

    #[test]
    fn budget_for_classes() {
        let c = config().unwrap();
        assert_eq!(
            c.budget(Class::Opus, None, None).unwrap_err(),
            "opus never runs on a pool"
        );
        let b = c.budget(Class::Haiku, None, None).unwrap();
        assert_eq!((b.usd, b.minutes, b.requests), (750_000, 30, 120));
        assert_eq!(
            c.budget(Class::Haiku, Some(1_000_000), None).unwrap().usd,
            1_000_000
        );
        let e = c.budget(Class::Haiku, Some(3_000_000), None).unwrap_err();
        assert!(e.contains("3.00") && e.contains("2.00"), "{e}");
        assert_eq!(c.budget(Class::Sonnet, None, Some(60)).unwrap().minutes, 60);
        assert!(c.budget(Class::Sonnet, None, Some(61)).is_err());
    }

    #[test]
    fn price_by_prompt_size() {
        let p = config().unwrap().pools["anthropic"].clone();
        assert_eq!(p.price("claude-haiku-5-5", 100_000).unwrap().input, 100_000);
        assert_eq!(p.price("claude-haiku-5-5", 100_001).unwrap().input, 500_000);
        assert_eq!(
            p.price("claude-sonnet-5-5", 1_000_000).unwrap().input,
            2_000_000
        );
        assert!(p.price("claude-unknown", 1).is_none());
    }

    #[test]
    fn priciest_of_committed() {
        let p = config().unwrap().pools["anthropic"].clone();
        assert_eq!(
            p.priciest().unwrap(),
            Price {
                input: 2_000_000,
                write_5m: 2_500_000,
                write_1h: 4_000_000,
                read: 100_000,
                output: 10_000_000,
            }
        );
        let mut empty = p;
        empty.prices.clear();
        empty.long.clear();
        assert!(empty.priciest().is_none());
    }
}
