//! Canonical JSON used for every signature in C1 (roster) and C7 (vault).
//!
//! Object keys are sorted bytewise at every level and no whitespace is
//! emitted, independent of `serde_json` features enabled elsewhere in the
//! workspace (for example `preserve_order`).
use serde::Serialize;
use serde_json::Value;

/// Serialize `value` as canonical JSON bytes.
pub fn to_vec<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let value = serde_json::to_value(value)?;
    let mut out = Vec::new();
    write(&value, &mut out)?;
    Ok(out)
}

fn write(value: &Value, out: &mut Vec<u8>) -> serde_json::Result<()> {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(&serde_json::to_vec(key)?);
                out.push(b':');
                write(&map[key], out)?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, out)?;
            }
            out.push(b']');
        }
        other => out.extend_from_slice(&serde_json::to_vec(other)?),
    }
    Ok(())
}

/// `domain ‖ canonical_json(value)`: the exact bytes that are signed.
pub fn signing_message<T: Serialize>(domain: &[u8], value: &T) -> serde_json::Result<Vec<u8>> {
    let mut message = domain.to_vec();
    message.extend_from_slice(&to_vec(value)?);
    Ok(message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_are_sorted_recursively() {
        let value = serde_json::json!({"b":1,"a":{"d":[{"z":1,"y":2}],"c":null}});
        assert_eq!(
            super::to_vec(&value).unwrap(),
            br#"{"a":{"c":null,"d":[{"y":2,"z":1}]},"b":1}"#
        );
    }
}
