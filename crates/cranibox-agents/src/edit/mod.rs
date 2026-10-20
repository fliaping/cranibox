//! Format-aware configuration edits. YAML and TOML retain unrelated comments.
use crate::{AgentError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
mod cordis;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum Format {
    Json,
    Json5,
    Toml,
    Yaml,
    Cordis,
    Text,
    Env,
}
pub(crate) fn error(e: impl std::fmt::Display) -> AgentError {
    AgentError::Config(e.to_string())
}
pub(crate) fn parse(text: &str, format: Format) -> Result<Value> {
    if text.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match format {
        Format::Json => serde_json::from_str(text).map_err(error),
        Format::Json5 => json5::from_str(text).map_err(error),
        Format::Yaml => serde_yaml::from_str(text).map_err(error),
        Format::Cordis => cordis::parse(text),
        Format::Text => Ok(serde_json::json!({"content":text})),
        Format::Toml => toml_edit::de::from_str(text).map_err(error),
        Format::Env => Ok(Value::Object(
            text.lines()
                .filter_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (!k.trim().starts_with('#')).then(|| (k.trim().into(), Value::String(v.into())))
                })
                .collect(),
        )),
    }
}
pub(crate) fn get<'a>(v: &'a Value, keys: &[String]) -> Option<&'a Value> {
    keys.iter().try_fold(v, |v, k| v.as_object()?.get(k))
}
pub(crate) fn set(v: &mut Value, keys: &[String], value: Option<Value>) {
    if keys.len() == 1 {
        if let Some(obj) = v.as_object_mut() {
            if let Some(value) = value {
                obj.insert(keys[0].clone(), value);
            } else {
                obj.remove(&keys[0]);
            }
        }
    } else if !keys.is_empty() {
        if v.is_null() {
            *v = serde_json::json!({});
        }
        if let Some(obj) = v.as_object_mut() {
            let child = obj
                .entry(keys[0].clone())
                .or_insert_with(|| serde_json::json!({}));
            set(child, &keys[1..], value);
        }
    }
}
pub(crate) fn render(
    text: &str,
    format: Format,
    keys: &[String],
    value: Option<Value>,
) -> Result<String> {
    match format {
        Format::Yaml => yaml(text, keys, value),
        Format::Cordis => cordis::render(text, keys, value),
        Format::Text => value
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| error("expected text"))
            })
            .transpose()
            .map(|s| s.unwrap_or_default()),
        Format::Toml => {
            let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(error)?;
            fn update(
                table: &mut dyn toml_edit::TableLike,
                keys: &[String],
                value: Option<toml_edit::Item>,
            ) -> Result<()> {
                if keys.len() == 1 {
                    if let Some(v) = value {
                        table.insert(&keys[0], v);
                    } else {
                        table.remove(&keys[0]);
                    }
                    return Ok(());
                }
                if !table.contains_key(&keys[0]) {
                    table.insert(&keys[0], toml_edit::Item::Table(toml_edit::Table::new()));
                }
                let next = table
                    .get_mut(&keys[0])
                    .and_then(|v| v.as_table_like_mut())
                    .ok_or_else(|| error("expected TOML table"))?;
                update(next, &keys[1..], value)
            }
            let item = value
                .map(|v| {
                    toml_edit::ser::to_document(&serde_json::json!({"value":v}))
                        .map(|d| d["value"].clone())
                        .map_err(error)
                })
                .transpose()?;
            update(doc.as_table_mut(), keys, item)?;
            Ok(doc.to_string())
        }
        Format::Env => {
            let mut lines: Vec<_> = text
                .lines()
                .filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != keys[0]))
                .map(str::to_owned)
                .collect();
            if let Some(v) = value {
                lines.push(format!(
                    "{}={}",
                    keys[0],
                    v.as_str().ok_or_else(|| error("env value must be text"))?
                ));
            }
            Ok(lines.join("\n") + "\n")
        }
        _ => {
            let mut doc = parse(text, format)?;
            if !doc.is_object() {
                return Err(error("expected config object"));
            }
            set(&mut doc, keys, value);
            Ok(serde_json::to_string_pretty(&doc).map_err(error)? + "\n")
        }
    }
}
fn yaml(text: &str, keys: &[String], value: Option<Value>) -> Result<String> {
    // Validate the input before editing. Only block mappings are edited in place;
    // flow mappings are rejected rather than losing comments or user data.
    let parsed = parse(text, Format::Yaml)?;
    if !parsed.is_object() {
        return Err(error("expected YAML mapping"));
    }
    fn patch(text: &str, keys: &[String], value: Option<Value>, indent: usize) -> Result<String> {
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let start = lines.iter().position(|l| {
            let t = l.trim_start();
            l.len() - t.len() == indent
                && t.split_once(':')
                    .is_some_and(|(k, _)| k.trim().trim_matches(['\'', '"']) == keys[0])
        });
        if let Some(start) = start {
            let end = (start + 1..lines.len())
                .find(|i| {
                    let t = lines[*i].trim_start();
                    !t.trim().is_empty()
                        && !t.starts_with('#')
                        && lines[*i].len() - t.len() <= indent
                })
                .unwrap_or(lines.len());
            let head = lines[..start].concat();
            let tail = lines[end..].concat();
            if keys.len() == 1 {
                let replacement = if let Some(v) = value {
                    block(&keys[0], v, indent)?
                } else {
                    String::new()
                };
                return Ok(head + &replacement + &tail);
            }
            let after = lines[start]
                .split_once(':')
                .map(|(_, v)| v.trim())
                .unwrap_or("");
            if !after.is_empty() && !after.starts_with('#') {
                return Err(error("nested YAML configuration must use block mappings"));
            }
            let body = lines[start + 1..end].concat();
            let child_indent = lines[start + 1..end]
                .iter()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(indent + 2);
            return Ok(head
                + lines[start]
                + &patch(&body, &keys[1..], value, child_indent)?
                + &tail);
        }
        if value.is_none() {
            return Ok(text.into());
        }
        let mut nested = value.unwrap();
        for key in keys[1..].iter().rev() {
            nested = serde_json::json!({key:nested});
        }
        Ok(format!(
            "{}{}{}",
            text,
            if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            },
            block(&keys[0], nested, indent)?
        ))
    }
    fn block(key: &str, value: Value, indent: usize) -> Result<String> {
        let map = serde_json::json!({key:value});
        let out = serde_yaml::to_string(&map).map_err(error)?;
        Ok(out
            .lines()
            .map(|l| format!("{}{l}\n", " ".repeat(indent)))
            .collect())
    }
    let result = patch(text, keys, value, 0)?;
    parse(&result, Format::Yaml)?;
    Ok(result)
}
