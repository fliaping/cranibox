//! Chinese provider compatibility. WS3.
// Ported from yetone/magpie internal/gateway/{kimi_schema,deepseek_schema}.go
// and internal/provider/zhipu_envelope.go@865b61808690bc9fd716257949414ec97618a5a5 (MIT).
use cranibox_contracts::llm::{Preset, UpstreamError};
use serde_json::{Value, json};

pub fn normalize(preset: Option<&Preset>, body: &mut Value) -> Result<(), UpstreamError> {
    let Some(preset) = preset else { return Ok(()) };
    if preset.id.starts_with("kimi") || preset.id.starts_with("moonshot") {
        normalize_tools(body);
        let locked = body.get("model").and_then(Value::as_str).is_some_and(|m| {
            m.starts_with("kimi-for-coding")
                || m.starts_with("kimi-k2.5")
                || m.starts_with("kimi-k2.6")
                || m.starts_with("kimi-k3")
                || m.starts_with("k3")
        });
        if locked && let Some(map) = body.as_object_mut() {
            map.remove("temperature");
            map.remove("top_p");
        }
    }
    if preset.id == "deepseek" {
        normalize_tools(body);
    }
    Ok(())
}
fn normalize_tools(body: &mut Value) {
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if let Some(schema) = tool.get_mut("input_schema") {
                infer_schema(schema);
            }
            if let Some(schema) = tool.pointer_mut("/function/parameters") {
                infer_schema(schema);
            }
        }
    }
}
fn infer_schema(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    if !map.contains_key("type") {
        let vals = map
            .get("enum")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| map.get("const").map(|v| vec![v.clone()]));
        if let Some(vals) = vals.filter(|v| !v.is_empty()) {
            let typ = if vals.iter().all(Value::is_string) {
                Some("string")
            } else if vals.iter().all(Value::is_boolean) {
                Some("boolean")
            } else if vals.iter().all(Value::is_number) {
                Some(if vals.iter().all(|v| v.is_i64() || v.is_u64()) {
                    "integer"
                } else {
                    "number"
                })
            } else {
                None
            };
            if let Some(typ) = typ {
                map.insert("type".into(), json!(typ));
            }
        }
    }
    // Descend only through JSON Schema positions, never enum/default/example instance data.
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(children) = map.get_mut(key).and_then(Value::as_object_mut) {
            for schema in children.values_mut() {
                infer_schema(schema);
            }
        }
    }
    for key in [
        "items",
        "additionalProperties",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "propertyNames",
    ] {
        if let Some(schema) = map.get_mut(key) {
            if let Some(items) = schema.as_array_mut() {
                for schema in items {
                    infer_schema(schema);
                }
            } else {
                infer_schema(schema);
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(items) = map.get_mut(key).and_then(Value::as_array_mut) {
            for schema in items {
                infer_schema(schema);
            }
        }
    }
}
pub fn envelope_error(status: u16, retry: Option<u64>, value: &Value) -> UpstreamError {
    let code = value
        .get("code")
        .or_else(|| value.pointer("/error/code"))
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_default();
    let mapped = match code.parse::<u32>().unwrap_or(0) {
        1000..=1005 => 401,
        1113 | 1302..=1399 => 429,
        1220 => 403,
        1210..=1229 | 1261 | 1301 => 400,
        _ => {
            if status >= 400 {
                status
            } else {
                502
            }
        }
    };
    super::apikey::status_error(mapped, retry, Some(value))
}
