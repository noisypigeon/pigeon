//! The neutral names: the schema extension's key, the ciphertext tag and the work directory; see
//! `.claude/decisions/2026-11-16-development-src-crates-neutral-names.md` §5.

use serde_json::Value;

/// The schema extension's key.
pub const SCHEMA_KEY: &str = "x-pigeon";

/// `schema`'s extension, `Null` when it has none.
#[must_use]
pub fn ext(schema: &Value) -> &Value {
    schema.get(SCHEMA_KEY).unwrap_or(&Value::Null)
}

/// The ciphertext tag.
pub const ENC_PREFIX: &str = "ENC[pigeon:v1:";

/// The ciphertext body after [`ENC_PREFIX`].
#[must_use]
pub fn enc_body(text: &str) -> Option<&str> {
    text.strip_prefix(ENC_PREFIX)
}

/// The work directory at the root; git-ignored.
pub const WORK_DIR: &str = ".pigeon";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_x_pigeon_is_read() {
        assert_eq!(
            ext(&json!({"x-pigeon": {"local": true}}))["local"],
            json!(true)
        );
        assert!(ext(&json!({"x-cobot": {"local": true}})).is_null());
        assert!(ext(&json!({})).is_null());
    }

    #[test]
    fn only_the_pigeon_ciphertext_tag_reads() {
        assert_eq!(enc_body("ENC[pigeon:v1:abc]"), Some("abc]"));
        assert_eq!(enc_body("ENC[cobot:v1:abc]"), None);
        assert_eq!(enc_body("ENC[gantry:v1:abc]"), None);
    }
}
