//! SHA-256 and canonical JSON, for the stage dependency hashes.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The JSON text of `v` with object keys sorted, no whitespace: the same value always gives the
/// same text, whatever order its keys were written in.
pub fn canonical(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", Value::String(k.clone()), canonical(&map[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// SHA-256 of the canonical text of `v`.
pub fn hash_value(v: &Value) -> String {
    sha256_hex(canonical(v).as_bytes())
}

#[cfg(test)]
mod tests {
    //! Label: sourced - FIPS 180-4 test vectors ("abc" and the empty string)
    use super::*;
    use serde_json::json;

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn canonical_json_ignores_key_order() {
        let a = json!({"b": [1, {"y": 2, "x": 1}], "a": null});
        let b = json!({"a": null, "b": [1, {"x": 1, "y": 2}]});
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(hash_value(&a), hash_value(&b));
        assert_ne!(hash_value(&a), hash_value(&json!({"a": null})));
    }
}
