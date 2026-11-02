//! The policy file format.

use std::path::PathBuf;

use regex::Regex;
use serde::{Deserialize, Deserializer};

/// A policy: what it guards, what it checks, and its rules.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// One line saying what the policy guards.
    pub description: String,
    /// What the rules run against.
    pub target: Target,
    /// The rules, in order.
    pub rules: Vec<Rule>,
}

/// What a policy's rules run against.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// `pull-request` or `tree`.
    Named(NamedTarget),
    /// `{ file: PATH }`.
    File(FileTarget),
    /// `{ files: [PATH...], optional }`.
    Files(FilesTarget),
    /// `{ dir: PATH, exclude, min }`.
    Dir(DirTarget),
}

/// A target named by a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NamedTarget {
    /// A pull request's title, body and number.
    PullRequest,
    /// The repository's directories, for `entries` rules.
    Tree,
}

/// One file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileTarget {
    /// Relative to the root.
    pub file: PathBuf,
}

/// Several files.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesTarget {
    /// Relative to the root.
    pub files: Vec<PathBuf>,
    /// A missing file is skipped rather than a violation.
    #[serde(default)]
    pub optional: bool,
}

/// Every regular file directly in a directory.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirTarget {
    /// Relative to the root.
    pub dir: PathBuf,
    /// File names to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The fewest files allowed.
    #[serde(default)]
    pub min: usize,
}

/// A compiled regular expression.
#[derive(Debug, Clone)]
pub struct Pattern(pub Regex);

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Regex::new(&s)
            .map(Pattern)
            .map_err(|e| serde::de::Error::custom(format!("invalid regex `{s}`: {e}")))
    }
}

/// One rule.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// The basename matches.
    Filename(FilenameRule),
    /// One line matches.
    Line(LineRule),
    /// Ordered `key: value` lines.
    Fields(FieldsRule),
    /// Headings at one level.
    Headings(HeadingsRule),
    /// Items within sections.
    Items(ItemsRule),
    /// A pull-request field.
    Field(FieldRule),
    /// A file contains a literal.
    Contains(ContainsRule),
    /// A directory holds only allowed entries.
    Entries(EntriesRule),
}

/// See [`Rule::Entries`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntriesRule {
    /// The directory, relative to the root.
    pub path: PathBuf,
    /// Allowed subdirectory names; absent allows none.
    #[serde(default)]
    pub dirs: Vec<Pattern>,
    /// Allowed file names; absent allows none.
    #[serde(default)]
    pub files: Vec<Pattern>,
    /// Names that must exist.
    #[serde(default)]
    pub require: Vec<String>,
    /// Names skipped entirely.
    #[serde(default)]
    pub ignore: Vec<Pattern>,
}

/// See [`Rule::Filename`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilenameRule {
    /// Captures become variables.
    pub pattern: Pattern,
}

/// See [`Rule::Line`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineRule {
    /// 1-based.
    pub at: usize,
    /// Must match; captures become variables.
    pub matches: Option<Pattern>,
    /// Must not match.
    pub not: Option<Pattern>,
}

/// See [`Rule::Fields`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldsRule {
    /// Fields start on the first non-blank line after this one.
    pub after: usize,
    /// Must capture `key` and `value`.
    pub pattern: Pattern,
    /// One entry per field, in order.
    pub expect: Vec<Expect>,
}

/// One expected field. Exactly one condition is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// The field's key.
    pub key: String,
    /// The value equals this, after interpolation.
    pub equals: Option<String>,
    /// The value is not empty.
    pub nonempty: Option<bool>,
    /// The value matches.
    pub matches: Option<Pattern>,
}

/// See [`Rule::Headings`]. Exactly one of `sequence` and `each` is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadingsRule {
    /// The heading level checked.
    pub level: u8,
    /// The first line considered.
    #[serde(default = "first_line")]
    pub from: usize,
    /// The exact headings, in order.
    pub sequence: Option<Vec<SequenceEntry>>,
    /// Every heading matches; captures feed the constraints.
    pub each: Option<Pattern>,
    /// The fewest headings allowed.
    #[serde(default)]
    pub min: usize,
    /// Levels that may not appear.
    #[serde(default)]
    pub forbid_levels: Vec<u8>,
    /// See [`Order`].
    pub order: Option<Order>,
    /// No value of this capture repeats.
    pub unique: Option<String>,
    /// Where both are captured, they are equal.
    pub equal: Option<[String; 2]>,
    /// Where both are captured, the first prefixes the second.
    pub prefix: Option<[String; 2]>,
}

fn first_line() -> usize {
    1
}

/// One heading in a sequence.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceEntry {
    /// The heading's text.
    pub text: String,
    /// May be absent.
    #[serde(default)]
    pub optional: bool,
}

/// See [`Rule::Items`]. Exactly one of `heading` and `lines` is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemsRule {
    /// The sections the items live in.
    pub under: Under,
    /// Items are headings at this level.
    pub heading: Option<u8>,
    /// Items are non-blank lines.
    #[serde(default)]
    pub lines: bool,
    /// Every item matches; captures feed the constraints.
    pub matches: Pattern,
    /// The fewest items per section.
    #[serde(default)]
    pub min: usize,
    /// `forbid`: no content outside the sections, after line 1.
    pub elsewhere: Option<Elsewhere>,
    /// See [`Order`].
    pub order: Option<Order>,
    /// No value of this capture repeats.
    pub unique: Option<String>,
    /// Where both are captured, they are equal.
    pub equal: Option<[String; 2]>,
    /// Where both are captured, the first prefixes the second.
    pub prefix: Option<[String; 2]>,
}

/// Sections headed at `level`, optionally only those titled `text`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Under {
    /// The section heading's level.
    pub level: u8,
    /// The section heading's text.
    pub text: Option<String>,
}

/// What content outside the sections means.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Elsewhere {
    /// A violation.
    Forbid,
}

/// Captured values run in one direction.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    /// The capture compared.
    pub by: String,
    /// The direction.
    pub direction: Direction,
    /// No two consecutive values are equal.
    #[serde(default)]
    pub strict: bool,
    /// No value is before this, after interpolation.
    pub from: Option<String>,
}

/// An ordering direction; values compare as strings.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

/// See [`Rule::Field`]. Exactly one of `matches` and `references` is set.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRule {
    /// The field.
    pub name: PrField,
    /// The field matches.
    pub matches: Option<Pattern>,
    /// The field does not match.
    pub not: Option<Pattern>,
    /// Every match is a path that exists under the root.
    pub references: Option<Pattern>,
    /// Paths not counted as references.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The fewest references.
    #[serde(default)]
    pub min: usize,
}

/// A pull-request field.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrField {
    /// The title.
    Title,
    /// The body.
    Body,
    /// The number.
    Number,
    /// The changed paths, one per line.
    Files,
}

/// See [`Rule::Contains`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainsRule {
    /// Relative to the root.
    pub file: PathBuf,
    /// The literal, after interpolation.
    pub text: String,
}

impl Policy {
    /// Parses and validates a policy.
    ///
    /// # Errors
    ///
    /// A message naming what is invalid.
    pub fn parse(yaml: &str) -> Result<Self, String> {
        let policy: Self = serde_saphyr::from_str(yaml).map_err(|e| e.to_string())?;
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<(), String> {
        if self.description.trim().is_empty() || self.description.contains('\n') {
            return Err("description must be one non-empty line".into());
        }
        let pr = matches!(self.target, Target::Named(NamedTarget::PullRequest));
        let tree = matches!(self.target, Target::Named(NamedTarget::Tree));
        for (i, rule) in self.rules.iter().enumerate() {
            let at = |m: &str| Err(format!("rule {}: {m}", i + 1));
            match rule {
                Rule::Entries(_) if !tree => return at("`entries` needs the `tree` target"),
                Rule::Entries(_) => {}
                _ if tree => return at("only `entries` applies to `tree`"),
                Rule::Field(r) => {
                    if !pr {
                        return at("`field` needs the `pull-request` target");
                    }
                    if (r.matches.is_some() || r.not.is_some()) == r.references.is_some() {
                        return at("`field` takes `references`, or `matches` and/or `not`");
                    }
                }
                Rule::Contains(_) => {}
                _ if pr => return at("only `field` and `contains` apply to `pull-request`"),
                Rule::Fields(r) => {
                    for e in &r.expect {
                        let set = usize::from(e.equals.is_some())
                            + usize::from(e.nonempty == Some(true))
                            + usize::from(e.matches.is_some());
                        if set != 1 {
                            return at(&format!(
                                "`{}` takes exactly one of `equals`, `nonempty: true` and `matches`",
                                e.key
                            ));
                        }
                    }
                }
                Rule::Headings(r) => {
                    if r.sequence.is_some() == r.each.is_some() {
                        return at("`headings` takes exactly one of `sequence` and `each`");
                    }
                    if r.each.is_none()
                        && (r.order.is_some()
                            || r.unique.is_some()
                            || r.equal.is_some()
                            || r.prefix.is_some())
                    {
                        return at("`order`, `unique`, `equal` and `prefix` need `each`");
                    }
                }
                Rule::Items(r) => {
                    if r.heading.is_some() == r.lines {
                        return at("`items` takes exactly one of `heading` and `lines: true`");
                    }
                }
                Rule::Filename(_) | Rule::Line(_) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Policy;

    fn err(yaml: &str) -> String {
        Policy::parse(yaml).expect_err("invalid")
    }

    #[test]
    fn parses_every_target() {
        for target in [
            "pull-request",
            "tree",
            "{ file: CHANGELOG.md }",
            "{ dir: d, exclude: [a], min: 1 }",
        ] {
            let yaml = format!("description: x\ntarget: {target}\nrules: []\n");
            assert!(Policy::parse(&yaml).is_ok(), "{target}");
        }
    }

    #[test]
    fn rejects_unknown_kinds_options_and_bad_regexes() {
        let base = "description: x\ntarget: { file: f }\nrules:\n";
        assert_ne!(err(&format!("{base}  - nope: {{}}\n")), "");
        assert_ne!(
            err(&format!("{base}  - filename: {{ pattern: a, extra: 1 }}\n")),
            ""
        );
        assert!(err(&format!("{base}  - filename: {{ pattern: '(' }}\n")).contains("regex"));
        assert_ne!(
            err("description: x\ntarget: { file: f, more: 1 }\nrules: []\n"),
            ""
        );
    }

    #[test]
    fn rejects_inconsistent_options() {
        let file = "description: x\ntarget: { file: f }\nrules:\n";
        assert!(err(&format!("{file}  - entries: {{ path: . }}\n")).contains("`tree`"));
        assert!(
            err("description: x\ntarget: tree\nrules:\n  - line: { at: 1 }\n")
                .contains("only `entries`")
        );
        assert!(err(&format!("{file}  - headings: {{ level: 2 }}\n")).contains("exactly one of"));
        assert!(
            err(&format!(
                "{file}  - items: {{ under: {{ level: 2 }}, matches: x }}\n"
            ))
            .contains("exactly one of")
        );
        assert!(
            err(&format!("{file}  - field: {{ name: title, matches: x }}\n"))
                .contains("pull-request")
        );
        assert!(
            err("description: x\ntarget: pull-request\nrules:\n  - line: { at: 1 }\n")
                .contains("only")
        );
        assert!(
            err(&format!(
                "{file}  - fields: {{ after: 1, pattern: x, expect: [{{ key: A }}] }}\n"
            ))
            .contains("exactly one of")
        );
        assert!(err("description: ''\ntarget: { file: f }\nrules: []\n").contains("description"));
    }
}
