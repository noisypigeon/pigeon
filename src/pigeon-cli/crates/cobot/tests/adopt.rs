//! Adopting existing resources by name; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use cobot::adopt::{self, Found, Kind, Lookup};
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
            Kind::ServiceToken => vec![found("pigeon-agent", "t1")],
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
fn modules_cobot_cant_find_and_computed_names_are_left_alone() {
    let r = json!({ "module": {
        "zone": { "source": "/p/catalog/cloudflare/modules/zone", "name": "pigeon.dev" },
        "d1": { "source": "/p/catalog/cloudflare/modules/d1-database", "name": "${var.name}" }
    }});
    assert_eq!(
        adopt::imports(&r, "acct", &Fake).expect("imports"),
        Vec::<Value>::new()
    );
}
