//! A site's `site/` against the template's layout, from the listing the shell took; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md`.

/// What a contract site's `site/` may hold.
pub const SITE_DIRS: [&str; 3] = ["posts", "pages", "assets"];

/// What the shell lists of a site's `site/`.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// `site/` as problems name it.
    pub shown: String,
    /// Its entries, each a name and whether it is a directory; none when it can't be listed.
    pub top: Option<Vec<(String, bool)>>,
    /// Every entry under `posts/` and `pages/`, at any depth, each a path from `site/` and
    /// whether it is a directory, in sorted walk order.
    pub nested: Vec<(String, bool)>,
    /// Each entry of `posts/`, sorted, with its text (empty when unreadable).
    pub posts: Vec<(String, String)>,
}

/// Whether `name` may name a data file: `[a-z0-9-]+`.
#[must_use]
pub fn is_data_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A site whose template has no content must have no `site/`, shown as `shown`.
#[must_use]
pub fn no_content_problems(shown: &str, exists: bool) -> Vec<String> {
    if exists {
        vec![format!(
            "{shown} must not exist: the site's template has no content"
        )]
    } else {
        Vec::new()
    }
}

/// Every way a contract site's `site/` breaks the layout: anything but [`SITE_DIRS`], an
/// `_index.md`, and, with `date_slug`, a misnamed post.
#[must_use]
pub fn layout_problems(listing: &Listing, date_slug: bool) -> Vec<String> {
    let Some(top) = &listing.top else {
        return vec![format!("{} is missing", listing.shown)];
    };
    let mut out: Vec<String> = top
        .iter()
        .filter(|(name, is_dir)| !is_dir || !SITE_DIRS.contains(&name.as_str()))
        .map(|(name, _)| {
            format!(
                "`{name}` does not belong in site/, which holds only posts/, pages/ and assets/"
            )
        })
        .collect();
    // Sections are the template's scaffold.
    out.extend(
        listing
            .nested
            .iter()
            .filter(|(rel, is_dir)| !is_dir && rel.rsplit('/').next() == Some("_index.md"))
            .map(|(rel, _)| format!("`{rel}`: sections are the template's scaffold")),
    );
    if date_slug {
        for (name, text) in &listing.posts {
            if name == "_index.md" {
                continue;
            }
            if let Err(m) = check_post(name, text) {
                out.push(format!("posts/{name}: {m}"));
            }
        }
    }
    out
}

/// The `YYYY-MM-DD` of a name `YYYY-MM-DD-<slug>.md`, its slug lowercase
/// words of `[a-z0-9]` joined by dashes.
fn date_prefix(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let bytes = stem.as_bytes();
    let dated = bytes.len() > 11
        && bytes[..10].iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
        && bytes[10] == b'-';
    if !dated {
        return None;
    }
    let slugged = stem[11..].split('-').all(|w| {
        !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    slugged.then(|| &stem[..10])
}

/// A post's name against `date-slug`: `YYYY-MM-DD-<slug>.md`, its prefix the
/// date of its `date`, and no `slug` of its own.
///
/// # Errors
///
/// What is wrong, in a few words.
pub fn check_post(name: &str, text: &str) -> Result<(), String> {
    let Some(prefix) = date_prefix(name) else {
        return Err("is not named YYYY-MM-DD-<slug>.md".into());
    };
    let front = text
        .strip_prefix("+++")
        .and_then(|rest| rest.split_once("\n+++"))
        .map(|(f, _)| f)
        .ok_or("has no +++ TOML front matter")?;
    let front: toml::Value = toml::from_str(front).map_err(|e| format!("front matter: {e}"))?;
    if front.get("slug").is_some() {
        return Err("sets slug; its name after the date is the slug".into());
    }
    let date = front
        .get("date")
        .map(|d| match d {
            toml::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .ok_or("has no date")?;
    if date.get(..10) != Some(prefix) {
        return Err(format!("is prefixed {prefix}, but its date is {date}"));
    }
    Ok(())
}
