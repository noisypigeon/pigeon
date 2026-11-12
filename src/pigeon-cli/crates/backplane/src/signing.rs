//! The Buildkite runner's half of the signing exchange, as data; see
//! `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md` §6.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The OIDC audience, and the control plane the grant comes from.
pub const PLATFORM: &str = "https://platform.pigeon.dev";
/// The control plane's exchange route.
pub const ROUTE: &str = "/v0/apple/release";
/// The only Buildkite step allowed to ask.
pub const STEP_KEY: &str = "sign";
/// The OIDC token's lifetime, in seconds.
pub const OIDC_LIFETIME: u32 = 300;
/// macOS's own `openssl` (`LibreSSL`), which reads and writes the legacy `.p12` `security` imports.
pub const OPENSSL: &str = "/usr/bin/openssl";
/// macOS's keychain tool.
pub const SECURITY: &str = "/usr/bin/security";
/// Apple's public WWDR G3 intermediate, which the `.p12` leaves out to fit a Worker secret; see the
/// 2026-11-12 amendment to `.claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md`.
pub const INTERMEDIATE_URL: &str = "https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer";
/// Its SHA-256, hex.
pub const INTERMEDIATE_SHA256: &str =
    "dcf21878c77f4198e4b4614f03d696d89c66c66008d4244e1b99161aac91601f";

/// Whether `der` is exactly [`INTERMEDIATE_URL`]'s certificate.
#[must_use]
pub fn intermediate_ok(der: &[u8]) -> bool {
    use sha2::Digest;
    crate::grants::hex(&sha2::Sha256::digest(der)) == INTERMEDIATE_SHA256
}

/// The prefix of every keychain the harness creates.
pub const KEYCHAIN_PREFIX: &str = "pigeon-sign-";

/// The route's answer: the signing identity, its profile and an upload token.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Grant {
    /// The Apple Distribution certificate and key, `.p12`, base64url.
    pub p12: String,
    /// The `.p12`'s password.
    pub p12_password: String,
    /// The App Store provisioning profile, base64url.
    pub profile: String,
    /// The profile's UUID.
    pub profile_uuid: String,
    /// The profile's name, as `ExportOptions.plist` names it.
    pub profile_name: String,
    /// The developer team.
    pub team_id: String,
    /// The app's bundle ID.
    pub bundle_id: String,
    /// An App Store Connect JWT for the upload.
    pub upload_jwt: String,
    /// When the upload JWT expires, in Unix seconds.
    pub expires: u64,
}

impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Grant")
            .field("p12", &"<redacted>")
            .field("p12_password", &"<redacted>")
            .field("profile_uuid", &self.profile_uuid)
            .field("profile_name", &self.profile_name)
            .field("team_id", &"<redacted>")
            .field("bundle_id", &self.bundle_id)
            .field("upload_jwt", &"<redacted>")
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}

impl Grant {
    /// Checks the fields that become paths or plist text.
    ///
    /// # Errors
    ///
    /// When the profile's UUID, its name, the team or the bundle ID is malformed.
    pub fn validate(&self) -> Result<(), String> {
        let uuid = self.profile_uuid.len() == 36
            && self.profile_uuid.char_indices().all(|(i, c)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            });
        if !uuid {
            return Err("the grant's profile UUID is malformed".into());
        }
        let plain = |s: &str, extra: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
        };
        if !plain(&self.profile_name, " ._-") {
            return Err("the grant's profile name is malformed".into());
        }
        if !plain(&self.team_id, "") {
            return Err("the grant's team ID is malformed".into());
        }
        if !plain(&self.bundle_id, ".-") {
            return Err("the grant's bundle ID is malformed".into());
        }
        Ok(())
    }
}

/// Refuses to run outside the `sign` step.
///
/// # Errors
///
/// When `step_key` is absent or isn't [`STEP_KEY`].
pub fn check_step(step_key: Option<&str>) -> Result<(), String> {
    match step_key {
        Some(STEP_KEY) => Ok(()),
        Some(k) => Err(format!("runs only in step `{STEP_KEY}`, not `{k}`")),
        None => Err(format!(
            "runs only in a Buildkite step `{STEP_KEY}`; BUILDKITE_STEP_KEY is unset"
        )),
    }
}

/// `buildkite-agent`'s argv for the job's OIDC token.
#[must_use]
pub fn oidc_argv() -> Vec<String> {
    [
        "buildkite-agent",
        "oidc",
        "request-token",
        "--audience",
        PLATFORM,
        "--lifetime",
        &OIDC_LIFETIME.to_string(),
    ]
    .map(String::from)
    .to_vec()
}

/// A program to run: its argv, and the environment its secrets travel in.
#[derive(Clone, PartialEq, Eq)]
pub struct Cmd {
    /// The program and its arguments; never a long-lived secret.
    pub argv: Vec<String>,
    /// Extra environment, for secrets.
    pub env: Vec<(String, String)>,
}

impl fmt::Debug for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.env.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("Cmd")
            .field("argv", &self.argv)
            .field("env", &names)
            .finish()
    }
}

/// One step of loading or cleaning.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// Write a file, mode `0600`.
    Write(PathBuf, Secret),
    /// Run a program; its failure stops a load.
    Run(Cmd),
    /// Run a program whose failure is only reported: Apple's intermediate, which an image with
    /// Xcode may already hold, so `security` refuses it as a duplicate.
    Try(Cmd),
    /// Remove a file, if it is there.
    Remove(PathBuf),
    /// Remove a directory and what it holds, if it is there.
    RemoveDir(PathBuf),
}

/// File contents that `Debug` never shows.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(pub Vec<u8>);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bytes>", self.0.len())
    }
}

/// Where a load puts things.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Layout {
    /// The harness's own `0700` directory.
    pub dir: PathBuf,
    /// The keychain.
    pub keychain: PathBuf,
    /// The installed profile.
    pub profile: PathBuf,
}

impl Layout {
    /// The layout for `job` under `tmp`, with the profile installed under `home`.
    ///
    /// # Errors
    ///
    /// When `job` holds anything but letters, digits and dashes.
    pub fn new(tmp: &Path, home: &Path, job: &str, profile_uuid: &str) -> Result<Self, String> {
        let dir = Self::dir_for(tmp, job)?;
        Ok(Self {
            keychain: dir.join(format!("{KEYCHAIN_PREFIX}{job}.keychain-db")),
            profile: home
                .join("Library/MobileDevice/Provisioning Profiles")
                .join(format!("{profile_uuid}.mobileprovision")),
            dir,
        })
    }

    /// The harness's directory for `job` under `tmp`.
    ///
    /// # Errors
    ///
    /// When `job` holds anything but letters, digits and dashes.
    pub fn dir_for(tmp: &Path, job: &str) -> Result<PathBuf, String> {
        if job.is_empty() || !job.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("BUILDKITE_JOB_ID is malformed".into());
        }
        Ok(tmp.join(format!("pigeon-signing-{job}")))
    }

    /// `ExportOptions.plist`.
    #[must_use]
    pub fn export_options(&self) -> PathBuf {
        self.dir.join("ExportOptions.plist")
    }

    /// The upload JWT.
    #[must_use]
    pub fn jwt(&self) -> PathBuf {
        self.dir.join("upload.jwt")
    }

    /// What `clean` reads.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    fn p12_in(&self) -> PathBuf {
        self.dir.join("in.p12")
    }

    fn pem(&self) -> PathBuf {
        self.dir.join("identity.pem")
    }

    fn p12_once(&self) -> PathBuf {
        self.dir.join("once.p12")
    }

    fn intermediate(&self) -> PathBuf {
        self.dir.join("AppleWWDRCAG3.cer")
    }
}

/// What `clean` undoes: written before anything else, so a failed load is cleaned too.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct State {
    /// Where the load put things.
    pub layout: Layout,
    /// The user's keychain search list before the load.
    pub search_list: Vec<String>,
}

/// The keychains in `security list-keychains -d user`'s output, less any this harness made.
#[must_use]
pub fn parse_search_list(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.trim().trim_matches('"').to_string())
        .filter(|l| !l.is_empty())
        .filter(|l| {
            !Path::new(l)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(KEYCHAIN_PREFIX))
        })
        .collect()
}

/// `ExportOptions.plist` for a manual App Store Connect export.
#[must_use]
pub fn export_options(grant: &Grant) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>destination</key>
	<string>export</string>
	<key>method</key>
	<string>app-store-connect</string>
	<key>provisioningProfiles</key>
	<dict>
		<key>{bundle}</key>
		<string>{profile}</string>
	</dict>
	<key>signingCertificate</key>
	<string>Apple Distribution</string>
	<key>signingStyle</key>
	<string>manual</string>
	<key>teamID</key>
	<string>{team}</string>
	<key>uploadSymbols</key>
	<true/>
</dict>
</plist>
"#,
        bundle = grant.bundle_id,
        profile = grant.profile_name,
        team = grant.team_id,
    )
}

fn run(argv: &[&str], env: &[(&str, &str)]) -> Step {
    Step::Run(Cmd {
        argv: argv.iter().map(|s| (*s).to_string()).collect(),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    })
}

fn try_run(argv: &[&str], env: &[(&str, &str)]) -> Step {
    match run(argv, env) {
        Step::Run(c) => Step::Try(c),
        other => other,
    }
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap_or_default()
}

/// The steps that load `grant`: the `.p12` re-wrapped under `once`, a keychain under
/// `keychain_password` holding it and Apple's `intermediate`, the profile, `ExportOptions.plist`
/// and the JWT. `p12` and `profile` are the grant's decoded bytes; `intermediate` is checked by
/// [`intermediate_ok`] first. The state file is the caller's, written first.
#[must_use]
pub fn load_plan(
    grant: &Grant,
    state: &State,
    p12: Vec<u8>,
    profile: Vec<u8>,
    intermediate: Vec<u8>,
    keychain_password: &str,
    once: &str,
) -> Vec<Step> {
    let l = &state.layout;
    let (kc, p12_once) = (path(&l.keychain), l.p12_once());
    let mut search = vec![SECURITY, "list-keychains", "-d", "user", "-s", kc];
    search.extend(state.search_list.iter().map(String::as_str));
    let mut steps = rewrap(grant, l, p12, once);
    steps.extend([
        run(
            &[SECURITY, "create-keychain", "-p", keychain_password, kc],
            &[],
        ),
        run(
            &[SECURITY, "set-keychain-settings", "-lut", "1800", kc],
            &[],
        ),
        run(
            &[SECURITY, "unlock-keychain", "-p", keychain_password, kc],
            &[],
        ),
        run(
            &[
                SECURITY,
                "import",
                path(&p12_once),
                "-k",
                kc,
                "-f",
                "pkcs12",
                "-P",
                once,
                "-T",
                "/usr/bin/codesign",
                "-T",
                SECURITY,
            ],
            &[],
        ),
        Step::Remove(p12_once),
        Step::Write(l.intermediate(), Secret(intermediate)),
        try_run(
            &[
                SECURITY,
                "import",
                path(&l.intermediate()),
                "-k",
                kc,
                "-t",
                "cert",
                "-f",
                "x509",
            ],
            &[],
        ),
        Step::Remove(l.intermediate()),
        run(
            &[
                SECURITY,
                "set-key-partition-list",
                "-S",
                "apple-tool:,apple:",
                "-s",
                "-k",
                keychain_password,
                kc,
            ],
            &[],
        ),
        run(&search, &[]),
        Step::Write(l.profile.clone(), Secret(profile)),
        Step::Write(
            l.export_options(),
            Secret(export_options(grant).into_bytes()),
        ),
        Step::Write(l.jwt(), Secret(grant.upload_jwt.clone().into_bytes())),
    ]);
    steps
}

/// The `.p12` re-wrapped under the one-time password `once`, so the long-lived password never
/// reaches an argv; it leaves only `once.p12`.
fn rewrap(grant: &Grant, l: &Layout, p12: Vec<u8>, once: &str) -> Vec<Step> {
    let (p12_in, pem, p12_once) = (l.p12_in(), l.pem(), l.p12_once());
    vec![
        Step::Write(p12_in.clone(), Secret(p12)),
        run(
            &[
                OPENSSL,
                "pkcs12",
                "-in",
                path(&p12_in),
                "-passin",
                "env:PIGEON_P12_IN",
                "-nodes",
                "-out",
                path(&pem),
            ],
            &[("PIGEON_P12_IN", &grant.p12_password)],
        ),
        Step::Remove(p12_in),
        run(
            &[
                OPENSSL,
                "pkcs12",
                "-export",
                "-in",
                path(&pem),
                "-passout",
                "env:PIGEON_P12_ONCE",
                "-out",
                path(&p12_once),
            ],
            &[("PIGEON_P12_ONCE", once)],
        ),
        Step::Remove(pem),
    ]
}

/// The steps that undo a load; each may fail without stopping the rest.
#[must_use]
pub fn clean_plan(state: &State) -> Vec<Step> {
    let l = &state.layout;
    let mut search = vec![SECURITY, "list-keychains", "-d", "user", "-s"];
    search.extend(state.search_list.iter().map(String::as_str));
    vec![
        run(&search, &[]),
        run(&[SECURITY, "delete-keychain", path(&l.keychain)], &[]),
        Step::Remove(l.profile.clone()),
        Step::RemoveDir(l.dir.clone()),
    ]
}

/// `iTMSTransporter`'s argv to upload `ipa` with `jwt`.
#[must_use]
pub fn upload_argv(ipa: &Path, jwt: &str) -> Vec<String> {
    [
        "xcrun",
        "iTMSTransporter",
        "-m",
        "upload",
        "-assetFile",
        path(ipa),
        "-jwt",
        jwt,
    ]
    .map(String::from)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> Grant {
        Grant {
            p12: "cDEy".into(),
            p12_password: "long-lived-p12-password".into(),
            profile: "cHJvZmlsZQ".into(),
            profile_uuid: "0f8fad5b-d9cb-469f-a165-70867728950e".into(),
            profile_name: "Pigeon App Store".into(),
            team_id: "TEAM123456".into(),
            bundle_id: "dev.pigeon.Pigeon".into(),
            upload_jwt: "header.claims.signature".into(),
            expires: 1_794_500_000,
        }
    }

    fn state() -> State {
        let layout = Layout::new(
            Path::new("/tmp"),
            Path::new("/Users/agent"),
            "0190-abcd",
            &grant().profile_uuid,
        )
        .unwrap();
        State {
            layout,
            search_list: vec!["/Users/agent/Library/Keychains/login.keychain-db".into()],
        }
    }

    fn plan() -> Vec<Step> {
        load_plan(
            &grant(),
            &state(),
            b"p12".to_vec(),
            b"profile".to_vec(),
            b"intermediate".to_vec(),
            "kc-once",
            "p12-once",
        )
    }

    #[test]
    fn no_long_lived_secret_is_in_any_argv() {
        let g = grant();
        for step in plan() {
            if let Step::Run(c) = step {
                for a in &c.argv {
                    assert!(!a.contains(&g.p12_password), "{a}");
                    assert!(!a.contains(&g.upload_jwt), "{a}");
                    assert!(!a.contains(&g.p12), "{a}");
                }
            }
        }
    }

    #[test]
    fn every_file_the_load_writes_is_removed_or_cleaned() {
        let s = state();
        let removed: Vec<PathBuf> = plan()
            .into_iter()
            .filter_map(|st| match st {
                Step::Remove(p) => Some(p),
                _ => None,
            })
            .collect();
        let clean = clean_plan(&s);
        for step in plan() {
            if let Step::Write(p, _) = step {
                let cleaned = removed.contains(&p)
                    || p.starts_with(&s.layout.dir)
                    || clean.contains(&Step::Remove(p.clone()));
                assert!(cleaned, "{}", p.display());
            }
        }
        assert!(clean.contains(&Step::RemoveDir(s.layout.dir.clone())));
    }

    #[test]
    fn clean_restores_the_search_list_and_deletes_the_keychain() {
        let s = state();
        let clean = clean_plan(&s);
        let Step::Run(restore) = &clean[0] else {
            panic!("{clean:?}")
        };
        assert_eq!(restore.argv[5..], s.search_list[..]);
        let Step::Run(delete) = &clean[1] else {
            panic!("{clean:?}")
        };
        assert_eq!(
            delete.argv,
            [SECURITY, "delete-keychain", path(&s.layout.keychain)]
        );
    }

    #[test]
    fn the_keychain_leads_the_search_list() {
        let s = state();
        let search = plan()
            .into_iter()
            .find_map(|st| match st {
                Step::Run(c) if c.argv.get(1).is_some_and(|a| a == "list-keychains") => Some(c),
                _ => None,
            })
            .unwrap();
        assert_eq!(search.argv[5], path(&s.layout.keychain));
        assert_eq!(search.argv[6..], s.search_list[..]);
    }

    #[test]
    fn the_search_list_parses_and_drops_old_harness_keychains() {
        let out = "    \"/Users/agent/Library/Keychains/login.keychain-db\"\n    \"/tmp/x/pigeon-sign-old.keychain-db\"\n";
        assert_eq!(
            parse_search_list(out),
            ["/Users/agent/Library/Keychains/login.keychain-db"]
        );
    }

    #[test]
    fn export_options_name_the_profile_team_and_method() {
        let plist = export_options(&grant());
        assert!(
            plist.contains("<key>dev.pigeon.Pigeon</key>\n\t\t<string>Pigeon App Store</string>")
        );
        assert!(plist.contains("<string>app-store-connect</string>"));
        assert!(plist.contains("<string>manual</string>"));
        assert!(plist.contains("<string>TEAM123456</string>"));
    }

    #[test]
    fn debug_shows_no_secret() {
        let g = grant();
        let shown = format!("{g:?} {:?}", plan());
        for secret in [&g.p12, &g.p12_password, &g.upload_jwt, &g.team_id] {
            assert!(!shown.contains(secret.as_str()), "{secret}");
        }
    }

    #[test]
    fn the_intermediate_joins_the_keychain_and_only_apples_is_accepted() {
        let p = plan();
        let at = |pred: &dyn Fn(&Step) -> bool| p.iter().position(pred).expect("a step");
        let leaf = at(
            &|s| matches!(s, Step::Run(c) if c.argv.iter().any(|a| a == "pkcs12") && c.argv[1] == "import"),
        );
        let cert = at(&|s| matches!(s, Step::Try(c) if c.argv.iter().any(|a| a == "x509")));
        assert!(leaf < cert, "the leaf, then its intermediate");
        let Step::Try(c) = &p[cert] else {
            unreachable!()
        };
        assert_eq!(c.argv[1], "import");
        assert!(
            c.argv
                .windows(2)
                .any(|w| w[0] == "-k" && w[1].ends_with(".keychain-db"))
        );
        assert!(
            p.iter()
                .any(|s| matches!(s, Step::Remove(f) if f.ends_with("AppleWWDRCAG3.cer")))
        );
        assert!(!intermediate_ok(b"not apple's certificate"));
        assert_eq!(INTERMEDIATE_SHA256.len(), 64);
    }

    #[test]
    fn only_the_sign_step_may_ask() {
        assert!(check_step(Some("sign")).is_ok());
        assert!(check_step(Some("archive")).is_err());
        assert!(check_step(None).is_err());
    }

    #[test]
    fn malformed_grants_and_jobs_are_refused() {
        let mut g = grant();
        g.profile_uuid = "../../evil".into();
        assert!(g.validate().is_err());
        let mut g = grant();
        g.profile_name = "x</string>".into();
        assert!(g.validate().is_err());
        assert!(grant().validate().is_ok());
        assert!(Layout::new(Path::new("/tmp"), Path::new("/h"), "../x", "u").is_err());
    }

    #[test]
    fn the_oidc_token_is_for_the_platform_and_short() {
        assert_eq!(
            oidc_argv(),
            [
                "buildkite-agent",
                "oidc",
                "request-token",
                "--audience",
                PLATFORM,
                "--lifetime",
                "300"
            ]
        );
    }
}
