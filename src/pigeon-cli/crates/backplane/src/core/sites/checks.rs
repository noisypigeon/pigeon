//! A published site's live checks, as data: what to fetch and what passes; the shell fetches
//! and retries; see `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §7.

/// One live check: a URL and the response it wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// A 200 whose body holds `<html`.
    Html(String),
    /// A 200.
    Ok(String),
    /// A 404.
    Missing(String),
    /// A 301 to `to`.
    Redirect {
        /// The URL fetched.
        from: String,
        /// Where it must point.
        to: String,
    },
}

impl Probe {
    /// The URL to fetch.
    #[must_use]
    pub fn url(&self) -> &str {
        match self {
            Self::Html(u) | Self::Ok(u) | Self::Missing(u) => u,
            Self::Redirect { from, .. } => from,
        }
    }

    /// What it checks, as the report names it.
    #[must_use]
    pub fn what(&self) -> String {
        match self {
            Self::Html(u) => format!("{u} returns 200 with <html"),
            Self::Ok(u) => format!("{u} returns 200"),
            Self::Missing(u) => format!("{u} returns 404"),
            Self::Redirect { from, to } => format!("{from} redirects 301 to {to}"),
        }
    }

    /// Whether a response, its status, `Location` and body, passes.
    #[must_use]
    pub fn passes(&self, status: u16, location: Option<&str>, body: &str) -> bool {
        match self {
            Self::Html(_) => status == 200 && body.contains("<html"),
            Self::Ok(_) => status == 200,
            Self::Missing(_) => status == 404,
            Self::Redirect { to, .. } => status == 301 && location == Some(to.as_str()),
        }
    }
}

/// The probes for a site served at `canonical` in `zone` on `hosts`, whose bucket holds `keys`;
/// `stamp` makes the missing page's path unique. None without a canonical host.
#[must_use]
pub fn probes<'a>(
    canonical: Option<&str>,
    zone: &str,
    hosts: &[String],
    keys: impl IntoIterator<Item = &'a String>,
    stamp: u64,
) -> Vec<Probe> {
    let Some(apex_host) = canonical else {
        return Vec::new();
    };
    let base = format!("https://{apex_host}");
    let mut list = vec![Probe::Html(format!("{base}/"))];
    if let Some(page) = keys.into_iter().find_map(|k| k.strip_suffix("/index.html")) {
        list.push(Probe::Ok(format!("{base}/{page}")));
        list.push(Probe::Ok(format!("{base}/{page}/")));
    }
    list.push(Probe::Missing(format!("{base}/cobot-missing-{stamp}/")));
    let www = format!("www.{zone}");
    if hosts.iter().any(|h| h == zone) && hosts.contains(&www) {
        list.push(Probe::Redirect {
            from: format!("https://{www}/"),
            to: format!("https://{zone}/"),
        });
    }
    list
}
