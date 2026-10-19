//! DSH loader patches are a sequence, addressed by entry id. Keep unrelated
//! entries and comments intact while presenting ids to the ownership journal.
use super::*;

fn id(row: &Value) -> Option<&str> {
    row.get("id").and_then(Value::as_str).or_else(|| {
        let insert = row.get("insert")?.as_array()?;
        (insert.len() == 1)
            .then(|| insert[0].get("id")?.as_str())
            .flatten()
    })
}
pub(super) fn parse(text: &str) -> Result<Value> {
    let rows: Vec<Value> = serde_yaml::from_str(text).map_err(error)?;
    let mut doc = serde_json::Map::new();
    for row in rows {
        if let Some(id) = id(&row)
            && doc.insert(id.to_owned(), row).is_some()
        {
            return Err(error(
                "duplicate DSH patch id; review configuration before editing",
            ));
        }
    }
    Ok(Value::Object(doc))
}
pub(super) fn render(text: &str, keys: &[String], value: Option<Value>) -> Result<String> {
    let mut doc = super::parse(text, Format::Cordis)?;
    let existed = doc.get(&keys[0]).is_some();
    set(&mut doc, keys, value);
    let row = doc.get_mut(&keys[0]);
    let replacement = if let Some(row) = row {
        if row.get("insert").is_none() {
            row["id"] = keys[0].clone().into();
        }
        serde_yaml::to_string(&vec![row]).map_err(error)?
    } else {
        String::new()
    };
    if text.trim() == "[]" || text.trim().is_empty() {
        return Ok(replacement);
    }
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.starts_with("- ").then_some(i))
        .collect();
    if starts.is_empty() {
        return Err(error(
            "DSH patches must use a block sequence at column zero",
        ));
    }
    for (n, start) in starts.iter().copied().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(lines.len());
        let chunk = lines[start..end].concat();
        let rows: Vec<Value> = serde_yaml::from_str(&chunk).map_err(error)?;
        if rows.first().and_then(id) != Some(keys[0].as_str()) {
            continue;
        }
        // Edit a targeted row as a block mapping to retain its own unrelated
        // settings and comments too. Whole-row edits intentionally replace it.
        let replacement = if keys.len() > 1 {
            let mapping: String = chunk
                .lines()
                .enumerate()
                .map(|(i, l)| {
                    let line = if i == 0 {
                        &l[2..]
                    } else {
                        l.strip_prefix("  ").unwrap_or(l)
                    };
                    format!("{line}\n")
                })
                .collect();
            let patched = super::yaml(&mapping, &keys[1..], get(&doc, keys).cloned())?;
            patched
                .lines()
                .enumerate()
                .map(|(i, l)| format!("{}{l}\n", if i == 0 { "- " } else { "  " }))
                .collect()
        } else {
            replacement
        };
        let output = lines[..start].concat() + &replacement + &lines[end..].concat();
        super::parse(&output, Format::Cordis)?;
        return Ok(output);
    }
    if existed {
        return Err(error(
            "DSH patch could not be located without losing formatting",
        ));
    }
    Ok(format!(
        "{}{}{}",
        text,
        if text.ends_with('\n') { "" } else { "\n" },
        replacement
    ))
}
