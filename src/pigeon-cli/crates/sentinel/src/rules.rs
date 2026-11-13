//! Evaluates rules against a document or a pull request.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::markdown::{self, Line};
use crate::policy::{
    ContainsRule, Direction, Elsewhere, EntriesRule, FieldRule, FieldsRule, FindRule, HeadingsRule,
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
            Rule::Fields(r) => fields(&mut ctx, &lines, r, root),
            Rule::Headings(r) => headings(&mut ctx, &lines, r),
            Rule::Items(r) => items(&mut ctx, &lines, r),
            Rule::Contains(r) => contains(&mut ctx, root, r),
            Rule::Field(_) | Rule::Entries(_) | Rule::Find(_) => {}
        }
    }
}

/// Runs a pull-request policy's rules.
pub fn run_pull_request(
    loaded: &LoadedPolicy,
    pr: &PullRequest,
    root: &Path,
    change: bool,
    out: &mut Vec<Violation>,
) {
    let mut ctx = Ctx {
        policy: &loaded.name,
        path: if change { "change" } else { "pull request" }.into(),
        vars: Vars::new(),
        out,
    };
    let extra = [("branch", &pr.branch), ("scope", &pr.scope)];
    for (name, value) in [
        ("title", &pr.title),
        ("body", &pr.body),
        ("number", &pr.number),
        ("files", &pr.files),
    ]
    .into_iter()
    .chain(extra.into_iter().filter(|_| change))
    {
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

/// Runs an `eviction` policy's `field` rules against `candidate`; a field the policy names that
/// the candidate lacks is a violation, so missing data never grants an eviction.
pub fn run_eviction(
    loaded: &LoadedPolicy,
    candidate: &BTreeMap<String, String>,
    out: &mut Vec<Violation>,
) {
    let mut ctx = Ctx {
        policy: &loaded.name,
        path: "eviction".into(),
        vars: candidate
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        out,
    };
    for rule in &loaded.policy.rules {
        if let Rule::Field(r) = rule {
            let name = field_name(r.name);
            if ctx.vars.contains_key(name) {
                field(&mut ctx, Path::new("."), r);
            } else {
                ctx.push(None, format!("`{name}` is not set"));
            }
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
        match rule {
            Rule::Entries(r) => entries(&mut ctx, root, r),
            Rule::Find(r) => find(&mut ctx, root, r),
            _ => {}
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

/// Repo-relative files and symlinks under `root` (never followed), minus `ignore`,
/// sorted; plus each path that could not be listed, and why.
fn walk(root: &Path, ignore: &[crate::policy::Pattern]) -> (Vec<String>, Vec<(String, String)>) {
    let mut found = Vec::new();
    let mut failed = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let shown = if rel.is_empty() {
            ".".to_string()
        } else {
            rel.clone()
        };
        let listing = match fs::read_dir(root.join(&rel)) {
            Ok(listing) => listing,
            Err(e) => {
                failed.push((shown, e.to_string()));
                continue;
            }
        };
        for entry in listing {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    failed.push((shown.clone(), e.to_string()));
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if ignore.iter().any(|p| p.0.is_match(&path)) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(_) => found.push(path),
                Err(e) => failed.push((path, e.to_string())),
            }
        }
    }
    found.sort();
    failed.sort();
    (found, failed)
}

fn find(ctx: &mut Ctx, root: &Path, r: &FindRule) {
    let (paths, failed) = walk(root, &r.ignore);
    for (path, why) in failed {
        ctx.push_at(path, None, format!("cannot be inspected: {why}"));
    }
    for path in paths {
        if !r.matches.0.is_match(&path)
            || r.allow_under.iter().any(|p| path.starts_with(p.as_str()))
        {
            continue;
        }
        let Some(forbidden) = &r.forbid_content else {
            let message = if r.allow_under.is_empty() {
                "not allowed anywhere".to_string()
            } else {
                format!("allowed only under {}", r.allow_under.join(", "))
            };
            ctx.push_at(path, None, message);
            continue;
        };
        let bytes = match fs::read(root.join(&path)) {
            Ok(bytes) => bytes,
            Err(e) => {
                ctx.push_at(path, None, format!("cannot be inspected: {e}"));
                continue;
            }
        };
        let content = String::from_utf8_lossy(&bytes);
        if let Some((i, line)) = content
            .lines()
            .enumerate()
            .find(|(_, l)| forbidden.0.is_match(l))
        {
            let found = forbidden.0.find(line).map_or("", |m| m.as_str());
            ctx.push_at(
                path,
                Some(i + 1),
                format!("mentions `{found}`, which is not allowed here"),
            );
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

fn fields(ctx: &mut Ctx, lines: &[Line], r: &FieldsRule, root: &Path) {
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
        } else if e.globs == Some(true) {
            for glob in globs(value) {
                if !matches_a_file(root, glob) {
                    ctx.push(
                        Some(l.number),
                        format!("`{}` names `{glob}`, which matches no file", e.key),
                    );
                }
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
    if r.only
        && let Some(l) = rest.next()
    {
        let message = captures(&r.pattern.0, l.text)
            .and_then(|c| c.get("key").cloned())
            .map_or_else(
                || "unexpected line".into(),
                |key| format!("unexpected field `{key}`"),
            );
        ctx.push(Some(l.number), message);
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

fn field_name(f: PrField) -> &'static str {
    match f {
        PrField::Title => "title",
        PrField::Body => "body",
        PrField::Number => "number",
        PrField::Files => "files",
        PrField::Branch => "branch",
        PrField::Scope => "scope",
        PrField::Kind => "kind",
        PrField::State => "state",
        PrField::Pushed => "pushed",
        PrField::SessionLive => "session_live",
        PrField::SessionGoneFor => "session_gone_for",
        PrField::Idle => "idle",
        PrField::Pinned => "pinned",
        PrField::ApprovalOpen => "approval_open",
        PrField::Queued => "queued",
    }
}

/// `glob` as a regex over a `/`-separated path: `**` spans directories, `*` stays within one,
/// and a trailing `/` matches everything under it.
fn glob(glob: &str) -> Option<Regex> {
    let mut re = String::from("^");
    let mut chars = glob.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                re.push_str(".*");
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    if glob.trim().ends_with('/') {
        re.push_str(".*");
    }
    re.push('$');
    Regex::new(&re).ok()
}

/// `field`'s `at_least`, `at_most` and `equals` checks of `value`.
fn bounds(ctx: &mut Ctx, name: &str, value: &str, r: &FieldRule) {
    if r.at_least.is_some() || r.at_most.is_some() {
        match value.trim().parse::<u64>() {
            Ok(n) => {
                if let Some(min) = r.at_least
                    && n < min
                {
                    ctx.push(None, format!("`{name}` is {n}; at least {min}"));
                }
                if let Some(max) = r.at_most
                    && n > max
                {
                    ctx.push(None, format!("`{name}` is {n}; at most {max}"));
                }
            }
            Err(_) => ctx.push(None, format!("`{name}` `{value}` is not a whole number")),
        }
    }
    if let Some(want) = &r.equals {
        let want = want.to_string();
        if value.trim() != want {
            ctx.push(None, format!("`{name}` is `{value}`, not `{want}`"));
        }
    }
}

fn field(ctx: &mut Ctx, root: &Path, r: &FieldRule) {
    let name = field_name(r.name);
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
    bounds(ctx, name, &value, r);
    if let Some(max) = r.max_lines {
        let n = value.lines().filter(|l| !l.trim().is_empty()).count();
        if n > max {
            ctx.push(None, format!("`{name}` has {n} lines; at most {max}"));
        }
    }
    if let Some(other) = r.within {
        let globs: Vec<Regex> = ctx
            .vars
            .get(field_name(other))
            .map(|s| {
                s.lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(glob)
                    .collect()
            })
            .unwrap_or_default();
        let outside: Vec<&str> = value
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !globs.iter().any(|g| g.is_match(l)))
            .collect();
        if !outside.is_empty() {
            ctx.push(
                None,
                format!(
                    "`{name}` holds paths outside `{}`: {}",
                    field_name(other),
                    outside.join(", ")
                ),
            );
        }
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

/// The globs of a `globs` field's value: comma-separated, each optionally in backticks.
fn globs(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(|g| g.trim().trim_matches('`').trim())
        .filter(|g| !g.is_empty())
}

/// Whether a file under `root` matches `glob`; only the glob's literal directory is walked, and
/// never `.git` or `target`.
fn matches_a_file(root: &Path, glob: &str) -> bool {
    let literal = glob.find(['*', '?']).map_or(glob, |i| &glob[..i]);
    if literal == glob {
        return root.join(glob).is_file() || (glob.ends_with('/') && root.join(glob).is_dir());
    }
    let dir = literal.rfind('/').map_or("", |i| &literal[..i]);
    let skip = [crate::policy::Pattern(
        regex::Regex::new(r"(^|/)(\.git|target)$").expect("valid"),
    )];
    let Some(re) = self::glob(glob) else {
        return false;
    };
    let (found, _) = walk(&root.join(dir), &skip);
    found.iter().any(|f| {
        let path = if dir.is_empty() {
            f.clone()
        } else {
            format!("{dir}/{f}")
        };
        re.is_match(&path)
    })
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
    fn fields_globs_condition_needs_each_glob_to_match_a_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let rules = "  - fields: { after: 0, pattern: '^(?<key>\\w+): (?<value>.*)$', expect: [{ key: g, globs: true }] }\n";
        let run_at = |content: &str| {
            let yaml = format!("description: test.\ntarget: {{ file: f }}\nrules:\n{rules}");
            let loaded = LoadedPolicy {
                name: "t".into(),
                policy: Policy::parse(&yaml).expect("valid"),
            };
            let mut out = Vec::new();
            run_document(&loaded, Path::new("f"), content, root, &mut out);
            out.iter().map(|v| v.message.clone()).collect::<Vec<_>>()
        };
        assert_eq!(
            run_at("g: `src/**`, `Cargo.toml`, src/rules.rs, `src/*.rs`, tests/\n"),
            Vec::<String>::new()
        );
        let got = run_at("g: `src/**`, `nowhere/**`, `src/*.md`\n");
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].contains("`nowhere/**`"), "{got:?}");
        assert!(got[1].contains("`src/*.md`"), "{got:?}");
    }

    #[test]
    fn fields_matches_condition() {
        let rules = "  - fields: { after: 0, pattern: '^(?<key>\\w+)=(?<value>.*)$', expect: [{ key: n, matches: '^[0-9]+$' }] }\n";
        assert_eq!(run(rules, "f", "n=12\n"), Vec::<String>::new());
        assert_eq!(run(rules, "f", "\n\nn=x\n").len(), 1);
    }

    #[test]
    fn fields_only_rejects_extra_fields() {
        let rules = "  - fields: { after: 0, pattern: '^(?<key>\\w+)=(?<value>.*)$', only: true, expect: [{ key: a, nonempty: true }, { key: b, nonempty: true }] }\n";
        assert_eq!(run(rules, "f", "a=1\nb=2\n\n"), Vec::<String>::new());
        let extra = run(rules, "f", "a=1\nb=2\nc=3\n");
        assert_eq!(extra.len(), 1);
        assert!(extra[0].contains("unexpected field `c`"), "{extra:?}");
        let raw = run(rules, "f", "a=1\nb=2\nnot-a-field-s3cr3t\n");
        assert_eq!(
            raw,
            ["unexpected line"],
            "a line that is not a field is never echoed"
        );
        let loose = "  - fields: { after: 0, pattern: '^(?<key>\\w+)=(?<value>.*)$', expect: [{ key: a, nonempty: true }] }\n";
        assert_eq!(run(loose, "f", "a=1\nb=2\n"), Vec::<String>::new());
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
