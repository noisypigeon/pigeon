//! The secret names every colour shares through the Secrets Store; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its colour-kind amendment.

/// Each Worker secret a colour reads from the Secrets Store, by its binding name.
pub const SHARED: [&str; 33] = [
    "BACKPLANE_TOKEN",
    "ARTIFACTS_TOKEN",
    "ACCESS_TEAM",
    "ACCESS_AUD",
    "DEPLOY_TOKEN",
    "SESSION_KEY",
    "APPROVER_TOTP",
    "APPROVER",
    "PASSKEY_ENROLL",
    "WORKERS_DEPLOY_TOKEN",
    "DEPLOYER_ACCESS_ID",
    "DEPLOYER_ACCESS_SECRET",
    "BUILDS_ACCESS_ID",
    "BUILDS_ACCESS_SECRET",
    "PUSH_VAPID_KEY",
    "PLATFORM_STATE_KEY",
    "GITHUB_APP_ID",
    "GITHUB_APP_KEY",
    "APPLE_ASC_KEY",
    "APPLE_ASC_KEY_ID",
    "APPLE_ASC_ISSUER",
    "APPLE_TEAM_ID",
    "APPLE_DIST_P12",
    "APPLE_DIST_P12_PASSWORD",
    "BUILDKITE_TOKEN",
    "APNS_KEY",
    "APNS_KEY_ID",
    "BITRISE_TOKEN",
    "BITRISE_APP_SLUG",
    "OBSERVABILITY_TOKEN",
    "POOL_ANTHROPIC_WORKSPACE",
    "POOL_ACCESS_ID",
    "POOL_ACCESS_SECRET",
];

#[cfg(test)]
mod tests {
    use super::SHARED;

    #[test]
    fn shared_names_are_the_control_secrets_less_the_pipelines_own() {
        let spec = include_str!("../../../../infra/pigeon/workers/control/worker.yaml");
        let line = spec
            .lines()
            .find(|l| l.starts_with("secrets: ["))
            .expect("a `secrets: [` line");
        let list = line
            .trim_start_matches("secrets: [")
            .trim_end()
            .trim_end_matches(']');
        let names: Vec<&str> = list
            .split(", ")
            .filter(|n| *n != "DEPLOY_OUTPUTS" && *n != "DEPLOY_IMAGES")
            .collect();
        assert_eq!(names, SHARED);
    }
}
