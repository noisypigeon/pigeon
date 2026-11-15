//! `architecture.describe`'s result as text or SVG, for agents; pure, so the Worker uses it; see
//! `.claude/decisions/2026-11-15-runtime-architecture-coverage-agents-deploys.md` §4.

use std::fmt::Write as _;

use serde_json::Value;

/// The forms `architecture.describe` answers in.
pub const FORMATS: [&str; 3] = ["json", "text", "svg"];

/// `result` in `format`: itself for `json` or none, else its text or SVG as a string.
///
/// # Errors
///
/// A format not in [`FORMATS`], named.
pub fn formatted(result: Value, format: Option<&str>) -> Result<Value, String> {
    match format.unwrap_or("json") {
        "json" => Ok(result),
        "text" => Ok(Value::String(text(&result))),
        "svg" => Ok(Value::String(svg(&result))),
        other => Err(format!(
            "`format` is one of {}, not `{other}`",
            FORMATS.join(", ")
        )),
    }
}

fn st(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn joined(v: &Value, sep: &str) -> String {
    list(v).iter().map(st).collect::<Vec<_>>().join(sep)
}

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// The network as an indented outline: clients and their routes, Access, the Worker, each
/// part, each link with its evidence, and the macOS providers.
#[must_use]
pub fn text(result: &Value) -> String {
    let net = &result["network"];
    let mut out = String::from("pigeon's architecture");
    if let Some(sha) = result["deployed"].as_str() {
        let _ = write!(out, ", deployed {sha}");
    }
    out.push('\n');
    if net.is_null() {
        out.push_str("(no network: this instance has no backplane Worker)\n");
        return out;
    }
    clients_text(&mut out, net);
    worker_text(&mut out, net);
    parts_text(&mut out, net);
    out
}

fn clients_text(out: &mut String, net: &Value) {
    out.push_str("\nclients\n");
    for client in list(&net["clients"]) {
        let items = list(&client["items"]);
        let _ = writeln!(out, "  {} ({})", st(&client["title"]), items.len());
        for item in items {
            let _ = match st(&item["v"]) {
                "" => writeln!(out, "    {}", st(&item["k"])),
                said => writeln!(out, "    {}  {said}", st(&item["k"])),
            };
        }
    }
    let access = &net["access"];
    let _ = writeln!(
        out,
        "\naccess\n  hosts: {}\n  service tokens: {}",
        joined(&access["hosts"], ", "),
        joined(&access["tokens"], ", ")
    );
}

fn worker_text(out: &mut String, net: &Value) {
    let worker = &net["worker"];
    let _ = writeln!(
        out,
        "\nworker {}\n  routes: {}\n  queues: {}\n  crons: {}\n  secrets: {}",
        st(&worker["name"]),
        worker["routes"],
        worker["queues"],
        joined(&worker["crons"], ", "),
        worker["secrets"]
    );
    for host in list(&worker["hosts"]) {
        let _ = writeln!(out, "  host: {}  {}", st(&host["k"]), st(&host["v"]));
    }
    for (key, label) in [
        ("objects", "durable objects"),
        ("d1", "d1"),
        ("r2", "r2"),
        ("secrets_store", "secrets store"),
    ] {
        if !list(&worker[key]).is_empty() {
            let _ = writeln!(out, "  {label}: {}", joined(&worker[key], ", "));
        }
    }
    for container in list(&worker["containers"]) {
        let jobs = match joined(&container["jobs"], ", ") {
            none if none.is_empty() => "every other job".to_string(),
            jobs => jobs,
        };
        let _ = writeln!(
            out,
            "  container {} ({}): {jobs}",
            st(&container["class"]),
            st(&container["instance_type"])
        );
    }
}

fn parts_text(out: &mut String, net: &Value) {
    out.push_str("\nparts\n");
    for part in list(&net["parts"]) {
        let many = if part["many"].as_bool() == Some(true) {
            ", one per change"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {} [{}{many}]: {}  ({})",
            st(&part["id"]),
            st(&part["kind"]),
            st(&part["role"]),
            st(&part["source"])
        );
    }
    out.push_str("\nlinks\n");
    for link in list(&net["links"]) {
        let _ = writeln!(
            out,
            "  {} -> {} [{}]: {}  ({})",
            st(&link["from"]),
            st(&link["to"]),
            st(&link["kind"]),
            st(&link["label"]),
            st(&link["source"])
        );
    }
    if !list(&net["macos"]).is_empty() {
        out.push_str("\nmacos\n");
        for provider in list(&net["macos"]) {
            let _ = writeln!(out, "  {}: {}", st(&provider["k"]), st(&provider["v"]));
        }
    }
    if let Some(repo) = net["github"].as_str() {
        let _ = writeln!(out, "\ngithub: {repo}");
    }
}

fn esc(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A card: a box at `at` of `size`, its first line bold.
struct Card<'a> {
    at: (i64, i64),
    size: (i64, i64),
    class: &'a str,
    lines: &'a [String],
}

fn card(out: &mut String, c: &Card<'_>) {
    let ((left, top), (width, height)) = (c.at, c.size);
    let _ = write!(
        out,
        r#"<g class="{}"><rect x="{left}" y="{top}" width="{width}" height="{height}" rx="8"/>"#,
        c.class
    );
    for (row, line) in (0_i64..).zip(c.lines) {
        let weight = if row == 0 { "t" } else { "m" };
        let _ = write!(
            out,
            r#"<text class="{weight}" x="{}" y="{}">{}</text>"#,
            left + 12,
            top + 22 + row * 18,
            esc(line)
        );
    }
    out.push_str("</g>");
}

/// Draws `cards` as one centred row at `top`, at most 320 wide each; its height.
fn row(
    out: &mut String,
    width: i64,
    top: i64,
    cards: &[(String, Vec<String>)],
    class: &str,
) -> i64 {
    let count = i64::try_from(cards.len()).unwrap_or(1).max(1);
    let each = ((width - 40 - (count - 1) * 16) / count).min(320);
    let tallest = cards.iter().map(|c| c.1.len()).max().unwrap_or(0);
    let height = 30 + 18 * i64::try_from(tallest).unwrap_or(0);
    let left = (width - (count * each + (count - 1) * 16)) / 2;
    for (i, (title, rest)) in (0_i64..).zip(cards) {
        let mut lines = vec![title.clone()];
        lines.extend(rest.iter().cloned());
        card(
            out,
            &Card {
                at: (left + i * (each + 16), top),
                size: (each, height),
                class,
                lines: &lines,
            },
        );
    }
    height
}

/// The cards above the diagram: clients, Access and the Worker.
/// A row of cards: each its title and its other lines.
type Cards = Vec<(String, Vec<String>)>;

fn top_cards(net: &Value) -> [(Cards, &'static str); 3] {
    let clients = list(&net["clients"])
        .iter()
        .map(|c| {
            let routes = format!("{} routes", list(&c["items"]).len());
            (st(&c["title"]).to_string(), vec![routes])
        })
        .collect();
    let access = &net["access"];
    let edge = vec![(
        "Access".to_string(),
        vec![
            joined(&access["hosts"], " · "),
            format!("service tokens: {}", joined(&access["tokens"], " · ")),
        ],
    )];
    let worker = &net["worker"];
    let hosts: Vec<&str> = list(&worker["hosts"]).iter().map(|h| st(&h["k"])).collect();
    let summary = format!(
        "{} routes · {} queue · {} cron · {} secrets",
        worker["routes"],
        worker["queues"],
        list(&worker["crons"]).len(),
        worker["secrets"]
    );
    let card = vec![(
        format!("{} · Worker", st(&worker["name"])),
        vec![summary, hosts.join(" · ")],
    )];
    [(clients, "client"), (edge, "edge"), (card, "worker")]
}

/// The connection diagram as `describe` placed it, moved down to `top`.
fn diagram(out: &mut String, net: &Value, top: i64) {
    let _ = write!(out, r#"<g transform="translate(0 {top})">"#);
    for route in list(&net["diagram"]["routes"]) {
        let (label, kind) = list(&net["links"])
            .iter()
            .find(|l| l["id"] == route["id"])
            .map_or(("", ""), |l| (st(&l["label"]), st(&l["kind"])));
        let _ = write!(
            out,
            r#"<g class="link k-{kind}"><path d="{}"/><text x="{}" y="{}" text-anchor="{}">{}</text></g>"#,
            esc(st(&route["d"])),
            int(&route["lx"]),
            int(&route["ly"]),
            esc(st(&route["anchor"])),
            esc(label)
        );
    }
    for placed in list(&net["diagram"]["boxes"]) {
        let part = list(&net["parts"]).iter().find(|p| p["id"] == placed["id"]);
        let kind = part.map_or("", |p| st(&p["kind"]));
        let role = part.map_or("", |p| st(&p["role"]));
        let lines = [
            st(&placed["id"]).to_string(),
            kind.to_string(),
            role.to_string(),
        ];
        card(
            out,
            &Card {
                at: (int(&placed["x"]), int(&placed["y"])),
                size: (int(&placed["w"]), int(&placed["h"])),
                class: &format!("part kind-{kind}"),
                lines: &lines,
            },
        );
    }
    out.push_str("</g>");
}

/// The parts of `kind` outside the diagram, each with what reaches it.
fn outside(net: &Value, kind: &str) -> Vec<(String, Vec<String>)> {
    list(&net["parts"])
        .iter()
        .filter(|p| st(&p["kind"]) == kind)
        .map(|p| {
            let mut rest = vec![st(&p["role"]).to_string()];
            rest.extend(
                list(&net["links"])
                    .iter()
                    .filter(|l| l["to"] == p["id"])
                    .map(|l| format!("← {}", st(&l["from"]))),
            );
            (st(&p["id"]).to_string(), rest)
        })
        .collect()
}

const STYLE: &str = "rect{fill:#14171c;stroke:#3a4048}.edge rect,.worker rect{stroke:#ff7a1a}text{fill:#d8dde3}.t,.h{font-weight:700}.m{fill:#9aa3ad}.link path{fill:none;stroke:#6b7480}.k-store path,.k-git path{stroke:#38d6c4}.k-job path{stroke:#ff7a1a}.link text{fill:#9aa3ad;font-size:10px}";

/// The network as one standalone SVG: clients, Access and the Worker as cards, the
/// connection diagram as `describe` placed it, then the outbound services and GitHub.
#[must_use]
pub fn svg(result: &Value) -> String {
    let net = &result["network"];
    let width = int(&net["diagram"]["width"]).max(1000);
    let mut body = String::new();
    let mut top = 56;
    for (cards, class) in top_cards(net) {
        top += row(&mut body, width, top, &cards, class) + 24;
    }
    top += 48;
    diagram(&mut body, net, top);
    top += int(&net["diagram"]["height"]) + 40;
    for (kind, title) in [("service", "Outbound services"), ("github", "GitHub")] {
        let cards = outside(net, kind);
        if cards.is_empty() {
            continue;
        }
        let _ = write!(
            body,
            r#"<text class="h" x="20" y="{}">{title}</text>"#,
            top + 4
        );
        top += 16;
        top += row(&mut body, width, top, &cards, kind) + 32;
    }
    let sha = result["deployed"]
        .as_str()
        .map(|sha| format!(", deployed {}", &sha[..sha.len().min(12)]))
        .unwrap_or_default();
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{top}" viewBox="0 0 {width} {top}" font-family="ui-monospace, monospace" font-size="12"><style>{STYLE}</style><rect x="0" y="0" width="{width}" height="{top}" style="fill:#0b0d10;stroke:none"/><text class="h" x="20" y="30" font-size="16">Network Diagram{}</text>{body}</svg>
"#,
        esc(&sha)
    )
}
