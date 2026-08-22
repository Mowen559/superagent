use serde_json::{Map, Value};
use xxhash_rust::xxh3::xxh3_64;

/// Stable, secret-minimising identity for a tool invocation. Object keys are
/// canonicalised so policy modules agree even when JSON key order differs.
pub fn fingerprint(tool_name: &str, arguments: &Value) -> String {
    let canonical = canonical_json(arguments);
    let payload = format!("{tool_name}\0{canonical}");
    format!("{:016x}", xxh3_64(payload.as_bytes()))
}

fn canonical_json(value: &Value) -> String {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort_unstable();
                let mut sorted = Map::new();
                for key in keys {
                    sorted.insert(key.clone(), canonicalize(&object[key]));
                }
                Value::Object(sorted)
            }
            Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
            other => other.clone(),
        }
    }

    serde_json::to_string(&canonicalize(value)).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fingerprint_is_key_order_independent_and_argument_sensitive() {
        assert_eq!(
            fingerprint("fixture", &json!({ "a": 1, "b": 2 })),
            fingerprint("fixture", &json!({ "b": 2, "a": 1 }))
        );
        assert_ne!(
            fingerprint("fixture", &json!({ "a": 1 })),
            fingerprint("fixture", &json!({ "a": 2 }))
        );
    }
}
