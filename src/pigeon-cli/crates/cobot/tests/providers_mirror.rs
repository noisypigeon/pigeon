//! The provider mirror a platform job installs from; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use std::path::Path;

use cobot::providers::{mirror_config, mirror_path, package};

#[test]
fn a_package_is_named_and_placed_as_a_packed_mirror_expects() {
    assert_eq!(
        package("cloudflare/cloudflare", "5.28.0"),
        "terraform-provider-cloudflare_5.28.0_linux_amd64.zip"
    );
    assert_eq!(
        mirror_path("hashicorp/random", "3.9.1"),
        "registry.opentofu.org/hashicorp/random/terraform-provider-random_3.9.1_linux_amd64.zip"
    );
}

#[test]
fn the_mirror_serves_only_its_providers_and_the_rest_install_directly() {
    let c = mirror_config(
        Path::new("/m"),
        &["cloudflare/cloudflare".into(), "hashicorp/random".into()],
    );
    let list = "[\"registry.opentofu.org/cloudflare/cloudflare\", \"registry.opentofu.org/hashicorp/random\"]";
    assert!(c.contains("filesystem_mirror {"), "{c}");
    assert!(c.contains("path    = \"/m\""), "{c}");
    assert!(c.contains(&format!("include = {list}")), "{c}");
    assert!(c.contains(&format!("exclude = {list}")), "{c}");
}
