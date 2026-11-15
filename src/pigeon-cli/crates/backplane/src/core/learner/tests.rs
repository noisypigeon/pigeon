use super::*;

fn form(number: u32, n: u32) -> Form {
    Form {
        number,
        question_ids: (1..=n).collect(),
        key: (0..n).map(|i| u8::try_from(i % 4).unwrap()).collect(),
        circles: vec![vec![Circle {
            question: 0,
            choice: 0,
            x: 12.5,
            y: 40.25,
        }]],
    }
}

fn attempt(id: &str, f: &Form, page: u32, marks: Vec<Vec<u8>>) -> Attempt {
    let (ids, _) = page_slice(f, page);
    Attempt {
        id: id.into(),
        form: f.number,
        page,
        date: 1_794_700_000,
        question_ids: ids.to_vec(),
        marks,
        results: vec![MarkResult::Correct; ids.len()],
    }
}

#[test]
fn the_apps_json_reads_as_it_writes_it() {
    let f: Form =
        serde_json::from_str(r#"{"number":17,"questionIDs":[4,9],"key":[2,0],"circles":[[{"question":0,"choice":1,"x":10.5,"y":20}]]}"#)
            .unwrap();
    assert_eq!(f.question_ids, [4, 9]);
    let a: Attempt = serde_json::from_str(
        r#"{"id":"6F1C-AB","form":17,"page":0,"date":5,"questionIDs":[4,9],"marks":[[2],[]],"results":["correct","blank"]}"#,
    )
    .unwrap();
    assert_eq!(a.results, [MarkResult::Correct, MarkResult::Blank]);
}

#[test]
fn score_is_the_apps() {
    let r = score(&[0, 1, 2, 3], &[vec![0], vec![2], vec![], vec![1, 3]]);
    assert_eq!(
        r,
        [
            MarkResult::Correct,
            MarkResult::Wrong,
            MarkResult::Blank,
            MarkResult::Double
        ]
    );
    assert_eq!(
        score(&[1, 1], &[vec![1]]),
        [MarkResult::Correct, MarkResult::Blank]
    );
}

#[test]
fn a_sync_holds_everything_and_rescoring_overrules_the_sender() {
    let f = form(17, 30);
    let a0 = attempt(
        "a0",
        &f,
        0,
        (0..25)
            .map(|i| vec![u8::try_from(i % 4).unwrap()])
            .collect(),
    );
    let a1 = attempt("a1", &f, 1, vec![vec![3]; 5]);
    let p = plan(
        &Index::default(),
        Sync {
            forms: vec![f.clone()],
            attempts: vec![a0, a1],
        },
        &BTreeMap::new(),
    );
    assert!(p.refused.is_empty(), "{:?}", p.refused);
    assert_eq!(p.forms, [f]);
    assert_eq!(p.attempts[0].correct(), 25);
    assert_eq!(p.attempts[1].question_ids, [26, 27, 28, 29, 30]);
    assert_eq!(p.attempts[1].correct(), 1);
    let ans = answer(&p.index, &p.refused);
    assert_eq!(ans["forms"], serde_json::json!([17]));
    assert_eq!(ans["attempts"], serde_json::json!(["a0", "a1"]));
}

#[test]
fn a_sync_is_idempotent() {
    let f = form(17, 25);
    let a = attempt("a0", &f, 0, vec![vec![0]; 25]);
    let s = Sync {
        forms: vec![f.clone()],
        attempts: vec![a],
    };
    let first = plan(&Index::default(), s.clone(), &BTreeMap::new());
    let again = plan(&first.index, s, &BTreeMap::from([(17, f)]));
    assert!(again.refused.is_empty(), "{:?}", again.refused);
    assert!(again.forms.is_empty() && again.attempts.is_empty());
    assert_eq!(again.index, first.index);
}

#[test]
fn other_content_under_a_held_name_is_refused_by_name() {
    let f = form(17, 25);
    let a = attempt("a0", &f, 0, vec![vec![0]; 25]);
    let first = plan(
        &Index::default(),
        Sync {
            forms: vec![f.clone()],
            attempts: vec![a.clone()],
        },
        &BTreeMap::new(),
    );
    let mut other_form = f.clone();
    other_form.key[0] = 3;
    let mut other_attempt = a;
    other_attempt.marks[0] = vec![1];
    let p = plan(
        &first.index,
        Sync {
            forms: vec![other_form],
            attempts: vec![other_attempt],
        },
        &BTreeMap::from([(17, f)]),
    );
    assert_eq!(
        p.refused,
        [
            "form 17: held with other content",
            "attempt a0: held with other content"
        ]
    );
    assert_eq!(p.index, first.index);
}

#[test]
fn an_attempt_needs_its_form_and_its_page() {
    let f = form(17, 25);
    let mut a = attempt("a0", &f, 0, vec![]);
    let p = plan(
        &Index::default(),
        Sync {
            forms: vec![],
            attempts: vec![a.clone()],
        },
        &BTreeMap::new(),
    );
    assert_eq!(p.refused, ["attempt a0: form 17 is not held"]);
    a.question_ids.swap(0, 1);
    let p = plan(
        &Index::default(),
        Sync {
            forms: vec![f],
            attempts: vec![a],
        },
        &BTreeMap::new(),
    );
    assert_eq!(
        p.refused,
        ["attempt a0: its questions aren't page 0 of form 17"]
    );
}

#[test]
fn the_sample_and_bad_ids_are_never_kept() {
    let p = plan(
        &Index::default(),
        Sync {
            forms: vec![form(SAMPLE_FORM, 25)],
            attempts: vec![attempt("../x", &form(17, 25), 0, vec![])],
        },
        &BTreeMap::new(),
    );
    assert_eq!(p.refused.len(), 2);
    assert!(p.index.forms.is_empty() && p.index.attempts.is_empty());
}

#[test]
fn progress_counts_each_question() {
    let f = form(17, 2);
    let mut a = attempt("a0", &f, 0, vec![vec![0], vec![0]]);
    a.results = score(&f.key, &a.marks);
    let mut b = a.clone();
    b.id = "a1".into();
    b.date += 60;
    b.results = vec![MarkResult::Correct, MarkResult::Correct];
    let p = progress_of(&[a, b.clone()]);
    assert_eq!(
        p[&1],
        Tally {
            seen: 2,
            right: 2,
            wrong: 0,
            last: b.date
        }
    );
    assert_eq!(
        p[&2],
        Tally {
            seen: 2,
            right: 1,
            wrong: 1,
            last: b.date
        }
    );
}

#[test]
fn the_index_rebuilds_from_what_is_stored() {
    let f = form(17, 25);
    let s = Sync {
        forms: vec![f.clone()],
        attempts: vec![attempt("a0", &f, 0, vec![vec![0]; 25])],
    };
    let p = plan(&Index::default(), s, &BTreeMap::new());
    assert_eq!(Index::of(&p.forms, &p.attempts), p.index);
}

#[test]
fn sealing_round_trips_and_needs_the_key() {
    let key = sealing_key(&"ab".repeat(32)).unwrap();
    let plain = serde_json::to_vec(&form(17, 25)).unwrap();
    let sealed = seal(&key, [7; 12], &plain).unwrap();
    assert_ne!(&sealed[12..], plain.as_slice());
    assert_eq!(open(&key, &sealed).unwrap(), plain);
    let other = sealing_key(&"cd".repeat(32)).unwrap();
    assert!(open(&other, &sealed).is_err());
    let mut tampered = sealed;
    tampered[20] ^= 1;
    assert!(open(&key, &tampered).is_err());
    assert!(sealing_key("short").is_err());
}

#[test]
fn the_sealing_key_is_hkdf_sha256_with_the_learner_info() {
    let ikm = [0x0bu8; 32];
    let mut want = [0u8; 32];
    hkdf::Hkdf::<Sha256>::new(None, &ikm)
        .expand(b"pigeon-learner-v1", &mut want)
        .unwrap();
    assert_eq!(sealing_key(&"0b".repeat(32)).unwrap(), want);
}
