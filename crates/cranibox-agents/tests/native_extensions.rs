use cranibox_agents::*;
use cranibox_contracts::app::ApprovalAction;
use std::{
    collections::BTreeMap,
    fs,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn claude_hook_preserves_user_hooks_and_returns_native_decision() {
    let root = tempfile::tempdir().unwrap();
    let env = environment(root.path().join("home"), root.path().join("data"));
    let settings = env.home.join(".claude/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let old =
        serde_json::json!({"matcher":"Read","hooks":[{"type":"command","command":"user-before"}]});
    let added =
        serde_json::json!({"matcher":"Write","hooks":[{"type":"command","command":"user-after"}]});
    fs::write(
        &settings,
        serde_json::json!({"hooks":{"PreToolUse":[old.clone()]},"theme":"dark"}).to_string(),
    )
    .unwrap();
    approvals::install(&env, AgentId::ClaudeCode).unwrap();
    approvals::install(&env, AgentId::ClaudeCode).unwrap();
    let mut doc: serde_json::Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(doc["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    if node_available() {
        use std::io::Write;
        let mut child = Command::new("node")
            .arg(env.home.join(".claude/cranibox-approval.mjs"))
            .arg("--claude-hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo hello"}}"#).unwrap();
        // Cold Node startup can exceed ten seconds on Windows CI. Keep the
        // human-decision wait separate from this bounded process startup wait.
        let deadline = Instant::now() + Duration::from_secs(30);
        let card = loop {
            if let Some(card) = approvals::pending(&env, "local")
                .unwrap()
                .into_iter()
                .next()
            {
                break card;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "Claude hook exited before publishing approval"
            );
            assert!(
                Instant::now() < deadline,
                "Claude hook did not publish approval"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(child.try_wait().unwrap().is_none());
        assert_eq!(card.agent, Some(AgentId::ClaudeCode));
        approvals::decide(&env, &card.id, ApprovalAction::Deny).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["hookSpecificOutput"]["permissionDecision"], "deny");
    }
    doc["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .push(added.clone());
    fs::write(&settings, doc.to_string()).unwrap();
    adapter(AgentId::ClaudeCode).unwrap().unwire(&env).unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(doc["hooks"]["PreToolUse"], serde_json::json!([old, added]));
    assert_eq!(doc["theme"], "dark");
}

#[test]
fn native_hook_waits_for_human_and_rejects_replay_expiry_and_withdrawal() {
    if !node_available() {
        eprintln!("node unavailable; native approval test skipped");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let env = environment(
        root.path().join("home with spaces#%"),
        root.path().join("data"),
    );
    approvals::install(&env, AgentId::Pi).unwrap();
    let bridge = env.home.join(".pi/agent/cranibox-approval.mjs");
    let code = "import {pathToFileURL} from 'node:url'; const {request}=await import(pathToFileURL(process.argv[1]).href); console.log(await request('pi','bash','echo approved'));";
    let mut child = Command::new("node")
        .args(["--input-type=module", "-e", code])
        .arg(&bridge)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    let card = loop {
        let cards = approvals::pending(&env, "local").unwrap();
        if let Some(card) = cards.into_iter().next() {
            break card;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "native hook exited before publishing approval"
        );
        assert!(Instant::now() < end, "native hook did not publish approval");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(child.try_wait().unwrap().is_none());
    assert_eq!(card.detail.as_deref(), Some("echo approved"));
    assert!(approvals::decide(&env, &card.id, ApprovalAction::AllowAlways).is_err());
    approvals::decide(&env, &card.id, ApprovalAction::AllowOnce).unwrap();
    assert!(approvals::decide(&env, &card.id, ApprovalAction::AllowOnce).is_err());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "true");
    assert!(approvals::pending(&env, "local").unwrap().is_empty());
    assert!(approvals::decide(&env, &card.id, ApprovalAction::AllowOnce).is_err());
    let id = "00000000-0000-4000-8000-000000000001";
    fs::write(env.data.join(format!("agents/approvals/{id}.json")), format!(r#"{{"id":"{id}","agent":"pi","tool":"bash","detail":"expired","created_at":1,"expires_at":2}}"#)).unwrap();
    assert!(approvals::pending(&env, "local").unwrap().is_empty());
    assert!(approvals::decide(&env, &format!("agent:{id}"), ApprovalAction::AllowOnce).is_err());
    let code = "import {pathToFileURL} from 'node:url'; const {request}=await import(pathToFileURL(process.argv[1]).href); const controller=new AbortController(); setTimeout(()=>controller.abort(),50); console.log(await request('pi','bash','cancelled',controller.signal));";
    let output = Command::new("node")
        .args(["--input-type=module", "-e", code])
        .arg(&bridge)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "false");
    assert!(approvals::pending(&env, "local").unwrap().is_empty());
}

#[test]
fn pi_extension_negotiates_and_calls_real_stdio_mcp_with_cancellation() {
    if !node_available() {
        eprintln!("node unavailable; Pi MCP test skipped");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let env = environment(root.path().join("home"), root.path().join("data"));
    let server = root.path().join("server.cjs");
    fs::write(&server, r#"
const lines=require('node:readline').createInterface({input:process.stdin});
lines.on('line', line=>{
 const msg=JSON.parse(line); let result;
 if(msg.method==='initialize') result={protocolVersion:'2024-11-05',capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}};
 if(msg.method==='tools/list') result={tools:[{name:'echo',description:'Echo text',inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text']}}]};
 if(msg.method==='tools/call') { if(msg.params.arguments.text==='wait') return; result={content:[{type:'text',text:msg.params.arguments.text}],structuredContent:{called:true}}; }
 if(result) process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:msg.id,result})+'\n');
});
"#).unwrap();
    adapter(AgentId::Pi)
        .unwrap()
        .register_mcp(
            &env,
            &McpSpec {
                name: "cranibox".into(),
                command: "node".into(),
                args: vec![server.to_string_lossy().into_owned()],
                env: BTreeMap::new(),
            },
        )
        .unwrap();
    let extension = env.home.join(".pi/agent/extensions/cranibox-mcp.ts");
    let test = root.path().join("test.mjs");
    fs::write(&test, r#"
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const {default:extension}=await import('data:text/javascript;base64,'+Buffer.from(await readFile(process.argv[2],'utf8')).toString('base64'));
let shutdown; const tools=[];
await extension({on:(name,callback)=>{assert.equal(name,'session_shutdown');shutdown=callback;},registerTool:tool=>tools.push(tool)});
try {
 assert.equal(tools.length,1);assert.equal(tools[0].name,'mcp__cranibox__echo');
 assert.equal(tools[0].parameters.required[0],'text');
 const response=await tools[0].execute('call1',{text:'native MCP reply'});
 assert.equal(response.content[0].text,'native MCP reply');assert.equal(response.details.called,true);
 const controller=new AbortController();setTimeout(()=>controller.abort(),20);
 await assert.rejects(tools[0].execute('call2',{text:'wait'},controller.signal),/cancelled/);
} finally {shutdown();}
"#).unwrap();
    let output = Command::new("node")
        .arg(test)
        .arg(extension)
        .env("PI_CODING_AGENT_DIR", env.home.join(".pi/agent"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    adapter(AgentId::Pi).unwrap().unwire(&env).unwrap();
    assert!(
        !env.home
            .join(".pi/agent/extensions/cranibox-mcp.ts")
            .exists()
    );
}
