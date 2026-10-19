//! Model catalog (`/v1/models`, ModelEntry availability). **WS2.**
use cranibox_contracts::llm::{Availability, ModelEntry, Preset, PresetFile, Wire};

pub fn presets() -> Vec<Preset> {
    toml::from_str::<PresetFile>(include_str!("upstream/presets.toml"))
        .expect("built-in presets are validated by tests")
        .preset
}
pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.id == id)
}

/// Load sorted TOML overrides without modifying the shipped table. Invalid files fail closed.
pub fn load_overrides(directory: &std::path::Path) -> anyhow::Result<Vec<Preset>> {
    let mut merged: std::collections::BTreeMap<_, _> =
        presets().into_iter().map(|p| (p.id.clone(), p)).collect();
    if !directory.exists() {
        return Ok(merged.into_values().collect());
    }
    let mut files = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    files.sort_by_key(|e| e.path());
    for file in files {
        if file.path().extension().is_none_or(|e| e != "toml") {
            continue;
        }
        anyhow::ensure!(
            file.file_type()?.is_file() && file.metadata()?.len() <= 1_048_576,
            "preset override must be a regular TOML file under 1 MiB"
        );
        let data: PresetFile = toml::from_str(&std::fs::read_to_string(file.path())?)?;
        for preset in data.preset {
            validate(&preset).map_err(anyhow::Error::msg)?;
            merged.insert(preset.id.clone(), preset);
        }
    }
    Ok(merged.into_values().collect())
}

pub fn validate_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "invalid endpoint URL")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("endpoint must use HTTPS (HTTP allowed only on loopback), without credentials, query or fragment".into());
    }
    Ok(url)
}
pub fn validate(p: &Preset) -> Result<(), String> {
    if p.id.is_empty()
        || !p
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || p.label.trim().is_empty()
    {
        return Err("invalid preset identity".into());
    }
    for raw in [&p.chat, &p.responses, &p.anthropic].into_iter().flatten() {
        validate_url(raw)?;
    }
    let native = match p.default_wire {
        Wire::Chat => &p.chat,
        Wire::Responses => &p.responses,
        Wire::Messages => &p.anthropic,
    };
    if native.is_none() || p.models.iter().any(|m| m.trim().is_empty()) {
        return Err("invalid preset endpoints or models".into());
    }
    Ok(())
}
pub fn model(provider: &str, name: &str, native: Wire) -> ModelEntry {
    ModelEntry {
        id: format!("{provider}/{name}"),
        display: name.into(),
        wires: vec![Wire::Chat, Wire::Responses, Wire::Messages],
        native,
        context: None,
        reasoning: false,
        vision: false,
        availability: Availability::Local,
    }
}
