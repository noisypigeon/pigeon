//! The registry: what each operation asks, needs and is reachable through.

use super::{ADMINISTRATOR_OPS, Asks, DEVICE_OPS, Door, Effect, GRANT, PAGE, catalog};
use crate::approval::grants;
use crate::member::{CAPABILITIES, PERMISSIONS, Requires, SERVICE_CEILING, STANDARD_CEILING};
use crate::view::View;

#[test]
fn every_grant_and_page_name_is_in_the_catalog() {
    let names: Vec<&str> = catalog().iter().map(|o| o.name).collect();
    for name in GRANT.iter().chain(PAGE.iter()) {
        assert!(names.contains(name), "{name} is not in the catalog");
    }
}

#[test]
fn the_grant_set_is_grants_operations() {
    let mut got: Vec<&str> = catalog()
        .iter()
        .filter(|o| o.asks() == Asks::Grant)
        .map(|o| o.name)
        .collect();
    got.sort_unstable();
    let mut want: Vec<&str> = grants::OPERATIONS.to_vec();
    want.sort_unstable();
    assert_eq!(got, want);
}

#[test]
fn no_operation_is_both_grant_and_page() {
    for o in catalog() {
        assert!(
            !(GRANT.contains(&o.name) && PAGE.contains(&o.name)),
            "{} is both grant and page",
            o.name
        );
    }
}

#[test]
fn every_requirement_is_known() {
    for o in catalog() {
        match o.requires() {
            Requires::Permission(p) => {
                assert!(PERMISSIONS.contains(&p), "{} needs {p}", o.name);
            }
            Requires::Capability { name, scope_arg } => {
                assert!(CAPABILITIES.contains(&name), "{} needs {name}", o.name);
                if let Some(a) = scope_arg {
                    assert!(
                        o.args.iter().any(|arg| arg.name == a),
                        "{} scopes {name} by {a}, which is not its argument",
                        o.name
                    );
                }
            }
            Requires::Own => {}
        }
    }
}

#[test]
fn reads_need_read() {
    for o in catalog().iter().filter(|o| {
        o.effect == Effect::Read
            && !ADMINISTRATOR_OPS.iter().any(|(n, _)| *n == o.name)
            && !DEVICE_OPS.contains(&o.name)
    }) {
        assert_eq!(
            o.permission(),
            Some("read"),
            "{} needs {:?}",
            o.name,
            o.requires()
        );
    }
}

#[test]
fn a_service_account_may_ask_every_control_plane_operation() {
    for o in catalog().iter().filter(|o| {
        o.control_plane()
            && !ADMINISTRATOR_OPS.iter().any(|(n, _)| *n == o.name)
            && !DEVICE_OPS.contains(&o.name)
    }) {
        assert!(
            o.permission().is_some_and(|p| SERVICE_CEILING.contains(&p)),
            "{} needs {:?}",
            o.name,
            o.requires()
        );
    }
}

#[test]
fn administrator_operations_require_what_no_other_level_holds() {
    let all = catalog();
    for (name, _) in ADMINISTRATOR_OPS {
        let o = all
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the catalog"));
        let p = o
            .permission()
            .unwrap_or_else(|| panic!("{name} requires no permission"));
        assert!(
            !SERVICE_CEILING.contains(&p) && !STANDARD_CEILING.contains(&p),
            "{name} requires {p}, which a lower level holds"
        );
    }
}

#[test]
fn a_capability_never_answers_for_the_administrator() {
    for o in catalog() {
        if matches!(o.requires(), Requires::Capability { .. }) {
            assert_eq!(
                o.asks(),
                Asks::None,
                "{} asks, yet a capability runs it",
                o.name
            );
            assert!(
                !ADMINISTRATOR_OPS.iter().any(|(n, _)| *n == o.name),
                "{} is an Administrator operation and a capability",
                o.name
            );
        }
    }
}

#[test]
fn device_door_operations_need_own_or_a_capability() {
    for o in catalog()
        .iter()
        .filter(|o| o.doors().contains(&Door::Device))
    {
        assert!(
            matches!(o.requires(), Requires::Own | Requires::Capability { .. }),
            "{} is on the device door but needs {:?}",
            o.name,
            o.requires()
        );
    }
}

#[test]
fn every_operation_names_its_door() {
    for o in catalog() {
        if crate::wire::DEVICE_OPS.contains(&o.name) {
            assert_eq!(
                o.doors(),
                [Door::Device],
                "{} is not on the device door alone",
                o.name
            );
        } else {
            assert!(o.doors().contains(&Door::Access), "{} lacks Access", o.name);
        }
    }
}

#[test]
fn sites_publish_and_infra_apply_need_platform() {
    for name in ["sites.publish", "infra.apply"] {
        let o = catalog()
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the catalog"));
        assert_eq!(
            o.permission(),
            Some("platform"),
            "{} needs {:?}",
            o.name,
            o.requires()
        );
    }
}

#[test]
fn forms_are_reads() {
    for o in catalog() {
        if o.view == View::Form {
            assert_eq!(
                o.effect,
                Effect::Read,
                "{} answers a form, so it only reads; see the feed-view ADR's #36 amendment",
                o.name
            );
        }
    }
}

#[test]
fn writes_are_cards_or_details() {
    for o in catalog() {
        if o.effect != Effect::Read {
            assert!(
                matches!(o.view, View::Card | View::Detail),
                "{} is a write with view {:?}",
                o.name,
                o.view
            );
        }
    }
}

#[test]
fn every_operation_the_control_plane_routes_runs_there() {
    for o in catalog() {
        let request = super::Request {
            operation: o.name.to_string(),
            args: serde_json::Map::new(),
        };
        if crate::control::route(&request, "test", 0).is_ok() {
            assert!(
                o.control_plane(),
                "{} is routed by the control plane but not marked as running there",
                o.name
            );
        }
    }
}
