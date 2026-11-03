//! The S3 calls `publish` needs, signed with AWS Signature Version 4; see
//! `.claude/decisions/2026-11-03-runtime-cloudflare-static-sites-r2.md` §7.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// R2's signing region.
pub const REGION: &str = "auto";
/// The signing service.
pub const SERVICE: &str = "s3";

/// An access key pair; the secret never appears in `Debug` output.
#[derive(Clone)]
pub struct Credentials {
    /// The access key id.
    pub access_key: String,
    /// The secret access key.
    pub secret_key: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .finish_non_exhaustive()
    }
}

/// Lowercase hex of `bytes`.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Hex MD5 of `bytes`, as R2 reports a single-part object's `ETag`.
#[must_use]
pub fn md5_hex(bytes: &[u8]) -> String {
    hex(&md5::Md5::digest(bytes))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// RFC 3986 encoding, keeping `/` when `path` is set.
#[must_use]
pub fn uri_encode(text: &str, path: bool) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || (path && b == b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// One request to sign: its method, path, query and headers, and the hex
/// SHA-256 of its payload.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// `GET`, `PUT` or `DELETE`.
    pub method: &'a str,
    /// The encoded path, from `/`.
    pub path: &'a str,
    /// Query parameters, unencoded.
    pub query: &'a [(&'a str, &'a str)],
    /// Headers to sign, including `host` and `x-amz-date`.
    pub headers: &'a [(&'a str, &'a str)],
    /// The payload's hex SHA-256.
    pub payload_hash: &'a str,
}

/// The canonical query string: each pair encoded, sorted.
fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// The `Authorization` header for `request`, at `amz_date` (`YYYYMMDDTHHMMSSZ`).
#[must_use]
pub fn authorization(
    request: &Request<'_>,
    creds: &Credentials,
    region: &str,
    service: &str,
    amz_date: &str,
) -> String {
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in request.headers {
        headers.insert(
            k.to_ascii_lowercase(),
            v.split_whitespace().collect::<Vec<_>>().join(" "),
        );
    }
    let signed = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_headers = headers.iter().fold(String::new(), |mut out, (k, v)| {
        let _ = writeln!(out, "{k}:{v}");
        out
    });
    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed}\n{}",
        request.method,
        request.path,
        canonical_query(request.query),
        request.payload_hash
    );
    let day = &amz_date[..8];
    let scope = format!("{day}/{region}/{service}/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let key = [day, region, service, "aws4_request"].iter().fold(
        format!("AWS4{}", creds.secret_key).into_bytes(),
        |k, part| hmac(&k, part),
    );
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={}",
        creds.access_key,
        hex(&hmac(&key, &to_sign))
    )
}

/// The `x-amz-date` form of a Unix time.
#[must_use]
pub fn amz_date(unix: u64) -> String {
    let day = crate::generations::date(i64::try_from(unix / 86_400).unwrap_or(0)).replace('-', "");
    let secs = unix % 86_400;
    format!(
        "{day}T{:02}{:02}{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// An object in a listing: its key and its `ETag`, unquoted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The key.
    pub key: String,
    /// The `ETag`, without quotes.
    pub etag: String,
}

/// Why a call failed; it never holds a credential.
#[derive(Debug)]
pub struct Failure(pub String);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A bucket, reached path-style at `endpoint/bucket`.
#[derive(Debug, Clone)]
pub struct Bucket {
    /// `https://<account>.eu.r2.cloudflarestorage.com`, or a test server.
    pub endpoint: String,
    /// The bucket name.
    pub name: String,
    /// The signing keys.
    pub creds: Credentials,
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The text inside each `<tag>…</tag>` of `xml`, in order.
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

/// Parses one `ListObjectsV2` page: its objects, and the continuation token if truncated.
#[must_use]
pub fn parse_listing(xml: &str) -> (Vec<Object>, Option<String>) {
    let objects = elements(xml, "Contents")
        .into_iter()
        .filter_map(|c| {
            let key = unescape(elements(c, "Key").first()?);
            let etag = unescape(elements(c, "ETag").first().copied().unwrap_or_default());
            Some(Object {
                key,
                etag: etag.trim_matches('"').to_string(),
            })
        })
        .collect();
    let truncated = elements(xml, "IsTruncated").first() == Some(&"true");
    let next = truncated
        .then(|| {
            elements(xml, "NextContinuationToken")
                .first()
                .map(|t| unescape(t))
        })
        .flatten();
    (objects, next)
}

impl Bucket {
    fn host(&self) -> String {
        self.endpoint
            .split("://")
            .nth(1)
            .unwrap_or(&self.endpoint)
            .trim_end_matches('/')
            .to_string()
    }

    /// Sends one signed request; the response's status and body.
    fn send(
        &self,
        method: &str,
        key: Option<&str>,
        query: &[(&str, &str)],
        body: &[u8],
        content_type: Option<&str>,
        now: u64,
    ) -> Result<(u16, String), Failure> {
        let path = match key {
            Some(k) => format!("/{}/{}", uri_encode(&self.name, false), uri_encode(k, true)),
            None => format!("/{}", uri_encode(&self.name, false)),
        };
        let host = self.host();
        let date = amz_date(now);
        let hash = sha256_hex(body);
        let mut headers = vec![
            ("host", host.as_str()),
            ("x-amz-content-sha256", hash.as_str()),
            ("x-amz-date", date.as_str()),
        ];
        if let Some(t) = content_type {
            headers.push(("content-type", t));
        }
        let auth = authorization(
            &Request {
                method,
                path: &path,
                query,
                headers: &headers,
                payload_hash: &hash,
            },
            &self.creds,
            REGION,
            SERVICE,
            &date,
        );
        let mut url = format!("{}{path}", self.endpoint.trim_end_matches('/'));
        if !query.is_empty() {
            url.push('?');
            url.push_str(&canonical_query(query));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let fail = |e: ureq::Error| Failure(format!("{method} {path}: {e}"));
        let mut response = match method {
            "GET" => {
                let mut r = agent.get(&url);
                for (k, v) in headers.iter().filter(|(k, _)| *k != "host") {
                    r = r.header(*k, *v);
                }
                r.header("authorization", &auth).call().map_err(fail)?
            }
            "DELETE" => {
                let mut r = agent.delete(&url);
                for (k, v) in headers.iter().filter(|(k, _)| *k != "host") {
                    r = r.header(*k, *v);
                }
                r.header("authorization", &auth).call().map_err(fail)?
            }
            _ => {
                let mut r = agent.put(&url);
                for (k, v) in headers.iter().filter(|(k, _)| *k != "host") {
                    r = r.header(*k, *v);
                }
                r.header("authorization", &auth).send(body).map_err(fail)?
            }
        };
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        Ok((status, text))
    }

    /// Every object in the bucket, through as many pages as it takes.
    ///
    /// # Errors
    ///
    /// A [`Failure`] for a transport error or a status other than 200.
    pub fn list(&self, now: u64) -> Result<Vec<Object>, Failure> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2")];
            if let Some(t) = &token {
                query.push(("continuation-token", t.as_str()));
            }
            let (status, body) = self.send("GET", None, &query, b"", None, now)?;
            if status != 200 {
                return Err(Failure(format!("listing {}: status {status}", self.name)));
            }
            let (objects, next) = parse_listing(&body);
            out.extend(objects);
            match next {
                Some(n) => token = Some(n),
                None => return Ok(out),
            }
        }
    }

    /// Writes one object.
    ///
    /// # Errors
    ///
    /// A [`Failure`] for a transport error or a status other than 200.
    pub fn put(&self, key: &str, body: &[u8], content_type: &str, now: u64) -> Result<(), Failure> {
        match self.send("PUT", Some(key), &[], body, Some(content_type), now)? {
            (200, _) => Ok(()),
            (status, _) => Err(Failure(format!("PUT {key}: status {status}"))),
        }
    }

    /// Deletes one object.
    ///
    /// # Errors
    ///
    /// A [`Failure`] for a transport error or a status other than 200 or 204.
    pub fn delete(&self, key: &str, now: u64) -> Result<(), Failure> {
        match self.send("DELETE", Some(key), &[], b"", None, now)? {
            (200 | 204, _) => Ok(()),
            (status, _) => Err(Failure(format!("DELETE {key}: status {status}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Credentials, Request, amz_date, authorization, parse_listing, sha256_hex};

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// AWS's `get-vanilla` and `get-vanilla-query-order-key-case` suite cases.
    #[test]
    fn signs_the_aws_suite_vectors() {
        let creds = Credentials {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
        };
        let headers = [
            ("Host", "example.amazonaws.com"),
            ("X-Amz-Date", "20150830T123600Z"),
        ];
        let vanilla = Request {
            method: "GET",
            path: "/",
            query: &[],
            headers: &headers,
            payload_hash: EMPTY,
        };
        assert_eq!(
            authorization(&vanilla, &creds, "us-east-1", "service", "20150830T123600Z"),
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        let ordered = Request {
            query: &[("Param2", "value2"), ("Param1", "value1")],
            ..vanilla
        };
        assert!(
            authorization(&ordered, &creds, "us-east-1", "service", "20150830T123600Z").ends_with(
                "Signature=b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500"
            )
        );
    }

    /// The `GET Object` example from AWS's S3 header-signing documentation.
    #[test]
    fn signs_the_s3_get_object_example() {
        let creds = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        };
        let headers = [
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("Range", "bytes=0-9"),
            ("x-amz-content-sha256", EMPTY),
            ("x-amz-date", "20130524T000000Z"),
        ];
        let get = Request {
            method: "GET",
            path: "/test.txt",
            query: &[],
            headers: &headers,
            payload_hash: EMPTY,
        };
        assert!(
            authorization(&get, &creds, "us-east-1", "s3", "20130524T000000Z").ends_with(
                "SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
            )
        );
        assert_eq!(sha256_hex(b""), EMPTY);
    }

    #[test]
    fn dates_and_listings_parse() {
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(amz_date(1_440_938_160), "20150830T123600Z");
        let (objects, next) = parse_listing(
            "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>a&amp;b/index.html</Key><ETag>&quot;abc&quot;</ETag></Contents><Contents><Key>c</Key><ETag>\"def\"</ETag></Contents><NextContinuationToken>t1</NextContinuationToken></ListBucketResult>",
        );
        assert_eq!(objects[0].key, "a&b/index.html");
        assert_eq!(objects[0].etag, "abc");
        assert_eq!(objects[1].etag, "def");
        assert_eq!(next.as_deref(), Some("t1"));
        assert_eq!(parse_listing("<IsTruncated>false</IsTruncated>").1, None);
    }
}
