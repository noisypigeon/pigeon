//! State in R2, against an in-process fake S3 server; see the phase 3 amendment §4 of
//! `.claude/decisions/2026-11-04-development-platform-instance-separation.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use cobot::s3::{Bucket, Credentials};
use cobot::state::{Mirror, Store};
use tempfile::TempDir;

type Objects = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

fn etag(body: &[u8]) -> String {
    cobot::s3::md5_hex(body)
}

fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("hex");
            out.push(u8::from_str_radix(hex, 16).expect("hex"));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf8")
}

/// A fake S3 bucket `b` on 127.0.0.1: listing, `GET` with `ETag`, conditional `PUT`, `DELETE`.
fn serve() -> (String, Objects) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let endpoint = format!("http://{}", listener.local_addr().expect("addr"));
    let objects: Objects = Arc::default();
    let store = Arc::clone(&objects);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("request line");
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let mut headers = BTreeMap::new();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).expect("header");
                    let h = h.trim_end();
                    if h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let len: usize = headers
                    .get("content-length")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).expect("body");
                let (path, query) = target.split_once('?').unwrap_or((target, ""));
                let key = decode(path.trim_start_matches("/b").trim_start_matches('/'));
                let mut objects = store.lock().expect("lock");
                let (status, out, tag): (u16, Vec<u8>, Option<String>) = match method {
                    "GET" if query.contains("list-type=2") => {
                        let contents = objects.iter().fold(String::new(), |mut out, (k, v)| {
                            let _ = write!(
                                out,
                                "<Contents><Key>{k}</Key><ETag>&quot;{}&quot;</ETag></Contents>",
                                etag(v)
                            );
                            out
                        });
                        (200, format!("<ListBucketResult><IsTruncated>false</IsTruncated>{contents}</ListBucketResult>").into_bytes(), None)
                    }
                    "GET" => match objects.get(&key) {
                        Some(v) => (200, v.clone(), Some(etag(v))),
                        None => (404, Vec::new(), None),
                    },
                    "PUT" => {
                        let current = objects.get(&key).map(|v| etag(v));
                        let refused = match (headers.get("if-none-match"), headers.get("if-match"))
                        {
                            (Some(_), _) => current.is_some(),
                            (_, Some(want)) => current.as_deref() != Some(want.trim_matches('"')),
                            _ => false,
                        };
                        if refused {
                            (412, Vec::new(), None)
                        } else {
                            let t = etag(&body);
                            objects.insert(key, body);
                            (200, Vec::new(), Some(t))
                        }
                    }
                    "DELETE" => {
                        let current = objects.get(&key).map(|v| etag(v));
                        match headers.get("if-match") {
                            Some(want) if current.as_deref() != Some(want.trim_matches('"')) => {
                                (412, Vec::new(), None)
                            }
                            _ => {
                                objects.remove(&key);
                                (204, Vec::new(), None)
                            }
                        }
                    }
                    _ => (400, Vec::new(), None),
                };
                drop(objects);
                let mut stream = stream;
                let tag = tag
                    .map(|t| format!("etag: \"{t}\"\r\n"))
                    .unwrap_or_default();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\n{tag}content-length: {}\r\nconnection: close\r\n\r\n",
                    out.len()
                );
                let _ = stream.write_all(&out);
            });
        }
    });
    (endpoint, objects)
}

fn bucket(endpoint: &str) -> Bucket {
    Bucket {
        endpoint: endpoint.into(),
        name: "b".into(),
        creds: Credentials {
            access_key: "id".into(),
            secret_key: "s3cret".into(),
        },
    }
}

fn put(objects: &Objects, key: &str, body: &[u8]) {
    objects
        .lock()
        .expect("lock")
        .insert(key.into(), body.to_vec());
}

fn get(objects: &Objects, key: &str) -> Option<Vec<u8>> {
    objects.lock().expect("lock").get(key).cloned()
}

#[test]
fn stores_parse_without_echoing_values() {
    let none = |_: &str| None;
    assert!(matches!(Store::parse("../state", none), Ok(Store::Dir(_))));
    let vars = |k: &str| {
        Some(if k == cobot::state::ENDPOINT_VAR {
            "https://r2.example".into()
        } else {
            format!("value-of-{k}")
        })
    };
    match Store::parse("r2://pigeon-state-x1/instances/noisypigeon/", vars).expect("r2") {
        Store::R2 { bucket, prefix } => {
            assert_eq!(bucket.name, "pigeon-state-x1");
            assert_eq!(prefix, "instances/noisypigeon");
        }
        Store::Dir(_) => panic!("not a dir"),
    }
    for insecure in [
        "http://example.com",
        "http://user@127.0.0.1",
        "ftp://127.0.0.1",
    ] {
        let err = Store::parse("r2://b/p", |k: &str| {
            Some(if k == cobot::state::ENDPOINT_VAR {
                insecure.into()
            } else {
                "v".into()
            })
        })
        .expect_err(insecure);
        assert!(err.contains("https://"), "{err}");
    }
    let missing = Store::parse("r2://b/p", none).expect_err("no endpoint");
    assert!(missing.contains(cobot::state::ENDPOINT_VAR), "{missing}");
    for bad in ["r2://b", "r2://b/", "r2://b/../x", "r2://b/a//c", "r2:///p"] {
        assert!(Store::parse(bad, vars).is_err(), "{bad}");
    }
}

#[test]
fn a_mirror_round_trips_changes_and_skips_other_prefixes() {
    let (endpoint, objects) = serve();
    put(&objects, "p/infra/cloudflare/terraform.tfstate", b"one");
    put(&objects, "p/infra/old/terraform.tfstate", b"old");
    put(&objects, "other/x/terraform.tfstate", b"not mine");
    let dir = TempDir::new().expect("tempdir");
    let mut mirror = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("pull");
    let m = mirror.dir().to_path_buf();
    assert_eq!(
        fs::read(m.join("infra/cloudflare/terraform.tfstate")).expect("read"),
        b"one"
    );
    assert!(!m.join("x").exists());

    assert_eq!(mirror.push(0).expect("no-op push"), Vec::<String>::new());
    fs::write(m.join("infra/cloudflare/terraform.tfstate"), b"two").expect("write");
    fs::create_dir_all(m.join("workloads/w")).expect("mkdir");
    fs::write(
        m.join("workloads/w/terraform.tfstate"),
        [0_u8, 159, 146, 150],
    )
    .expect("write");
    fs::remove_file(m.join("infra/old/terraform.tfstate")).expect("rm");
    let mut written = mirror.push(0).expect("push");
    written.sort();
    assert_eq!(
        written,
        [
            "p/infra/cloudflare/terraform.tfstate",
            "p/infra/old/terraform.tfstate",
            "p/workloads/w/terraform.tfstate"
        ]
    );
    assert_eq!(
        get(&objects, "p/infra/cloudflare/terraform.tfstate").as_deref(),
        Some(&b"two"[..])
    );
    assert_eq!(
        get(&objects, "p/workloads/w/terraform.tfstate").as_deref(),
        Some(&[0_u8, 159, 146, 150][..])
    );
    assert_eq!(
        get(&objects, "p/infra/old/terraform.tfstate").as_deref(),
        Some(cobot::state::TOMBSTONE),
        "removal overwrites with a tombstone, never a DELETE"
    );
    assert_eq!(
        get(&objects, "other/x/terraform.tfstate").as_deref(),
        Some(&b"not mine"[..])
    );

    let mut again =
        Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("re-pull");
    assert!(
        !again.dir().join("infra/old").exists(),
        "a tombstone pulls as absent"
    );
    fs::create_dir_all(again.dir().join("infra/old")).expect("mkdir");
    fs::write(again.dir().join("infra/old/terraform.tfstate"), b"back").expect("write");
    assert_eq!(
        again.push(0).expect("re-create over a tombstone"),
        ["p/infra/old/terraform.tfstate"]
    );
    assert_eq!(
        get(&objects, "p/infra/old/terraform.tfstate").as_deref(),
        Some(&b"back"[..])
    );
    assert_eq!(
        fs::read(again.dir().join("workloads/w/terraform.tfstate")).expect("binary"),
        [0_u8, 159, 146, 150]
    );
}

#[test]
fn a_mirror_pushes_again_after_its_own_writes() {
    let (endpoint, objects) = serve();
    put(&objects, "p/u/terraform.tfstate", b"one");
    let dir = TempDir::new().expect("tempdir");
    let mut mirror = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("pull");
    let m = mirror.dir().to_path_buf();
    let state = m.join("u/terraform.tfstate");
    let fresh = m.join("n/terraform.tfstate");

    fs::write(&state, b"two").expect("write");
    assert_eq!(mirror.push(0).expect("first"), ["p/u/terraform.tfstate"]);
    fs::create_dir_all(m.join("n")).expect("mkdir");
    fs::write(&fresh, b"new").expect("write");
    fs::write(&state, b"three").expect("write");
    let mut written = mirror.push(0).expect("second, over its own write");
    written.sort();
    assert_eq!(written, ["p/n/terraform.tfstate", "p/u/terraform.tfstate"]);
    fs::remove_file(&fresh).expect("rm");
    assert_eq!(
        mirror.push(0).expect("a removal"),
        ["p/n/terraform.tfstate"]
    );
    fs::write(&fresh, b"again").expect("write");
    assert_eq!(
        mirror.push(0).expect("over its own tombstone"),
        ["p/n/terraform.tfstate"]
    );
    assert_eq!(
        get(&objects, "p/u/terraform.tfstate").as_deref(),
        Some(&b"three"[..])
    );
    assert_eq!(
        get(&objects, "p/n/terraform.tfstate").as_deref(),
        Some(&b"again"[..])
    );

    put(&objects, "p/u/terraform.tfstate", b"theirs");
    fs::write(&state, b"four").expect("write");
    let err = mirror.push(0).expect_err("another writer still wins");
    assert!(err.to_string().contains("changed in R2"), "{err}");
    assert_eq!(
        get(&objects, "p/u/terraform.tfstate").as_deref(),
        Some(&b"theirs"[..])
    );
}

#[test]
fn another_writer_wins_and_the_push_fails() {
    let (endpoint, objects) = serve();
    put(&objects, "p/u/terraform.tfstate", b"base");
    let dir = TempDir::new().expect("tempdir");
    let mut mirror = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("pull");
    put(&objects, "p/u/terraform.tfstate", b"theirs");
    fs::write(mirror.dir().join("u/terraform.tfstate"), b"mine").expect("write");
    let err = mirror.push(0).expect_err("conflict").to_string();
    assert!(err.contains("changed in R2"), "{err}");
    assert_eq!(
        get(&objects, "p/u/terraform.tfstate").as_deref(),
        Some(&b"theirs"[..])
    );

    let mut mirror = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("pull");
    put(&objects, "p/new/terraform.tfstate", b"theirs");
    fs::create_dir_all(mirror.dir().join("new")).expect("mkdir");
    fs::write(mirror.dir().join("new/terraform.tfstate"), b"mine").expect("write");
    assert!(
        mirror.push(0).is_err(),
        "a first write never overwrites another's"
    );
}

#[test]
fn a_key_leaving_the_mirror_is_refused() {
    for bad in [
        "p/../escape",
        "p/a\\..\\..\\escape",
        "p/C:/escape",
        "p/a//b",
        "p/./a",
    ] {
        let (endpoint, objects) = serve();
        put(&objects, bad, b"x");
        let dir = TempDir::new().expect("tempdir");
        let err = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect_err(bad);
        assert!(err.to_string().contains("refusing"), "{bad}: {err}");
    }
}

#[test]
fn a_delete_never_erases_another_writers_update() {
    let (endpoint, objects) = serve();
    put(&objects, "p/old/terraform.tfstate", b"base");
    let dir = TempDir::new().expect("tempdir");
    let mut mirror = Mirror::pull(bucket(&endpoint), "p", &dir.path().join("m"), 0).expect("pull");
    put(&objects, "p/old/terraform.tfstate", b"theirs");
    fs::remove_file(mirror.dir().join("old/terraform.tfstate")).expect("rm");
    let err = mirror.push(0).expect_err("conflict").to_string();
    assert!(err.contains("not removing"), "{err}");
    assert_eq!(
        get(&objects, "p/old/terraform.tfstate").as_deref(),
        Some(&b"theirs"[..])
    );
}
