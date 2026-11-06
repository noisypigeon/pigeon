//! The git front door's push rule and its answers.

use backplane::changes::{Change, State};
use backplane::gitdoor::{self, RefUpdate, Service};

const A: &str = "1111111111111111111111111111111111111111";
const B: &str = "2222222222222222222222222222222222222222";
const ZERO: &str = "0000000000000000000000000000000000000000";

fn pkt(s: &str) -> String {
    format!("{:04x}{s}", s.len() + 4)
}

fn body(lines: &[String]) -> Vec<u8> {
    let mut b = lines.concat();
    b.push_str("0000PACK…");
    b.into_bytes()
}

fn change(state: State) -> Change {
    let mut c = Change::propose(18, "front-door", "t", "p", vec!["**".into()], "agent");
    c.state = state;
    c
}

fn update(name: &str, new: &str) -> RefUpdate {
    RefUpdate {
        old: ZERO.into(),
        new: new.into(),
        name: name.into(),
    }
}

#[test]
fn paths_name_the_repository_and_service() {
    assert_eq!(
        gitdoor::path("/git/pigeon/pigeon.git/info/refs"),
        Some(("pigeon", "pigeon", Service::InfoRefs))
    );
    assert_eq!(
        gitdoor::path("/git/pigeon/pigeon.git/git-receive-pack"),
        Some(("pigeon", "pigeon", Service::ReceivePack))
    );
    assert_eq!(
        gitdoor::path("/git/ns/r.git/git-upload-pack"),
        Some(("ns", "r", Service::UploadPack))
    );
    for bad in [
        "/git/pigeon/pigeon.git/objects/info/packs",
        "/git/pigeon/pigeon/info/refs",
        "/git//pigeon.git/info/refs",
        "/git/a/b/c.git/info/refs",
        "/mcp",
    ] {
        assert_eq!(gitdoor::path(bad), None, "{bad}");
    }
}

#[test]
fn commands_are_read_up_to_the_flush_with_the_first_lines_capabilities() {
    let push = gitdoor::commands(&body(&[
        pkt(&format!(
            "{ZERO} {A} refs/heads/change/18-front-door\0report-status side-band-64k agent=git/2\n"
        )),
        pkt(&format!("{A} {B} refs/heads/main\n")),
    ]))
    .expect("commands");
    assert_eq!(push.updates.len(), 2);
    assert_eq!(push.updates[0].name, "refs/heads/change/18-front-door");
    assert_eq!(push.updates[1].old, A);
    assert!(push.side_band());
    assert_eq!(
        push.updates[0].change(),
        Some((18, "front-door".to_string()))
    );
    assert_eq!(push.updates[1].change(), None);
    let shallow = gitdoor::commands(&body(&[
        pkt(&format!("shallow {A}")),
        pkt(&format!(
            "{ZERO} {A} refs/heads/change/18-front-door\0report-status"
        )),
    ]))
    .expect("shallow");
    assert!(!shallow.side_band());
    for bad in [
        b"zzzz".to_vec(),
        b"0000".to_vec(),
        body(&[pkt("not a command\n")]),
        body(&[pkt(&format!("{ZERO} {A}\n"))]),
        body(&[pkt(&format!("{ZERO} nope refs/heads/x\n"))]),
        body(&[
            pkt(&format!("{ZERO} {A} refs/heads/a\0caps\n")),
            pkt(&format!("{ZERO} {A} refs/heads/b\0caps\n")),
        ]),
        pkt(&format!("{ZERO} {A} refs/heads/a\n")).into_bytes(),
    ] {
        assert!(gitdoor::commands(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn only_an_authorized_changes_branch_is_pushed() {
    let branch = "refs/heads/change/18-front-door";
    for state in [
        State::Planned,
        State::Pushed,
        State::Refused,
        State::Checked,
        State::Failed,
        State::Approved,
    ] {
        assert_eq!(
            gitdoor::judge(&update(branch, A), Some(&change(state))),
            Ok(()),
            "{state:?}"
        );
    }
    let refused =
        |u: RefUpdate, c: Option<Change>| gitdoor::judge(&u, c.as_ref()).expect_err("refused");
    assert!(refused(update(branch, A), Some(change(State::Proposed))).contains("not approved"));
    assert!(refused(update(branch, A), Some(change(State::Merged))).contains("merged"));
    assert!(refused(update(branch, A), Some(change(State::Closed))).contains("closed"));
    assert!(refused(update(branch, ZERO), Some(change(State::Planned))).contains("deleted"));
    assert!(refused(update(branch, A), None).contains("no change 18"));
    assert!(
        refused(
            update("refs/heads/change/18-other", A),
            Some(change(State::Planned))
        )
        .contains("change/18-front-door")
    );
    let mut other = change(State::Planned);
    other.number = 19;
    assert!(refused(update(branch, A), Some(other)).contains("no change 18"));
    for name in [
        "refs/heads/main",
        "refs/tags/v1",
        "refs/heads/claude/x",
        "change/18-front-door",
        "refs/notes/change/18-front-door",
    ] {
        assert!(
            refused(update(name, A), Some(change(State::Planned))).contains("only change/"),
            "{name}"
        );
    }
}

#[test]
fn a_refusal_reports_every_ref_ng_on_the_side_band_when_asked() {
    let lines = [
        pkt(&format!(
            "{ZERO} {A} refs/heads/change/18-front-door\0report-status\n"
        )),
        pkt(&format!("{A} {B} refs/heads/main\n")),
    ];
    let push = gitdoor::commands(&body(&lines)).expect("commands");
    let why = vec![(
        "refs/heads/main".to_string(),
        "only change/<n>-<slug> branches\nare pushed here".to_string(),
    )];
    let plain = String::from_utf8(gitdoor::refusal(&push, &why)).expect("utf8");
    assert_eq!(
        plain,
        [
            pkt("unpack ok\n"),
            pkt("ng refs/heads/change/18-front-door another ref in this push was refused\n"),
            pkt("ng refs/heads/main only change/<n>-<slug> branches are pushed here\n"),
            "0000".into(),
        ]
        .concat()
    );
    let mut caps = push.clone();
    caps.caps.push("side-band-64k".into());
    let banded = gitdoor::refusal(&caps, &why);
    let inner = plain.as_bytes();
    let len =
        usize::from_str_radix(std::str::from_utf8(&banded[..4]).expect("len"), 16).expect("hex");
    assert_eq!(len, inner.len() + 5);
    assert_eq!(banded[4], 1, "channel 1");
    assert_eq!(&banded[5..len], inner);
    assert_eq!(&banded[len..], b"0000");
}
