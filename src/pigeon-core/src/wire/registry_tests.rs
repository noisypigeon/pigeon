//! The registry: what each operation asks, needs and is reachable through.

use super::{Asks, Door, Effect, GRANT, PAGE, catalog};
use crate::approval::grants;
use crate::member::{CAPABILITIES, SERVICE_CEILING};
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
fn every_capability_is_a_member_capability() {
    for o in catalog() {
        assert!(
            CAPABILITIES.contains(&o.capability()),
            "{} needs {}",
            o.name,
            o.capability()
        );
    }
}

#[test]
fn reads_need_read() {
    for o in catalog().iter().filter(|o| o.effect == Effect::Read) {
        assert_eq!(
            o.capability(),
            "read",
            "{} needs {}",
            o.name,
            o.capability()
        );
    }
}

#[test]
fn a_service_account_may_ask_every_control_plane_operation() {
    for o in catalog().iter().filter(|o| o.control_plane()) {
        assert!(
            SERVICE_CEILING.contains(&o.capability()),
            "{} needs {}",
            o.name,
            o.capability()
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
            o.capability(),
            "platform",
            "{} needs {}",
            o.name,
            o.capability()
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
