//! Evaluates rules against a document or a pull request.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::markdown::{self, Line};
use crate::policy::{
    ContainsRule, Direction, Elsewhere, EntriesRule, FieldRule, FieldsRule, HeadingsRule,
    ItemsRule, LineRule, Order, PrField, Rule,
};
use crate::report::Violation;
use crate::{LoadedPolicy, PullRequest};

type Vars = HashMap<String, String>;

struct Ctx<'a> {
    policy: &'a str,
    path: String,
    vars: Vars,
    out: &'a mut Vec<Violation>,
}

impl Ctx<'_> {
    fn push(&mut self, line: Option<usize>, message: impl Into<String>) {
        let path = self.path.clone();
        self.push_at(path, line, message);
    }

    fn push_at(&mut self, path: String, line: Option<usize>, message: impl Into<String>) {
        self.out.push(Violation {
            path,
            line,
            policy: self.policy.to_string(),
            message: message.into(),
        });
    }

    fn interpolate(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('}') {
                Some(close) if self.vars.contains_key(&after[..close]) => {
                    out.push_str(&self.vars[&after[..close]]);
                    rest = &after[close + 1..];
                }
                _ => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

fn captures(re: &Regex, text: &str) -> Option<Vars> {
    let caps = re.captures(text)?;
    Some(
        re.capture_names()
            .flatten()
            .filter_map(|name| Some((name.to_string(), caps.name(name)?.as_str().to_string())))
            .collect(),
    )
}

/// Runs a file or directory policy's rules against one document.
pub fn run_document(
    loaded: &LoadedPolicy,
    rel: &Path,
    content: &str,
    root: &Path,
    out: &mut Vec<Violation>,
) {
    let lines = markdown::parse(content);
    let mut ctx = Ctx {
        policy: &loaded.name,
        path: rel.display().to_string(),
        vars: Vars::new(),
        out,
    };
    let basename = rel
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    for rule in &loaded.policy.rules {
        match rule {
            Rule::Filename(r) => {
                if let Some(vars) = captures(&r.pattern.0, &basename) {
                    ctx.vars.extend(vars);
                } else {
                    // Later rules depend on the name's captures.
                    ctx.push(
                        None,
                        format!("name `{basename}` does not match `{}`", r.pattern.0),
                    );
                    return;
                }
            }
            Rule::Line(r) => line(&mut ctx, &lines, r),
            Rule::Fields(r) => fields(&mut ctx, &lines, r),
            Rule::Headings(r) => headings(&mut ctx, &lines, r),
            Rule::Items(r) => items(&mut ctx, &lines, r),
            Rule::Contains(r) => contains(&mut ctx, root, r),
            Rule::Field(_) | Rule::Entries(_) => {}
        }
    }
}

/// Runs a pull-request policy's rules.
pub fn run_pull_request(
    loaded: &LoadedPolicy,
    pr: &PullRequest,
    root: &Path,
    out: &mut Vec<Violation>,
) {
    let mut ctx = Ctx {
        policy: &loaded.name,
        path: "pull request".into(),
        vars: Vars::new(),
        out,
    };
    for (name, value) in [
        ("title", &pr.title),
        ("body", &pr.body),
        ("number", &pr.number),
        ("files", &pr.files),
    ] {
        match value {
            Some(v) => {
                ctx.vars.insert(name.into(), v.clone());
            }
            None => ctx.push(None, format!("`{name}` is not set")),
        }
    }
    for rule in &loaded.policy.rules {
        match rule {
            Rule::Field(r) => field(&mut ctx, root, r),
            Rule::Contains(r) => contains(&mut ctx, root, r),
            _ => {}
        }
    }
}

/// Runs a `tree` policy's `entries` rules.
pub fn run_tree(loaded: &LoadedPolicy, root: &Path, out: &mut Vec<Violation>) {
    let mut ctx = Ctx {
        policy: &loaded.name,
        path: String::new(),
        vars: Vars::new(),
        out,
    };
    for rule in &loaded.policy.rules {
        if let Rule::Entries(r) = rule {
            entries(&mut ctx, root, r);
        }
    }
}

fn entries(ctx: &mut Ctx, root: &Path, r: &EntriesRule) {
    let path = r.path.display().to_string();
    let listing = match fs::read_dir(root.join(&r.path)) {
        Ok(listing) => listing,
        Err(e) => {
            ctx.push_at(path, None, format!("cannot list: {e}"));
            return;
        }
    };
    let mut names = Vec::new();
    for entry in listing.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if r.ignore.iter().any(|p| p.0.is_match(&name)) {
            continue;
        }
        names.push((name, entry.path().is_dir()));
    }
    names.sort();
    for (name, is_dir) in &names {
        let (allowed, kind) = if *is_dir {
            (&r.dirs, "directory")
        } else {
            (&r.files, "file")
        };
        if !allowed.iter().any(|p| p.0.is_match(name)) {
            ctx.push_at(path.clone(), None, format!("unexpected {kind} '{name}'"));
        }
    }
    for want in &r.require {
        if !names.iter().any(|(name, _)| name == want) {
            ctx.push_at(path.clone(), None, format!("missing '{want}'"));
        }
    }
}

fn line(ctx: &mut Ctx, lines: &[Line], r: &LineRule) {
    let Some(l) = r.at.checked_sub(1).and_then(|i| lines.get(i)) else {
        ctx.push(None, format!("has no line {}", r.at));
        return;
    };
    if let Some(m) = &r.matches {
        match captures(&m.0, l.text) {
            Some(vars) => ctx.vars.extend(vars),
            None => ctx.push(Some(l.number), format!("line does not match `{}`", m.0)),
        }
    }
    if let Some(n) = &r.not
        && n.0.is_match(l.text)
    {
        ctx.push(
            Some(l.number),
            format!("line matches the forbidden `{}`", n.0),
        );
    }
}

fn fields(ctx: &mut Ctx, lines: &[Line], r: &FieldsRule) {
    let mut rest = lines.iter().skip(r.after).filter(|l| !l.is_blank());
    for e in &r.expect {
        let Some(l) = rest.next() else {
            ctx.push(None, format!("missing field `{}`", e.key));
            return;
        };
        let caps = captures(&r.pattern.0, l.text);
        let Some(caps) = caps.filter(|c| c.get("key") == Some(&e.key)) else {
            let keys: Vec<_> = r.expect.iter().map(|e| e.key.as_str()).collect();
            ctx.push(
                Some(l.number),
                format!(
                    "expected field `{}` (fields in order: {})",
                    e.key,
                    keys.join(", ")
                ),
            );
            return;
        };
        let value = caps.get("value").map_or("", String::as_str);
        if let Some(want) = &e.equals {
            let want = ctx.interpolate(want);
            if value != want {
                ctx.push(
                    Some(l.number),
                    format!("`{}` is `{value}`; expected `{want}`", e.key),
                );
            }
        } else if e.nonempty == Some(true) {
            if value.trim().is_empty() {
                ctx.push(Some(l.number), format!("`{}` is empty", e.key));
            }
        } else if let Some(m) = &e.matches
            && !m.0.is_match(value)
        {
            ctx.push(
                Some(l.number),
                format!("`{}` `{value}` does not match `{}`", e.key, m.0),
            );
        }
    }
}

fn headings(ctx: &mut Ctx, lines: &[Line], r: &HeadingsRule) {
    let considered: Vec<&Line> = lines.iter().filter(|l| l.number >= r.from).collect();
    let mut heads = Vec::new();
    for l in &considered {
        let Some((level, text)) = l.heading else {
            continue;
        };
        if r.forbid_levels.contains(&level) {
            ctx.push(
                Some(l.number),
                format!("a level-{level} heading is not allowed"),
            );
        }
        if level == r.level {
            heads.push((l.number, text));
        }
    }
    let last = lines.last().map(|l| l.number);
    if heads.len() < r.min {
        ctx.push(
            last,
            format!(
                "{} level-{} heading(s); at least {} required",
                heads.len(),
                r.level,
                r.min
            ),
        );
    }
    if let Some(seq) = &r.sequence {
        let expected: Vec<_> = seq.iter().map(|e| format!("`{}`", e.text)).collect();
        let mut next = 0;
        for (number, text) in &heads {
            let mut k = next;
            let found = loop {
                match seq.get(k) {
                    Some(e) if e.text == *text => break Some(k),
                    Some(e) if e.optional => k += 1,
                    _ => break None,
                }
            };
            if let Some(k) = found {
                next = k + 1;
            } else {
                ctx.push(
                    Some(*number),
                    format!(
                        "unexpected heading `{text}`; level-{} headings are {} in order",
                        r.level,
                        expected.join(", ")
                    ),
                );
                return;
            }
        }
        for e in seq.iter().skip(next).filter(|e| !e.optional) {
            ctx.push(last, format!("missing heading `{}`", e.text));
        }
    }
    if let Some(each) = &r.each {
        let mut matched = Vec::new();
        for (number, text) in heads {
            match captures(&each.0, text) {
                Some(vars) => matched.push((number, vars)),
                None => ctx.push(
                    Some(number),
                    format!("heading `{text}` does not match `{}`", each.0),
                ),
            }
        }
        constraints(
            ctx,
            &matched,
            r.order.as_ref(),
            r.unique.as_ref(),
            r.equal.as_ref(),
            r.prefix.as_ref(),
        );
    }
}

fn items(ctx: &mut Ctx, lines: &[Line], r: &ItemsRule) {
    struct Section {
        line: usize,
        title: String,
        count: usize,
    }
    let mut sections: Vec<Section> = Vec::new();
    let mut inside = false;
    let mut matched = Vec::new();
    for l in lines {
        if let Some((level, text)) = l.heading
            && level <= r.under.level
        {
            inside = level == r.under.level && r.under.text.as_deref().is_none_or(|t| t == text);
            if inside {
                sections.push(Section {
                    line: l.number,
                    title: text.to_string(),
                    count: 0,
                });
                continue;
            }
        }
        if !inside {
            if matches!(r.elsewhere, Some(Elsewhere::Forbid)) && l.number > 1 && !l.is_blank() {
                ctx.push(Some(l.number), "content outside a section");
            }
            continue;
        }
        let item = match r.heading {
            Some(n) => l.heading.is_some_and(|(level, _)| level == n),
            None => !l.is_blank(),
        };
        if !item {
            continue;
        }
        if let Some(s) = sections.last_mut() {
            s.count += 1;
        }
        let text = l.heading.map_or(l.text, |(_, t)| t);
        match captures(&r.matches.0, text) {
            Some(vars) => matched.push((l.number, vars)),
            None => ctx.push(
                Some(l.number),
                format!("`{text}` does not match `{}`", r.matches.0),
            ),
        }
    }
    for s in &sections {
        if s.count < r.min {
            ctx.push(
                Some(s.line),
                format!(
                    "section `{}` has {} item(s); at least {} required",
                    s.title, s.count, r.min
                ),
            );
        }
    }
    constraints(
        ctx,
        &matched,
        r.order.as_ref(),
        r.unique.as_ref(),
        r.equal.as_ref(),
        r.prefix.as_ref(),
    );
}

fn constraints(
    ctx: &mut Ctx,
    matched: &[(usize, Vars)],
    order: Option<&Order>,
    unique: Option<&String>,
    equal: Option<&[String; 2]>,
    prefix: Option<&[String; 2]>,
) {
    if let Some(o) = order {
        let from = o.from.as_deref().map(|f| ctx.interpolate(f));
        let mut prev: Option<&str> = None;
        for (number, vars) in matched {
            let Some(v) = vars.get(&o.by) else {
                continue;
            };
            if let Some(f) = &from
                && v.as_str() < f.as_str()
            {
                ctx.push(Some(*number), format!("`{}` {v} is before {f}", o.by));
            }
            if let Some(p) = prev {
                let (bad, word) = match o.direction {
                    Direction::Ascending if o.strict => (v.as_str() <= p, "strictly ascending"),
                    Direction::Ascending => (v.as_str() < p, "ascending"),
                    Direction::Descending if o.strict => (v.as_str() >= p, "strictly descending"),
                    Direction::Descending => (v.as_str() > p, "descending"),
                };
                if bad {
                    ctx.push(
                        Some(*number),
                        format!("`{}` {v} after {p} is not {word}", o.by),
                    );
                }
            }
            prev = Some(v);
        }
    }
    if let Some(name) = unique {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (number, vars) in matched {
            if let Some(v) = vars.get(name) {
                if let Some(first) = seen.get(v.as_str()) {
                    ctx.push(Some(*number), format!("`{name}` {v} repeats line {first}"));
                } else {
                    seen.insert(v, *number);
                }
            }
        }
    }
    for (pair, relation) in [(equal, "equal"), (prefix, "prefix")] {
        let Some([a, b]) = pair else {
            continue;
        };
        for (number, vars) in matched {
            if let (Some(x), Some(y)) = (vars.get(a), vars.get(b)) {
                let holds = if relation == "equal" {
                    x == y
                } else {
                    y.starts_with(x.as_str())
                };
                if !holds {
                    let verb = if relation == "equal" {
                        "does not equal"
                    } else {
                        "is not a prefix of"
                    };
                    ctx.push(Some(*number), format!("`{a}` {x} {verb} `{b}` {y}"));
                }
            }
        }
    }
}

fn field(ctx: &mut Ctx, root: &Path, r: &FieldRule) {
    let name = match r.name {
        PrField::Title => "title",
        PrField::Body => "body",
        PrField::Number => "number",
        PrField::Files => "files",
    };
    let Some(value) = ctx.vars.get(name).cloned() else {
        return;
    };
    if let Some(m) = &r.matches
        && !m.0.is_match(&value)
    {
        let shown = if matches!(r.name, PrField::Body | PrField::Files) {
            String::new()
        } else {
            format!(" `{value}`")
        };
        ctx.push(None, format!("`{name}`{shown} does not match `{}`", m.0));
    }
    if let Some(n) = &r.not
        && let Some(found) = n.0.find(&value)
    {
        ctx.push(
            None,
            format!(
                "`{name}` holds `{}`, which `{}` forbids",
                found.as_str(),
                n.0
            ),
        );
    }
    if let Some(re) = &r.references {
        let mut cited: Vec<&str> = Vec::new();
        for m in re.0.find_iter(&value) {
            let m = m.as_str();
            if !r.exclude.iter().any(|e| e == m) && !cited.contains(&m) {
                cited.push(m);
            }
        }
        for m in &cited {
            if !root.join(m).is_file() {
                ctx.push(None, format!("`{name}` cites {m}, which does not exist"));
            }
        }
        if cited.len() < r.min {
            ctx.push(
                None,
                format!(
                    "`{name}` cites {} reference(s) matching `{}`; at least {} required",
                    cited.len(),
                    re.0,
                    r.min
                ),
            );
        }
    }
}

fn contains(ctx: &mut Ctx, root: &Path, r: &ContainsRule) {
    let text = ctx.interpolate(&r.text);
    let path = r.file.display().to_string();
    match fs::read_to_string(root.join(&r.file)) {
        Ok(content) if content.contains(&text) => {}
        Ok(_) => ctx.push_at(path, None, format!("does not contain `{text}`")),
        Err(e) => ctx.push_at(path, None, format!("cannot read: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::run_document;
    use crate::{LoadedPolicy, Policy};

    fn run(rules: &str, name: &str, content: &str) -> Vec<String> {
        let yaml = format!("description: test.\ntarget: {{ file: {name} }}\nrules:\n{rules}");
        let loaded = LoadedPolicy {
            name: "t".into(),
            policy: Policy::parse(&yaml).expect("valid"),
        };
        let mut out = Vec::new();
        run_document(&loaded, Path::new(name), content, Path::new("."), &mut out);
        out.iter().map(|v| v.message.clone()).collect()
    }

    #[test]
    fn interpolates_captures_and_keeps_unknown_braces() {
        let rules = "  - filename: { pattern: '^(?<n>[a-z]+)\\.md$' }\n  - fields: { after: 0, pattern: '^(?<key>\\w+)=(?<value>.*)$', expect: [{ key: a, equals: '{n}-{x}' }] }\n";
        assert_eq!(run(rules, "abc.md", "a=abc-{x}\n"), Vec::<String>::new());
        assert_eq!(
            run(rules, "abc.md", "a=abc-y\n"),
            vec!["`a` is `abc-y`; expected `abc-{x}`"]
        );
    }

    #[test]
    fn fields_matches_condition() {
        let rules = "  - fields: { after: 0, pattern: '^(?<key>\\w+)=(?<value>.*)$', expect: [{ key: n, matches: '^[0-9]+$' }] }\n";
        assert_eq!(run(rules, "f", "n=12\n"), Vec::<String>::new());
        assert_eq!(run(rules, "f", "\n\nn=x\n").len(), 1);
    }

    #[test]
    fn strict_ascending_rejects_repeats() {
        let rules = "  - headings: { level: 2, each: '^(?<v>\\d+)$', order: { by: v, direction: ascending, strict: true } }\n";
        assert_eq!(run(rules, "f", "## 1\n## 2\n"), Vec::<String>::new());
        assert_eq!(run(rules, "f", "## 1\n## 1\n").len(), 1);
    }

    #[test]
    fn items_ignore_fenced_headings() {
        let rules = "  - items: { under: { level: 2 }, heading: 3, matches: '^ok$', min: 1 }\n";
        assert_eq!(
            run(rules, "f", "## s\n### ok\n```\n### bad\n```\n"),
            Vec::<String>::new()
        );
    }
}

#[cfg(test)]
mod tree_tests {
    use std::fs;

    use tempfile::TempDir;

    use super::run_tree;
    use crate::{LoadedPolicy, Policy};

    fn run(rule: &str, root: &TempDir) -> Vec<String> {
        let yaml = format!("description: test.\ntarget: tree\nrules:\n  - entries: {rule}\n");
        let loaded = LoadedPolicy {
            name: "t".into(),
            policy: Policy::parse(&yaml).expect("valid"),
        };
        let mut out = Vec::new();
        run_tree(&loaded, root.path(), &mut out);
        out.iter().map(ToString::to_string).collect()
    }

    fn tree() -> TempDir {
        let root = TempDir::new().expect("tempdir");
        fs::create_dir(root.path().join("a")).expect("mkdir");
        fs::create_dir(root.path().join(".git")).expect("mkdir");
        fs::write(root.path().join("f.md"), "").expect("write");
        root
    }

    #[test]
    fn allows_by_kind() {
        let root = tree();
        let rule = "{ path: ., dirs: ['^a$'], files: ['^f\\.md$'], ignore: ['^\\.git$'] }";
        assert_eq!(run(rule, &root), Vec::<String>::new());
        let swapped = "{ path: ., dirs: ['^f\\.md$'], files: ['^a$'], ignore: ['^\\.git$'] }";
        assert_eq!(
            run(swapped, &root),
            vec![
                ".: t: unexpected directory 'a'",
                ".: t: unexpected file 'f.md'"
            ]
        );
    }

    #[test]
    fn absent_lists_allow_nothing_and_ignore_skips() {
        let root = tree();
        assert_eq!(run("{ path: . }", &root).len(), 3);
        assert_eq!(run("{ path: ., ignore: ['.'] }", &root).len(), 0);
    }

    #[test]
    fn require_and_unlistable_paths() {
        let root = tree();
        assert_eq!(
            run(
                "{ path: ., dirs: ['.'], files: ['.'], require: [a, b] }",
                &root
            ),
            vec![".: t: missing 'b'"]
        );
        assert_eq!(run("{ path: nope }", &root).len(), 1);
    }
}
