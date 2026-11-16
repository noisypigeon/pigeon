//! A stored plan's attribute-level diff, for `pigeon cobot show`: what changes, never a
//! sensitive value.

use backplane::cobot::plan_diff;

const SHOWN: &str = r#"{
  "resource_changes": [
    {
      "address": "module.d1.cloudflare_d1_database.this",
      "change": {
        "actions": ["update"],
        "before": { "name": "pigeon", "read_replication": { "mode": "disabled" }, "account_id": "abc123secret", "uuid": "u-1" },
        "after": { "name": "pigeon", "read_replication": null, "account_id": "abc123secret", "uuid": "u-1" },
        "after_unknown": {},
        "before_sensitive": { "account_id": true },
        "after_sensitive": { "account_id": true }
      }
    },
    {
      "address": "module.b.cloudflare_r2_bucket.this",
      "change": {
        "actions": ["delete", "create"],
        "before": { "name": "old", "token": { "value": "s3cr3t" } },
        "after": { "name": "new", "token": { "value": "n3w" }, "id": null },
        "after_unknown": { "id": true },
        "before_sensitive": { "token": { "value": true } },
        "after_sensitive": { "token": { "value": true } },
        "replace_paths": [["name"]]
      }
    },
    { "address": "x.y", "change": { "actions": ["no-op"], "before": {}, "after": {} } }
  ]
}"#;

#[test]
fn only_changed_attributes_are_shown_and_never_a_sensitive_value() {
    let d = plan_diff(SHOWN);
    let lines: Vec<String> = d
        .iter()
        .map(|c| {
            format!(
                "{} {} {}: {} -> {} {}",
                c.action, c.address, c.attribute, c.before, c.after, c.replaces
            )
        })
        .collect();
    assert_eq!(
        lines,
        [
            r#"update module.d1.cloudflare_d1_database.this read_replication: {"mode":"disabled"} -> null false"#,
            "replace module.b.cloudflare_r2_bucket.this id: null -> (known after apply) false",
            r#"replace module.b.cloudflare_r2_bucket.this name: "old" -> "new" true"#,
            "replace module.b.cloudflare_r2_bucket.this token: (sensitive) -> (sensitive) false",
        ]
    );
    let all = lines.join("\n");
    assert!(!all.contains("s3cr3t") && !all.contains("n3w") && !all.contains("abc123secret"));
    assert_eq!(plan_diff("not json"), Vec::new());
}
