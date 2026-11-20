//! The private zone operations: what each asks and requires.

use super::{Asks, Effect, catalog};
use crate::zones_private::{LIST, SET, SHOW};

fn op(name: &str) -> &'static super::Operation {
    catalog()
        .iter()
        .find(|o| o.name == name)
        .expect("in the catalog")
}

#[test]
fn setting_a_private_zone_asks_for_its_page() {
    let set = op(SET);
    assert_eq!(set.asks(), Asks::Page);
    assert_eq!(set.effect, Effect::Plan);
    assert_eq!(set.permission(), Some("propose"));
    assert!(set.control_plane());
    let routed = crate::control::route(
        &super::Request {
            operation: SET.into(),
            args: serde_json::Map::new(),
        },
        "test",
        0,
    );
    assert!(matches!(
        routed,
        Ok(crate::control::Routed::ZonePrivate { .. })
    ));
}

#[test]
fn showing_and_listing_private_zones_are_reads() {
    for name in [SHOW, LIST] {
        let o = op(name);
        assert_eq!(o.asks(), Asks::None);
        assert_eq!(o.effect, Effect::Read);
        assert_eq!(o.permission(), Some("read"));
        assert!(o.control_plane());
    }
}
