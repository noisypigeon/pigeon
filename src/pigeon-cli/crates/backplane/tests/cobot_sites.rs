//! Publishing a built site: the sync against a local S3 stand-in.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backplane::cobot::s3::{Bucket, Credentials, md5_hex, sha256_hex};
use tempfile::TempDir;

type Store = Arc<Mutex<BTreeMap<String, (Vec<u8>, String)>>>;

fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            out.push(u8::from_str_radix(&path[i + 1..i + 3], 16).expect("hex"));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf-8")
}

/// Serves `bucket` path-style from `store`, two keys per listing page; every
/// request must be signed by `access` and carry its payload's SHA-256.
fn serve(store: Store, bucket: &'static str, access: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let target = parts.next().unwrap_or_default().to_string();
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
            let signed = headers
                .get("authorization")
                .is_some_and(|a| a.starts_with(&format!("AWS4-HMAC-SHA256 Credential={access}/")))
                && headers.get("x-amz-content-sha256") == Some(&sha256_hex(&body));
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let key = path.strip_prefix(&format!("/{bucket}/")).map(decode);
            let (status, reply) = if signed {
                let mut s = store.lock().expect("lock");
                match (method.as_str(), key) {
                    ("PUT", Some(k)) => {
                        let etag = md5_hex(&body);
                        s.insert(k, (body, etag));
                        ("200 OK", String::new())
                    }
                    ("GET", Some(k)) => match s.get(&k) {
                        Some((body, _)) => ("200 OK", String::from_utf8_lossy(body).into_owned()),
                        None => ("404 Not Found", String::new()),
                    },
                    ("DELETE", Some(k)) => {
                        s.remove(&k);
                        ("204 No Content", String::new())
                    }
                    ("GET", None) if query.contains("list-type=2") => {
                        let after = query
                            .split('&')
                            .find_map(|p| p.strip_prefix("continuation-token="))
                            .map(decode);
                        let keys: Vec<&String> = s
                            .keys()
                            .filter(|k| after.as_ref().is_none_or(|a| *k > a))
                            .collect();
                        let page = &keys[..keys.len().min(2)];
                        let mut xml = format!(
                            "<ListBucketResult><IsTruncated>{}</IsTruncated>",
                            keys.len() > 2
                        );
                        for k in page {
                            let _ = write!(
                                xml,
                                "<Contents><Key>{}</Key><ETag>&quot;{}&quot;</ETag></Contents>",
                                k.replace('&', "&amp;"),
                                s[*k].1
                            );
                        }
                        if keys.len() > 2 {
                            let _ = write!(
                                xml,
                                "<NextContinuationToken>{}</NextContinuationToken>",
                                page[1]
                            );
                        }
                        xml.push_str("</ListBucketResult>");
                        ("200 OK", xml)
                    }
                    _ => ("400 Bad Request", String::new()),
                }
            } else {
                ("403 Forbidden", String::new())
            };
            let mut out = stream;
            let _ = write!(
                out,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    format!("http://{addr}")
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

#[test]
fn a_sync_puts_what_changed_writes_index_copies_and_deletes_stale_keys() {
    let public = TempDir::new().expect("public");
    write(public.path(), "index.html", "<html>home</html>");
    write(public.path(), "posts/a/index.html", "<html>a, new</html>");
    write(public.path(), "assets/site.css", "body {}");
    let expected = backplane::cobot::sites::expected(public.path()).expect("walk");
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        [
            "assets/site.css",
            "index.html",
            "posts/a",
            "posts/a/",
            "posts/a/index.html"
        ]
    );
    assert_eq!(expected["posts/a"].content_type, "text/html; charset=utf-8");
    assert_eq!(expected["posts/a/"].body, b"<html>a, new</html>");

    let store: Store = Arc::default();
    {
        let mut s = store.lock().expect("lock");
        s.insert(
            "assets/site.css".into(),
            (b"body {}".to_vec(), md5_hex(b"body {}")),
        );
        s.insert(
            "posts/a/index.html".into(),
            (b"old".to_vec(), md5_hex(b"old")),
        );
        s.insert("stale & gone.html".into(), (b"x".to_vec(), md5_hex(b"x")));
    }
    let bucket = Bucket {
        endpoint: serve(store.clone(), "blog-z-com", "token-id"),
        name: "blog-z-com".into(),
        creds: Credentials {
            access_key: "token-id".into(),
            secret_key: sha256_hex(b"token-value"),
        },
    };
    let done = backplane::cobot::sites::sync(&bucket, &expected).expect("sync");
    assert_eq!(
        done.put,
        [
            "assets/site.css",
            "index.html",
            "posts/a",
            "posts/a/",
            "posts/a/index.html"
        ],
        "with no record of its type, even an unchanged body is rewritten"
    );
    assert_eq!(done.deleted, ["stale & gone.html"]);
    let held: Vec<String> = store.lock().expect("lock").keys().cloned().collect();
    let mut want: Vec<String> = expected.keys().cloned().collect();
    want.insert(0, backplane::cobot::sites::TYPES.to_string());
    assert_eq!(held, want);

    let again = backplane::cobot::sites::sync(&bucket, &expected).expect("resync");
    assert!(again.put.is_empty() && again.deleted.is_empty());
    assert_eq!(again.unchanged, 5);

    {
        let mut s = store.lock().expect("lock");
        let (body, _) = s[backplane::cobot::sites::TYPES].clone();
        let mut types: BTreeMap<String, String> = serde_json::from_slice(&body).expect("json");
        types.insert("assets/site.css".into(), "application/octet-stream".into());
        let body = serde_json::to_vec(&types).expect("json");
        let etag = md5_hex(&body);
        s.insert(backplane::cobot::sites::TYPES.into(), (body, etag));
    }
    let retyped = backplane::cobot::sites::sync(&bucket, &expected).expect("retype");
    assert_eq!(retyped.put, ["assets/site.css"]);

    write(public.path(), backplane::cobot::sites::TYPES, "{}");
    let err = backplane::cobot::sites::expected(public.path())
        .expect_err("reserved")
        .to_string();
    assert!(err.contains("is cobot's own key"), "{err}");

    let wrong = Bucket {
        creds: Credentials {
            access_key: "other".into(),
            secret_key: "s".into(),
        },
        ..bucket
    };
    let err = backplane::cobot::sites::sync(&wrong, &expected)
        .expect_err("refused")
        .to_string();
    assert!(
        err.contains("status 403")
            && !err.contains(&sha256_hex(b"token-value"))
            && !err.contains("blog-z-com"),
        "{err}"
    );
}
