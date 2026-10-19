use cranibox_agents::*;
use std::{collections::BTreeMap, fs};
fn fixture() -> (tempfile::TempDir, Env, ModelPick) {
    let root = tempfile::tempdir().unwrap();
    let mut env = environment(root.path().join("home"), root.path().join("data"));
    env.path.clear();
    let pick = ModelPick {
        default: "test/model".into(),
        small_fast: None,
        models: vec![],
    };
    (root, env, pick)
}
#[test]
fn all_six_round_trip_and_detect_drift() {
    for adapter in registry() {
        let (_root, env, pick) = fixture();
        let applied = adapter
            .wire(&env, &LocalEndpoint::for_agent(adapter.id()), &pick)
            .unwrap();
        assert!(!applied.files.is_empty());
        assert!(
            adapter
                .drift(&env)
                .unwrap()
                .files
                .iter()
                .all(|f| f.state == DriftState::Clean)
        );
        adapter
            .wire(&env, &LocalEndpoint::for_agent(adapter.id()), &pick)
            .unwrap();
        adapter.unwire(&env).unwrap();
        assert!(applied.files.iter().all(|f| !f.path.exists()));
    }
}
#[test]
fn original_files_restored_byte_for_byte() {
    for adapter in registry() {
        let (_root, env, pick) = fixture();
        let generated = adapter
            .wire(&env, &LocalEndpoint::for_agent(adapter.id()), &pick)
            .unwrap();
        adapter.unwire(&env).unwrap();
        let mut originals = vec![];
        for file in generated.files {
            let text = match file.path.extension().and_then(|x| x.to_str()) {
                Some("toml") => "# user preferences\ntheme = 'night'\n",
                Some("yaml" | "yml") if adapter.id() == AgentId::DeepseekHarness => {
                    "# user preferences\n- id: custom\n  config: retained\n"
                }
                Some("yaml" | "yml") => "# user preferences\ncustom: retained\n",
                _ if file.path.file_name().unwrap() == ".env" => {
                    "# user preferences\nUSER_KEY=retained\n"
                }
                _ => "{\"custom\": \"retained\"}\n",
            };
            fs::create_dir_all(file.path.parent().unwrap()).unwrap();
            fs::write(&file.path, text).unwrap();
            originals.push((file.path, text.to_owned()));
        }
        adapter
            .wire(&env, &LocalEndpoint::for_agent(adapter.id()), &pick)
            .unwrap();
        adapter.unwire(&env).unwrap();
        for (path, text) in originals {
            assert_eq!(fs::read_to_string(path).unwrap(), text);
        }
    }
}
#[test]
fn user_changes_survive_unwire() {
    let (_root, env, pick) = fixture();
    let a = adapter(AgentId::Pi).unwrap();
    a.wire(&env, &LocalEndpoint::for_agent(a.id()), &pick)
        .unwrap();
    let path = env.home.join(".pi/agent/settings.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["theme"] = "user-theme".into();
    v["defaultModel"] = "user/model".into();
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(
        a.drift(&env)
            .unwrap()
            .files
            .iter()
            .any(|f| f.state == DriftState::Modified)
    );
    assert!(
        a.wire(&env, &LocalEndpoint::for_agent(a.id()), &pick)
            .is_err()
    );
    a.unwire(&env).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(v["theme"], "user-theme");
    assert_eq!(v["defaultModel"], "user/model");
    assert!(v.get("defaultProvider").is_none());
}
#[test]
fn yaml_keeps_unrelated_comments() {
    let (_root, env, pick) = fixture();
    let a = adapter(AgentId::Hermes).unwrap();
    fs::create_dir_all(env.home.join(".hermes")).unwrap();
    let path = env.home.join(".hermes/config.yaml");
    fs::write(
        &path,
        "# user header\nmodel:\n  # keep temperature\n  temperature: 0.4 # precise\ncustom: true\n",
    )
    .unwrap();
    a.wire(&env, &LocalEndpoint::for_agent(a.id()), &pick)
        .unwrap();
    let s = fs::read_to_string(path).unwrap();
    assert!(s.contains("# user header"));
    assert!(s.contains("# keep temperature"));
    assert!(s.contains("temperature: 0.4 # precise"));
}
#[test]
fn launch_arguments_and_mcp_are_reversible() {
    for a in registry() {
        let (_root, env, _) = fixture();
        let prompt = "quote ' ; $(not-a-shell)";
        let launch = a
            .headless(&HeadlessTask {
                prompt: prompt.into(),
                cwd: env.home.clone(),
                session_id: None,
                json: true,
            })
            .unwrap();
        assert!(
            launch.args.contains(&prompt.to_owned()) || launch.stdin.as_deref() == Some(prompt)
        );
        let applied = a
            .register_mcp(
                &env,
                &McpSpec {
                    name: "cranibox".into(),
                    command: "cranibox".into(),
                    args: vec!["mcp".into()],
                    env: BTreeMap::new(),
                },
            )
            .unwrap();
        a.unwire(&env).unwrap();
        assert!(applied.files.iter().all(|f| !f.path.exists()));
    }
}

#[test]
fn removing_one_mcp_preserves_models_other_servers_and_undo_history() {
    for a in registry() {
        let (_root, env, pick) = fixture();
        let wired = a
            .wire(&env, &LocalEndpoint::for_agent(a.id()), &pick)
            .unwrap();
        for name in ["first", "second"] {
            a.register_mcp(
                &env,
                &McpSpec {
                    name: name.into(),
                    command: "test-mcp".into(),
                    args: vec![name.into()],
                    env: BTreeMap::new(),
                },
            )
            .unwrap();
        }
        let removed = unregister_mcp(a.id(), &env, "first").unwrap();
        assert!(!removed.files.is_empty(), "{}", a.id().as_str());
        assert!(
            a.drift(&env)
                .unwrap()
                .files
                .iter()
                .all(|f| f.state == DriftState::Clean)
        );
        let text = removed
            .files
            .iter()
            .map(|f| fs::read_to_string(&f.path).unwrap())
            .collect::<String>();
        assert!(!text.contains("first"));
        assert!(text.contains("second"));
        // Re-registering after partial undo must still have a valid journal.
        a.register_mcp(
            &env,
            &McpSpec {
                name: "third".into(),
                command: "test-mcp".into(),
                args: vec![],
                env: BTreeMap::new(),
            },
        )
        .unwrap();
        a.unwire(&env).unwrap();
        assert!(wired.files.iter().all(|f| !f.path.exists()));
        assert!(removed.files.iter().all(|f| !f.path.exists()));
    }
}

#[test]
fn all_six_configuration_goldens_and_drift_preserve_user_values() {
    for adapter in registry() {
        let (_root, env, pick) = fixture();
        let applied = adapter
            .wire(&env, &LocalEndpoint::for_agent(adapter.id()), &pick)
            .unwrap();
        let mut snapshot = BTreeMap::new();
        for file in &applied.files {
            let mut text = fs::read_to_string(&file.path).unwrap();
            if adapter.id() == AgentId::Codex
                && file.path.extension().and_then(|value| value.to_str()) == Some("toml")
            {
                // Validate the native path, then normalize only that value for
                // the shared golden (Windows TOML may use literal backslashes).
                let mut config = text.parse::<toml_edit::DocumentMut>().unwrap();
                assert_eq!(
                    config["model_catalog_json"].as_str(),
                    env.home.join(".codex/cranibox-models.json").to_str()
                );
                config["model_catalog_json"] =
                    toml_edit::value("$HOME/.codex/cranibox-models.json");
                text = config.to_string();
            }
            snapshot.insert(
                file.path
                    .strip_prefix(&env.home)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                text.replace(env.home.to_str().unwrap(), "$HOME"),
            );
        }
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/goldens")
            .join(format!("{}.json", adapter.id().as_str()));
        let rendered = serde_json::to_string_pretty(&snapshot).unwrap() + "\n";
        if std::env::var_os("UPDATE_AGENT_GOLDENS").is_some() {
            fs::create_dir_all(golden.parent().unwrap()).unwrap();
            fs::write(&golden, &rendered).unwrap();
        }
        assert_eq!(
            rendered,
            fs::read_to_string(golden).unwrap().replace("\r\n", "\n"),
            "{}",
            adapter.id().as_str()
        );
        // Simulate a user changing every generated file, not just the primary
        // config. Undo must retain their value and remove our owned leaves.
        for file in &applied.files {
            let mut text = fs::read_to_string(&file.path).unwrap();
            match file.path.extension().and_then(|s| s.to_str()) {
                Some("toml") => text.insert_str(0, "user_preference = 'retained'\n"),
                Some("yaml" | "yml") if adapter.id() == AgentId::DeepseekHarness => {
                    text.push_str("\n- id: user_preference\n  config: retained\n")
                }
                Some("yaml" | "yml") => text.push_str("\nuser_preference: retained\n"),
                _ if file.path.file_name().unwrap() == ".env" => {
                    text.push_str("\nUSER_PREFERENCE=retained\n")
                }
                _ => {
                    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
                    doc["user_preference"] = "retained".into();
                    text = serde_json::to_string(&doc).unwrap();
                }
            }
            fs::write(&file.path, text).unwrap();
        }
        assert!(
            adapter
                .drift(&env)
                .unwrap()
                .files
                .iter()
                .all(|f| f.state == DriftState::Modified)
        );
        adapter.unwire(&env).unwrap();
        for file in &applied.files {
            let text = fs::read_to_string(&file.path).unwrap();
            assert!(text.contains("retained"));
            assert!(!text.contains("127.0.0.1:3424"));
            assert!(!text.contains("cbx-"));
        }
    }
}

#[cfg(unix)]
#[test]
fn openclaw_prefers_cli_and_cleans_only_our_generated_cache() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, mut env, pick) = fixture();
    let bin = env.home.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let cli = bin.join("openclaw");
    fs::write(
        &cli,
        "#!/bin/sh\nprintf '%s %s %s\\n' \"$1\" \"$2\" \"$3\" >> \"$HOME/cli-calls\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(cli, fs::Permissions::from_mode(0o700)).unwrap();
    env.path.push(bin);
    let cache = env.home.join(".openclaw/agents/main/agent/models.json");
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, r#"{"providers":{"cranibox":{"baseUrl":"http://127.0.0.1:3424/v1","apiKey":"cbx-openclaw"},"user":{"apiKey":"fake-user-key"}}}"#).unwrap();
    let a = adapter(AgentId::Openclaw).unwrap();
    a.wire(&env, &LocalEndpoint::for_agent(a.id()), &pick)
        .unwrap();
    assert!(
        fs::read_to_string(env.home.join("cli-calls"))
            .unwrap()
            .contains("config set models.providers.cranibox")
    );
    let cached: serde_json::Value = serde_json::from_slice(&fs::read(cache).unwrap()).unwrap();
    assert!(cached["providers"].get("cranibox").is_none());
    assert_eq!(cached["providers"]["user"]["apiKey"], "fake-user-key");
    a.unwire(&env).unwrap();
}

#[test]
fn hermes_discovers_sqlite_sessions_without_writing_or_reading_messages() {
    let (_root, env, _) = fixture();
    let path = env.home.join(".hermes/state.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE sessions (id TEXT, title TEXT, started_at REAL, ended_at REAL, cwd TEXT); CREATE TABLE messages (content TEXT); INSERT INTO messages VALUES ('private transcript'); INSERT INTO sessions VALUES ('old', 'Earlier', 10.5, NULL, '/work/old'), ('new', 'Latest', 20.5, 30.2, '/work/new');").unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let sessions = adapter(AgentId::Hermes).unwrap().sessions(&Roots {
        home: env.home,
        extra: vec![],
    });
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "new");
    assert_eq!(sessions[0].title.as_deref(), Some("Latest"));
    assert_eq!(
        sessions[0].cwd.as_deref(),
        Some(std::path::Path::new("/work/new"))
    );
    assert_eq!(sessions[0].updated_at, Some(30));
    assert!(!format!("{sessions:?}").contains("private transcript"));
    assert_eq!(fs::read(path).unwrap(), before);
}
