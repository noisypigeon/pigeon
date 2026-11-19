//! Zone import: a zone's live records, from a BIND file or the API, as keyed `records:` YAML;
//! see `.claude/decisions/2026-11-15-runtime-zoned-standalone-account-imports.md` §5.

use core::net::Ipv6Addr;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::infra::apex;

const CAA_TAGS: [&str; 3] = ["issue", "issuewild", "iodef"];

/// One record read from a zone, with its name relative to the zone.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The relative name: `@`, `www`, `*`, `*.dev`.
    pub name: String,
    /// The record type, upper case.
    pub kind: String,
    /// The content, for every type but SRV and CAA.
    pub content: Option<String>,
    /// The structured data, for SRV and CAA.
    pub data: Option<Value>,
    /// The TTL in seconds; 1 is automatic.
    pub ttl: i64,
    /// The priority, for MX.
    pub priority: Option<i64>,
    /// Whether Cloudflare proxies the record.
    pub proxied: bool,
    /// The record's comment.
    pub comment: Option<String>,
}

/// A record the import leaves out because no unit can manage it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmanaged {
    /// The record type, upper case.
    pub kind: String,
    /// The relative name, when known.
    pub name: Option<String>,
}

impl fmt::Display for Unmanaged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "record {} at {name}", self.kind),
            None => write!(f, "{}", self.kind),
        }
    }
}

/// A record the units already render.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// The fully qualified name, lower case.
    pub name: String,
    /// The record type.
    pub kind: String,
    /// The content, with no template escapes.
    pub content: Option<String>,
    /// The structured data.
    pub data: Option<Value>,
}

/// Where the records come from.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    /// A BIND zone file's text.
    Bind(&'a str),
    /// The `result` of the API's `dns_records`.
    Api(&'a [Value]),
}

/// The import: the YAML and what it left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The `records:` document.
    pub yaml: String,
    /// How many records it holds.
    pub records: usize,
    /// The records no unit can manage.
    pub unmanaged: Vec<Unmanaged>,
}

type Parsed = (Vec<Imported>, Vec<Unmanaged>);

/// The zone a BIND file names: its first `$ORIGIN`, else its first SOA's absolute owner.
#[must_use]
pub fn origin_of(text: &str) -> Option<String> {
    let lines: Vec<Vec<&str>> = text
        .lines()
        .map(|l| {
            l.split(';')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect()
        })
        .collect();
    let dot = |s: &str| s.trim_end_matches('.').to_ascii_lowercase();
    for t in &lines {
        if t.len() >= 2 && t[0].eq_ignore_ascii_case("$ORIGIN") {
            return Some(dot(t[1]));
        }
    }
    let soa = lines
        .iter()
        .find(|t| t.iter().skip(1).any(|w| w.eq_ignore_ascii_case("SOA")))?;
    soa[0].ends_with('.').then(|| dot(soa[0]))
}

struct Logical {
    line: usize,
    cont: bool,
    tokens: Vec<(String, bool)>,
    comment: String,
}

fn logical_records(text: &str) -> Result<Vec<Logical>, String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur: Option<Logical> = None;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let rec = cur.get_or_insert_with(|| Logical {
            line: n,
            cont: raw.starts_with([' ', '\t']),
            tokens: Vec::new(),
            comment: String::new(),
        });
        let mut tok = String::new();
        let mut live = false;
        let mut quoted = false;
        let mut in_quote = false;
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if in_quote {
                match c {
                    '"' => in_quote = false,
                    '\\' => match chars.peek().copied() {
                        Some(d) if d.is_ascii_digit() => {
                            let digits: String = (0..3)
                                .map_while(|_| chars.next_if(char::is_ascii_digit))
                                .collect();
                            let byte = digits.parse::<u32>().unwrap_or(0);
                            tok.push(char::from(u8::try_from(byte).unwrap_or(0)));
                        }
                        Some(d) => {
                            tok.push(d);
                            chars.next();
                        }
                        None => tok.push('\\'),
                    },
                    _ => tok.push(c),
                }
                continue;
            }
            match c {
                '"' => {
                    in_quote = true;
                    live = true;
                    quoted = true;
                }
                ';' => {
                    rec.comment = chars.by_ref().collect();
                    break;
                }
                '(' | ')' | ' ' | '\t' | '\r' => {
                    if live {
                        rec.tokens.push((std::mem::take(&mut tok), quoted));
                        live = false;
                        quoted = false;
                    }
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    if depth < 0 {
                        return Err("unbalanced parentheses".into());
                    }
                }
                _ => {
                    tok.push(c);
                    live = true;
                }
            }
        }
        if in_quote {
            return Err(format!("line {n}: unterminated quote"));
        }
        if live {
            rec.tokens.push((tok, quoted));
        }
        if depth == 0 {
            out.extend(cur.take().filter(|d| !d.tokens.is_empty()));
        }
    }
    if depth != 0 {
        return Err("unbalanced parentheses".into());
    }
    Ok(out)
}

fn parse_ttl(s: &str) -> Option<i64> {
    let mut total = 0i64;
    let mut num: Option<i64> = None;
    for c in s.chars() {
        if let Some(d) = c.to_digit(10) {
            num = Some(
                num.unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(i64::from(d)),
            );
        } else {
            let unit = match c.to_ascii_lowercase() {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86_400,
                'w' => 604_800,
                _ => return None,
            };
            total = total.saturating_add(num.take()?.saturating_mul(unit));
        }
    }
    (!s.is_empty()).then_some(total.saturating_add(num.unwrap_or(0)))
}

fn qualify(name: &str, origin: &str) -> String {
    if name == "@" {
        origin.to_string()
    } else if let Some(abs) = name.strip_suffix('.') {
        abs.to_string()
    } else {
        format!("{name}.{origin}")
    }
}

fn relative(owner: &str, zone: &str) -> Option<String> {
    let (owner, zone) = (owner.to_ascii_lowercase(), zone.to_ascii_lowercase());
    if owner == zone {
        return Some("@".into());
    }
    owner.strip_suffix(&format!(".{zone}")).map(str::to_string)
}

fn number(s: &str) -> Option<u64> {
    s.parse().ok()
}

/// A BIND file's records.
///
/// # Errors
/// A message naming the line, for `$INCLUDE`, `$GENERATE`, a class other than IN, a name
/// outside the zone, or a malformed record.
#[allow(clippy::too_many_lines)]
pub fn parse_bind(text: &str, zone: &str) -> Result<Parsed, String> {
    let mut origin = zone.to_string();
    let mut default_ttl = 1i64;
    let mut prev: Option<String> = None;
    let mut records = Vec::new();
    let mut unmanaged = Vec::new();
    for rec in logical_records(text)? {
        let n = rec.line;
        let bad = |t: &str| format!("line {n}: malformed {t} record");
        let first = rec.tokens[0].0.as_str();
        if !rec.cont && !rec.tokens[0].1 && first.starts_with('$') {
            match first.to_ascii_uppercase().as_str() {
                "$ORIGIN" => {
                    let v = rec.tokens.get(1).ok_or_else(|| bad("$ORIGIN"))?;
                    origin = qualify(&v.0, &origin);
                }
                "$TTL" => {
                    let v = rec.tokens.get(1).ok_or_else(|| bad("$TTL"))?;
                    default_ttl = parse_ttl(&v.0).ok_or_else(|| bad("$TTL"))?;
                }
                "$INCLUDE" => {
                    return Err(format!(
                        "line {n}: $INCLUDE isn't supported; export the zone as one file"
                    ));
                }
                "$GENERATE" => return Err(format!("line {n}: $GENERATE isn't supported")),
                _ => return Err(format!("line {n}: unknown directive {first}")),
            }
            continue;
        }
        let mut rest = rec.tokens.iter().map(|t| t.0.as_str()).peekable();
        let owner = if rec.cont {
            prev.clone()
                .ok_or_else(|| format!("line {n}: no owner name"))?
        } else {
            let name = rest.next().unwrap_or("@");
            qualify(name, &origin).to_ascii_lowercase()
        };
        prev = Some(owner.clone());
        let mut ttl = None;
        for _ in 0..2 {
            let Some(t) = rest.peek().copied() else { break };
            if ["IN", "CH", "HS", "CS"]
                .iter()
                .any(|c| t.eq_ignore_ascii_case(c))
            {
                if !t.eq_ignore_ascii_case("IN") {
                    return Err(format!("line {n}: class {t} isn't IN"));
                }
            } else if let Some(v) = parse_ttl(t) {
                ttl = Some(v);
            } else {
                break;
            }
            rest.next();
        }
        let kind = rest
            .next()
            .ok_or_else(|| format!("line {n}: malformed record"))?
            .to_ascii_uppercase();
        let data: Vec<&str> = rest.collect();
        if kind == "SOA" {
            continue;
        }
        let name = relative(&owner, zone)
            .ok_or_else(|| format!("line {n}: `{owner}` is outside the zone"))?;
        let mut item = Imported {
            name: name.clone(),
            kind: kind.clone(),
            content: None,
            data: None,
            ttl: ttl.unwrap_or(default_ttl),
            priority: None,
            proxied: rec.comment.contains("cf-proxied:true"),
            comment: None,
        };
        match (kind.as_str(), data.as_slice()) {
            ("A" | "AAAA", [v]) => item.content = Some((*v).to_string()),
            ("CNAME" | "NS", [v]) => item.content = Some(qualify(v, &origin)),
            ("MX", [p, v]) => {
                item.priority = Some(p.parse().map_err(|_| bad("MX"))?);
                item.content = Some(qualify(v, &origin));
            }
            ("TXT", [_, ..]) => item.content = Some(data.concat()),
            ("SRV", [p, w, port, target]) => {
                let nums = (number(p), number(w), number(port));
                let (Some(p), Some(w), Some(port)) = nums else {
                    return Err(bad("SRV"));
                };
                item.data = Some(json!({
                    "priority": p, "weight": w, "port": port,
                    "target": qualify(target, &origin),
                }));
            }
            ("CAA", [flags, tag, value]) => {
                let tag = tag.to_ascii_lowercase();
                if !CAA_TAGS.contains(&tag.as_str()) {
                    unmanaged.push(Unmanaged {
                        kind: kind.clone(),
                        name: Some(name.clone()),
                    });
                    continue;
                }
                let flags = number(flags).ok_or_else(|| bad("CAA"))?;
                item.data = Some(json!({"flags": flags, "tag": tag, "value": value}));
            }
            ("A" | "AAAA" | "CNAME" | "NS" | "MX" | "TXT" | "SRV" | "CAA", _) => {
                return Err(bad(&kind));
            }
            _ => {
                unmanaged.push(Unmanaged {
                    kind: kind.clone(),
                    name: Some(name.clone()),
                });
                continue;
            }
        }
        records.push(item);
    }
    Ok((records, unmanaged))
}

fn txt(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].replace("\" \"", "")
    } else {
        s.to_string()
    }
}

fn caa_from_content(content: &str) -> Option<Value> {
    let mut parts = content.trim().splitn(3, ' ');
    let flags = number(parts.next()?)?;
    let tag = parts.next()?;
    let value = parts.next()?.trim().trim_matches('"');
    Some(json!({"flags": flags, "tag": tag, "value": value}))
}

fn pick(data: &Value, keys: &[&str]) -> Option<Value> {
    let mut out = Map::new();
    for k in keys {
        out.insert((*k).to_string(), data.get(k)?.clone());
    }
    Some(Value::Object(out))
}

/// The records of the API's `dns_records` result.
///
/// # Errors
/// A message, never naming a record, for one outside the zone or malformed.
pub fn from_api(records: &[Value], zone: &str) -> Result<Parsed, String> {
    let mut out = Vec::new();
    let mut unmanaged = Vec::new();
    for r in records {
        let kind = r["type"].as_str().unwrap_or("").to_ascii_uppercase();
        let name = r["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let name = relative(&name, zone).ok_or("a record is outside the zone")?;
        if r["meta"]["read_only"] == true {
            unmanaged.push(Unmanaged {
                kind: kind.clone(),
                name: Some(name.clone()),
            });
            continue;
        }
        let bad = || format!("a {kind} record is malformed");
        let content = r["content"].as_str();
        let mut item = Imported {
            name: name.clone(),
            kind: kind.clone(),
            content: None,
            data: None,
            ttl: r["ttl"].as_i64().unwrap_or(1),
            priority: None,
            proxied: r["proxied"] == true,
            comment: r["comment"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string),
        };
        match kind.as_str() {
            "A" | "AAAA" | "CNAME" | "NS" => {
                item.content = Some(content.ok_or_else(bad)?.to_string());
            }
            "MX" => {
                item.content = Some(content.ok_or_else(bad)?.to_string());
                item.priority = Some(r["priority"].as_i64().ok_or_else(bad)?);
            }
            "TXT" => item.content = Some(txt(content.ok_or_else(bad)?)),
            "SRV" => {
                let keys = ["priority", "weight", "port", "target"];
                item.data = Some(if r["data"].is_object() {
                    pick(&r["data"], &keys).ok_or_else(bad)?
                } else {
                    let mut p = content.ok_or_else(bad)?.split_whitespace();
                    let (w, port, target) = (p.next(), p.next(), p.next());
                    let (Some(w), Some(port), Some(target)) = (w, port, target) else {
                        return Err(bad());
                    };
                    json!({
                        "priority": r["priority"].as_u64().ok_or_else(bad)?,
                        "weight": number(w).ok_or_else(bad)?,
                        "port": number(port).ok_or_else(bad)?,
                        "target": target,
                    })
                });
            }
            "CAA" => {
                let data = if r["data"].is_object() {
                    pick(&r["data"], &["flags", "tag", "value"])
                } else {
                    content.and_then(caa_from_content)
                }
                .ok_or_else(bad)?;
                let tag = data["tag"].as_str().unwrap_or("").to_ascii_lowercase();
                if !CAA_TAGS.contains(&tag.as_str()) {
                    unmanaged.push(Unmanaged {
                        kind: kind.clone(),
                        name: Some(name.clone()),
                    });
                    continue;
                }
                item.data = Some(data);
            }
            _ => {
                unmanaged.push(Unmanaged {
                    kind: kind.clone(),
                    name: Some(name.clone()),
                });
                continue;
            }
        }
        out.push(item);
    }
    Ok((out, unmanaged))
}

/// The records `items` (the zone-records expander's) render in `zone`; secret-backed ones cover
/// nothing and are left out.
#[must_use]
pub fn rendered(items: &Map<String, Value>, zone: &str) -> Vec<Rendered> {
    items
        .values()
        .filter(|i| i["zone"].as_str() == Some(zone))
        .filter_map(|i| {
            let content = i["content"].as_str();
            if content.is_some_and(|c| c.contains("${var.secrets[")) {
                return None;
            }
            Some(Rendered {
                name: i["name"].as_str()?.to_ascii_lowercase(),
                kind: i["type"].as_str()?.to_string(),
                content: content.map(|c| c.replace("$${", "${").replace("%%{", "%{")),
                data: i["data"].is_object().then(|| i["data"].clone()),
            })
        })
        .collect()
}

fn trimmed_eq(a: &str, b: &str) -> bool {
    a.trim_end_matches('.')
        .eq_ignore_ascii_case(b.trim_end_matches('.'))
}

fn same_content(kind: &str, a: &str, b: &str) -> bool {
    match kind {
        "TXT" => txt(a) == txt(b),
        "CNAME" | "MX" | "NS" => trimmed_eq(a, b),
        "AAAA" => match (a.parse::<Ipv6Addr>(), b.parse::<Ipv6Addr>()) {
            (Ok(x), Ok(y)) => x == y,
            _ => a == b,
        },
        _ => a == b,
    }
}

fn same_data(a: &Value, b: &Value) -> bool {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
        return false;
    };
    a.len() == b.len()
        && a.iter().all(|(k, x)| {
            b.get(k).is_some_and(|y| match (x, y) {
                (Value::String(x), Value::String(y)) => trimmed_eq(x, y),
                (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
                _ => false,
            })
        })
}

fn covers(r: &Rendered, i: &Imported, zone: &str) -> bool {
    if !r.name.eq_ignore_ascii_case(&apex::fqdn(&i.name, zone)) || r.kind != i.kind {
        return false;
    }
    match (&r.data, &i.data) {
        (Some(a), Some(b)) => same_data(a, b),
        (None, None) => match (&r.content, &i.content) {
            (Some(a), Some(b)) => same_content(&i.kind, a, b),
            _ => false,
        },
        _ => false,
    }
}

fn part(name: &str) -> String {
    let label = apex::label(&name.to_ascii_lowercase());
    let clean: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let joined = clean
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if joined.is_empty() {
        "record".into()
    } else {
        joined
    }
}

/// A record's key: `<label>-<type>`, with `apex-` for `@` and `wildcard-` for a wildcard.
#[must_use]
pub fn key(name: &str, kind: &str) -> String {
    let t = kind.to_ascii_lowercase();
    if name == "@" {
        format!("apex-{t}")
    } else if name == "*" {
        format!("wildcard-{t}")
    } else if let Some(rest) = name.strip_prefix("*.") {
        format!("wildcard-{}-{t}", part(rest))
    } else {
        format!("{}-{t}", part(name))
    }
}

#[derive(Serialize)]
struct Doc<'a> {
    records: BTreeMap<String, Out<'a>>,
}

#[derive(Serialize)]
struct Out<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    comment: Option<&'a str>,
}

/// The `records:` YAML for `source`'s records in `zone`, less the apex NS and those `covered`.
///
/// # Errors
/// The parser's message, or the serializer's.
pub fn import(source: Source<'_>, zone: &str, covered: &[Rendered]) -> Result<Import, String> {
    let (records, unmanaged) = match source {
        Source::Bind(text) => parse_bind(text, zone)?,
        Source::Api(records) => from_api(records, zone)?,
    };
    let kept: Vec<Imported> = records
        .into_iter()
        .filter(|i| !(i.kind == "NS" && i.name == "@"))
        .filter(|i| !covered.iter().any(|r| covers(r, i, zone)))
        .collect();
    let mut taken = BTreeSet::new();
    let mut keyed = BTreeMap::new();
    for i in &kept {
        let base = key(&i.name, &i.kind);
        let mut k = base.clone();
        let mut n = 2;
        while taken.contains(&k) {
            k = format!("{base}-{n}");
            n += 1;
        }
        taken.insert(k.clone());
        keyed.insert(
            k,
            Out {
                name: &i.name,
                kind: &i.kind,
                content: i.content.as_deref(),
                data: i.data.as_ref(),
                ttl: (i.ttl != 1).then_some(i.ttl),
                priority: i.priority.filter(|_| i.kind == "MX"),
                proxied: i.proxied.then_some(true),
                comment: i.comment.as_deref(),
            },
        );
    }
    let yaml = serde_saphyr::to_string(&Doc { records: keyed }).map_err(|e| e.to_string())?;
    Ok(Import {
        yaml,
        records: kept.len(),
        unmanaged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "$ORIGIN example.com.\n$TTL 3600\n@ IN SOA ns1.example.net. hostmaster.example.com. (\n    2026111901 ; serial\n    7200 3600 1209600 300 )\n@ IN NS ns1.example.net.\n@ 300 IN A 192.0.2.1\nwww IN CNAME @\n@ IN MX 10 mx.example.net.\n@ IN TXT \"v=spf1 \" \"-all\"\n_sip._tcp IN SRV 10 5 5060 sip.example.com.\n@ IN CAA 0 issue \"ca.example.net\"\nsub IN NS ns1.example.net.\napi.example.com. 1 IN A 192.0.2.2 ; cf_tags=cf-proxied:true\nmail IN AAAA 2001:db8::1\n     IN AAAA 2001:db8::2\nwww IN PTR host.example.net.\n";

    fn yaml(i: &Import) -> Value {
        serde_saphyr::from_str::<Value>(&i.yaml).expect("yaml")
    }

    #[test]
    fn bind_records_become_records_yaml() {
        let i = import(Source::Bind(FIXTURE), "example.com", &[]).expect("import");
        assert_eq!(
            yaml(&i),
            json!({"records": {
                "apex-a": {"name": "@", "type": "A", "content": "192.0.2.1", "ttl": 300},
                "www-cname": {"name": "www", "type": "CNAME", "content": "example.com", "ttl": 3600},
                "apex-mx": {"name": "@", "type": "MX", "content": "mx.example.net", "ttl": 3600, "priority": 10},
                "apex-txt": {"name": "@", "type": "TXT", "content": "v=spf1 -all", "ttl": 3600},
                "sip-tcp-srv": {"name": "_sip._tcp", "type": "SRV", "ttl": 3600,
                    "data": {"priority": 10, "weight": 5, "port": 5060, "target": "sip.example.com"}},
                "apex-caa": {"name": "@", "type": "CAA", "ttl": 3600,
                    "data": {"flags": 0, "tag": "issue", "value": "ca.example.net"}},
                "sub-ns": {"name": "sub", "type": "NS", "content": "ns1.example.net", "ttl": 3600},
                "api-a": {"name": "api", "type": "A", "content": "192.0.2.2", "proxied": true},
                "mail-aaaa": {"name": "mail", "type": "AAAA", "content": "2001:db8::1", "ttl": 3600},
                "mail-aaaa-2": {"name": "mail", "type": "AAAA", "content": "2001:db8::2", "ttl": 3600},
            }})
        );
        assert_eq!(i.records, 10);
        assert_eq!(
            i.unmanaged,
            vec![Unmanaged {
                kind: "PTR".into(),
                name: Some("www".into())
            }]
        );
    }

    #[test]
    fn colliding_keys_take_numeric_suffixes_in_input_order() {
        let text = "www A 192.0.2.1\nwww A 192.0.2.2\nwww A 192.0.2.3\n";
        let i = import(Source::Bind(text), "example.com", &[]).expect("import");
        let y = yaml(&i);
        assert_eq!(y["records"]["www-a"]["content"], "192.0.2.1");
        assert_eq!(y["records"]["www-a-2"]["content"], "192.0.2.2");
        assert_eq!(y["records"]["www-a-3"]["content"], "192.0.2.3");
    }

    #[test]
    fn wildcards_are_keyed_wildcard() {
        assert_eq!(key("*", "A"), "wildcard-a");
        assert_eq!(key("*.dev", "A"), "wildcard-dev-a");
        assert_eq!(key("*.a.b", "CNAME"), "wildcard-a-b-cname");
        assert_eq!(key("@", "MX"), "apex-mx");
        assert_eq!(key("_sip._tcp", "SRV"), "sip-tcp-srv");
    }

    #[test]
    fn records_the_units_render_are_dropped() {
        let srv = json!({"priority": 10, "weight": 5, "port": 5060, "target": "sip.example.com"});
        let mut items = Map::new();
        items.insert(
            "example.com:apex-txt".into(),
            json!({"zone": "example.com", "name": "example.com", "type": "TXT", "content": "v=spf1 -all", "data": null}),
        );
        items.insert(
            "example.com:sip".into(),
            json!({"zone": "example.com", "name": "_sip._tcp.example.com", "type": "SRV", "content": null, "data": srv}),
        );
        items.insert(
            "example.com:dkim".into(),
            json!({"zone": "example.com", "name": "k._domainkey.example.com", "type": "TXT", "content": "${var.secrets[\"dkim\"]}", "data": null}),
        );
        items.insert(
            "example.net:www".into(),
            json!({"zone": "example.net", "name": "www.example.net", "type": "A", "content": "192.0.2.9", "data": null}),
        );
        let covered = rendered(&items, "example.com");
        assert_eq!(covered.len(), 2);
        let text = "@ TXT \"v=spf1 -all\"\n_sip._tcp SRV 10 5 5060 sip.example.com.\nk._domainkey TXT \"v=DKIM1\"\nwww A 192.0.2.1\n";
        let i = import(Source::Bind(text), "example.com", &covered).expect("import");
        let y = yaml(&i);
        let keys: Vec<&str> = y["records"]
            .as_object()
            .expect("map")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["k-domainkey-txt", "www-a"]);
    }

    #[test]
    fn bind_refuses_include_and_names_outside_the_zone() {
        let err = |t: &str| parse_bind(t, "example.com").expect_err("refused");
        assert!(err("$INCLUDE x\n").contains("$INCLUDE"));
        assert!(err("www.example.net. A 192.0.2.1\n").contains("outside the zone"));
        assert!(err("x IN CH A 192.0.2.1\n").contains("isn't IN"));
    }

    #[test]
    fn the_zone_comes_from_origin_then_soa() {
        assert_eq!(
            origin_of("$ORIGIN Example.COM.\n"),
            Some("example.com".into())
        );
        assert_eq!(
            origin_of("example.com. IN SOA a. b. 1 2 3 4 5\n"),
            Some("example.com".into())
        );
        assert_eq!(origin_of("www A 192.0.2.1\n"), None);
    }
}
