//! The registry: what each operation asks, needs and is reachable through.

use super::{Asks, Door, Effect, GRANT, PAGE, catalog};
use crate::core::approval::grants;
use crate::core::member::{CAPABILITIES, SERVICE_CEILING};

#[test]
fn every_grant_and_page_name_is_in_the_catalog() {
    for name in GRANT.iter().chain(PAGE.iter()) {
        assert!(
            catalog().iter().any(|o| o.name == *name),
            "{name} is not in the catalog"
        );
    }
}

#[test]
fn the_grant_set_is_grants_operations() {
    let mut got: Vec<&str> = catalog()
        .iter()
        .filter(|o| o.asks() == Asks::Grant)
        .map(|o| o.name)
        .collect();
    let mut want: Vec<&str> = grants::OPERATIONS.to_vec();
    got.sort_unstable();
    want.sort_unstable();
    assert_eq!(got, want);
}

#[test]
fn no_operation_is_both_grant_and_page() {
    for name in GRANT {
        assert!(!PAGE.contains(&name), "{name} is both grant and page");
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
        assert_eq!(o.capability(), "read", "{}", o.name);
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
fn every_operation_has_the_access_door() {
    for o in catalog() {
        assert!(o.doors().contains(&Door::Access), "{}", o.name);
    }
}

#[test]
fn sites_publish_and_infra_apply_need_platform() {
    for name in ["sites.publish", "infra.apply"] {
        let o = catalog()
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the catalog"));
        assert_eq!(o.capability(), "platform", "{name}");
    }
}
