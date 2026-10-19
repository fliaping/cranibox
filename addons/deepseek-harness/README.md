# DeepSeek Harness

This optional Agent installs DeepSeek AI's official developer-preview
`@deepseek-ai/dsh` package into the persistent npm prefix under `/config`. The
Agent Workspace wrapper keeps runtime state in `/config/.dsh`, starts from
`/config/Workspace`, and loads the MCP patch owned by Control Center.

Install and operate it through the same Agent interfaces as the other Agents:

```bash
agent-workspace-manager install agent deepseek-harness
deepseek-harness web --no-open --port 3080
deepseek-harness headless "summarize this workspace"
```

Normal manager installation also creates `deepseek-harness.service`. Its Web UI
binds to loopback and is available from the authenticated code-server origin at
`/proxy/3080/`; it must not be exposed directly without an authentication
layer. A local compatibility proxy keeps the official UI's root-relative
assets, API calls, and WebSockets inside that path while the official Harness
server remains private on port `3081`. Configure the model and API key from the
Web UI. Credentials remain in DeepSeek Harness' own write-only credential store
under `/config/.dsh`.

DeepSeek Harness natively discovers shared Skills under
`/config/.agents/skills`. Global MCP entries selected for this Agent are
rendered into `/config/.dsh/agent-workspace-mcp.cordis.yml` and loaded by the
wrapper without overwriting user-owned Harness profile patches.

### Reviewed runtime version

Cranibox defaults to DeepSeek Harness. The installer and cranibox-agents pin
@deepseek-ai/dsh@0.1.1-rc.2 with SHA-256
47ec05f45ada5ab87779ae18a90456b5ebff5421dc0ff5c179677d65e1c16057.
The installer verifies the downloaded tarball before running installation scripts.
To upgrade, review the official release and configuration format, download the
exact npm tarball, compute its SHA-256, update both constants, and run adapter
round-trip tests and a headless conversation against a mock model before release.
