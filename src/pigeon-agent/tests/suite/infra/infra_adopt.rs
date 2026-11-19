//! Adopting existing resources by name; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use agent::infra::adopt::{self, Found, Kind, Lookup, Record};
use serde_json::{Value, json};

struct Fake;

fn found(name: &str, id: &str) -> Found {
    Found {
        name: name.into(),
        id: id.into(),
    }
}

impl Lookup for Fake {
    fn list(&self, kind: Kind, _jurisdiction: &str) -> Result<Vec<Found>, String> {
        Ok(match kind {
            Kind::D1 => vec![found("pigeon", "db1"), found("other", "db2")],
            Kind::Bucket => vec![
                found("pigeon-ci-x7k2qa", "pigeon-ci-x7k2qa"),
                found("pigeon-state-a1b2c3", "pigeon-state-a1b2c3"),
                found("pigeon-cinema", "pigeon-cinema"),
            ],
            Kind::Queue => vec![found("pigeon-ci-events", "q1")],
            Kind::AccessApp => vec![found("pigeon-control", "app1")],
            Kind::AccessPolicy => vec![
                found("pigeon-owner", "p1"),
                found("dup", "p2"),
                found("dup", "p3"),
                found("agent", "p4"),
            ],
            Kind::ServiceToken => vec![found("pigeon-agent", "t1"), found("pigeon-pool", "t2")],
        })
    }
}

fn rendered() -> Value {
    let src = |m: &str| format!("/platform/catalog/cloudflare/modules/{m}");
    json!({ "module": {
        "account": { "source": src("account") },
        "d1": { "source": src("d1-database"), "name": "pigeon" },
        "bucket_suffixes": { "source": "/platform/catalog/agnostic/modules/random-string",
            "for_each": { "pigeon-ci": { "name": "pigeon-ci" }, "pigeon-state": { "name": "pigeon-state" }, "pigeon-plans": { "name": "pigeon-plans" } } },
        "buckets": { "source": src("r2-bucket"), "name": "${each.value.name}",
            "suffix": "${module.bucket_suffixes[each.value.name].result}",
            "for_each": { "pigeon-ci": { "name": "pigeon-ci" }, "pigeon-state": { "name": "pigeon-state" }, "pigeon-plans": { "name": "pigeon-plans" } } },
        "queue": { "source": src("queue"), "name": "pigeon-ci-events" },
        "application": { "source": src("access-application"), "name": "pigeon-control" },
        "owners": { "source": src("access-policy"), "name": "pigeon-owner" },
        "service_tokens": { "source": src("access-service-token"), "name": "${each.value.name}",
            "for_each": { "pigeon-agent": { "name": "pigeon-agent" }, "pigeon-builds": { "name": "pigeon-builds" } } },
        "adopted_service_tokens": { "source": src("access-service-token-adopted"), "name": "${each.value.name}",
            "for_each": { "pigeon-pool": { "name": "pigeon-pool" } } },
        "token_policies": { "source": src("access-policy"), "name": "${each.value.policy}",
            "for_each": { "pigeon-agent": { "name": "pigeon-agent", "policy": "agent" } } }
    }})
}

fn to(imports: &[Value], address: &str) -> Option<String> {
    imports
        .iter()
        .find(|i| i["to"] == address)
        .and_then(|i| i["id"].as_str().map(str::to_string))
}

#[test]
fn each_declared_name_the_account_holds_is_imported_and_the_rest_created() {
    let got = adopt::imports(&rendered(), "acct", &Fake).expect("imports");
    assert_eq!(
        to(&got, "module.d1.cloudflare_d1_database.this").as_deref(),
        Some("acct/db1")
    );
    assert_eq!(
        to(
            &got,
            "module.buckets[\"pigeon-ci\"].cloudflare_r2_bucket.this"
        )
        .as_deref(),
        Some("acct/pigeon-ci-x7k2qa/eu")
    );
    assert_eq!(
        to(
            &got,
            "module.bucket_suffixes[\"pigeon-ci\"].random_string.this"
        )
        .as_deref(),
        Some("x7k2qa")
    );
    assert_eq!(
        to(&got, "module.queue.cloudflare_queue.this").as_deref(),
        Some("acct/q1")
    );
    assert_eq!(
        to(
            &got,
            "module.application.cloudflare_zero_trust_access_application.this"
        )
        .as_deref(),
        Some("accounts/acct/app1")
    );
    assert_eq!(
        to(
            &got,
            "module.owners.cloudflare_zero_trust_access_policy.this"
        )
        .as_deref(),
        Some("acct/p1")
    );
    assert_eq!(
        to(&got, "module.service_tokens[\"pigeon-agent\"].cloudflare_zero_trust_access_service_token.this")
            .as_deref(),
        Some("accounts/acct/t1")
    );
    // An adopted token is only read, so nothing is imported for it.
    assert!(got.iter().all(|i| {
        !i["to"]
            .as_str()
            .unwrap_or_default()
            .starts_with("module.adopted_service_tokens")
    }));
    // Not held, so created: the plans bucket and the builds token.
    assert!(got.iter().all(|i| {
        !i["to"]
            .as_str()
            .unwrap_or_default()
            .contains("pigeon-plans")
    }));
    assert!(got.iter().all(|i| {
        !i["to"]
            .as_str()
            .unwrap_or_default()
            .contains("pigeon-builds")
    }));
    // `pigeon-cinema` isn't `pigeon-ci` with a suffix, so it isn't adopted.
    assert!(got.iter().all(|i| i["id"] != "acct/pigeon-cinema/eu"));
    assert_eq!(
        to(
            &got,
            "module.bucket_suffixes[\"pigeon-state\"].random_string.this"
        )
        .as_deref(),
        Some("a1b2c3")
    );
    assert_eq!(
        to(
            &got,
            "module.token_policies[\"pigeon-agent\"].cloudflare_zero_trust_access_policy.this"
        )
        .as_deref(),
        Some("acct/p4"),
        "a name from the item's own field"
    );
    assert_eq!(got.len(), 10, "{got:#?}");
}

#[test]
fn a_name_held_twice_stops_the_plan() {
    let mut r = rendered();
    r["module"]["owners"]["name"] = json!("dup");
    let e = adopt::imports(&r, "acct", &Fake).expect_err("ambiguous");
    assert!(e.contains("`dup`"), "{e}");
}

#[test]
fn modules_infra_cant_find_and_computed_names_are_left_alone() {
    let r = json!({ "module": {
        "zone": { "source": "/p/catalog/cloudflare/modules/zone", "name": "pigeon.dev" },
        "d1": { "source": "/p/catalog/cloudflare/modules/d1-database", "name": "${var.name}" }
    }});
    assert_eq!(
        adopt::imports(&r, "acct", &Fake).expect("imports"),
        Vec::<Value>::new()
    );
}

/// This repository's `platform/blue`, rendered as a plan would render it.
fn platform_blue() -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let units = agent::infra::select_in(
        &root,
        &root,
        &root.join(".config"),
        &["platform/blue".to_string()],
    )
    .expect("platform/blue is valid");
    agent::infra::render_json(
        &units[0],
        std::path::Path::new("/state"),
        &agent::infra::providers::Allowlist::default(),
        agent::infra::generations::day("2026-11-18").expect("day"),
        &agent::infra::config::Existing::new(),
    )
}

/// An account holding only the pool's token.
struct Held;

impl Lookup for Held {
    fn list(&self, kind: Kind, _jurisdiction: &str) -> Result<Vec<Found>, String> {
        Ok(match kind {
            Kind::ServiceToken => vec![found("pigeon-pool", "t9")],
            _ => Vec::new(),
        })
    }
}

// The pool's hand-made token is read, never managed, so no plan can rotate its secret; see
// .claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md, its 2026-11-18
// amendments.
#[test]
fn an_adopted_token_is_only_read_and_never_imported() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tf = std::fs::read_to_string(
        root.join("catalog/cloudflare/modules/access-service-token-adopted/main.tf"),
    )
    .expect("module");
    assert!(!tf.contains("resource \""), "the module only reads");

    let json = platform_blue();
    let m = &json["module"];
    assert!(
        m["adopted_service_tokens"]["for_each"]
            .get("pigeon-pool")
            .is_some()
    );
    assert!(m["service_tokens"]["for_each"].get("pigeon-pool").is_none());
    assert!(
        m["adopted_token_policies"]["service_token_id"]
            .as_str()
            .expect("an input")
            .contains("module.adopted_service_tokens[")
    );
    let got = adopt::imports(&json, "acct", &Held).expect("imports");
    assert!(
        got.iter().all(|i| i["id"] != "accounts/acct/t9"),
        "{got:#?}"
    );
}

struct Zone {
    calls: std::cell::Cell<usize>,
    records: Vec<Record>,
}

impl Lookup for Zone {
    fn list(&self, _kind: Kind, _jurisdiction: &str) -> Result<Vec<Found>, String> {
        Ok(vec![])
    }

    fn records(&self, zone: &str) -> Result<(String, Vec<Record>), String> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(zone, "example.com");
        Ok(("zone1".into(), self.records.clone()))
    }
}

fn rec(id: &str, name: &str, kind: &str, content: &str, data: Option<Value>) -> Record {
    Record {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        content: content.into(),
        data,
    }
}

fn dns(items: &Value) -> Value {
    json!({ "module": {
        "zone": { "source": "/platform/catalog/cloudflare/modules/zone", "name": "example.com" },
        "dns_records": { "source": "/platform/catalog/cloudflare/modules/dns-record",
            "zone_id": "${module.zone.id}", "for_each": items }
    }})
}

fn record_address(key: &str) -> String {
    format!("module.dns_records[\"{key}\"].cloudflare_dns_record.this")
}

#[test]
fn dns_records_are_adopted_by_name_type_and_content_or_data() {
    let items = json!({
        "example.com:www-a": { "zone": "example.com", "name": "www.example.com", "type": "A", "content": "192.0.2.1", "data": null },
        "example.com:apex-txt": { "zone": "example.com", "name": "example.com", "type": "TXT", "content": "v=spf1 -all", "data": null },
        "example.com:sip": { "zone": "example.com", "name": "_sip._tcp.example.com", "type": "SRV", "content": null,
            "data": { "priority": 10, "weight": 5, "port": 5060, "target": "sip.example.com" } },
        "example.com:caa": { "zone": "example.com", "name": "example.com", "type": "CAA", "content": null,
            "data": { "flags": 0, "tag": "issue", "value": "ca.example.net" } },
        "example.com:mail-mx": { "zone": "example.com", "name": "mail.example.com", "type": "MX", "content": "mx2.example.net", "data": null }
    });
    let lookup = Zone {
        calls: std::cell::Cell::new(0),
        records: vec![
            rec("r1", "www.example.com", "A", "192.0.2.1", None),
            rec("r2", "example.com", "TXT", "\"v=spf1 -all\"", None),
            rec(
                "r3",
                "_sip._tcp.example.com",
                "SRV",
                "",
                Some(
                    json!({ "priority": 10, "weight": 5, "port": 5060, "target": "sip.example.com." }),
                ),
            ),
            rec(
                "r4",
                "example.com",
                "CAA",
                "",
                Some(json!({ "flags": 0, "tag": "issue", "value": "ca.example.net" })),
            ),
            rec("r5", "www.example.com", "AAAA", "2001:db8::1", None),
            rec("r6", "mail.example.com", "MX", "mx.example.net", None),
        ],
    };
    let got = adopt::imports(&dns(&items), "acct", &lookup).expect("imports");
    assert_eq!(
        to(&got, &record_address("example.com:www-a")).as_deref(),
        Some("zone1/r1")
    );
    assert_eq!(
        to(&got, &record_address("example.com:apex-txt")).as_deref(),
        Some("zone1/r2")
    );
    assert_eq!(
        to(&got, &record_address("example.com:sip")).as_deref(),
        Some("zone1/r3")
    );
    assert_eq!(
        to(&got, &record_address("example.com:caa")).as_deref(),
        Some("zone1/r4")
    );
    assert_eq!(to(&got, &record_address("example.com:mail-mx")), None);
    assert!(got.iter().all(|i| {
        !i["to"]
            .as_str()
            .unwrap_or_default()
            .contains("cloudflare_zone")
    }));
    assert_eq!(lookup.calls.get(), 1);
}

#[test]
fn a_record_matching_two_live_records_is_refused() {
    let items = json!({
        "example.com:www-a": { "zone": "example.com", "name": "www.example.com", "type": "A", "content": "192.0.2.1", "data": null }
    });
    let lookup = Zone {
        calls: std::cell::Cell::new(0),
        records: vec![
            rec("r1", "www.example.com", "A", "192.0.2.1", None),
            rec("r2", "www.example.com", "A", "192.0.2.1", None),
        ],
    };
    let err = adopt::imports(&dns(&items), "acct", &lookup).expect_err("ambiguous");
    assert!(err.contains("matches 2 live records"), "{err}");
}

#[test]
fn a_secret_record_is_adopted_by_name_and_type() {
    let items = json!({
        "example.com:dkim": { "zone": "example.com", "name": "k._domainkey.example.com", "type": "TXT", "content": "${var.secrets[\"dkim\"]}", "data": null }
    });
    let lookup = Zone {
        calls: std::cell::Cell::new(0),
        records: vec![rec(
            "r7",
            "k._domainkey.example.com",
            "TXT",
            "\"v=DKIM1; p=abc\"",
            None,
        )],
    };
    let got = adopt::imports(&dns(&items), "acct", &lookup).expect("imports");
    assert_eq!(
        to(&got, &record_address("example.com:dkim")).as_deref(),
        Some("zone1/r7")
    );
}
