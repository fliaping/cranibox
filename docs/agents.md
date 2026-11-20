# Connect Cranibox to more agents

Any MCP client can use Cranibox. `cranibox mcp` is a stdio MCP server: the agent starts it as a subprocess and talks to it over stdin/stdout. The README covers Claude Code and Codex; this page covers Cursor, Gemini CLI, OpenCode and Hermes Agent.

The desktop app can also write these entries for detected agents with one click. Use the snippets below when you prefer to do it yourself.

## Before you start: put `cranibox` on PATH

Every snippet runs the command `cranibox`, so the agent must be able to find it on its PATH.

- **npm install** (`npm install -g @fliaping/cranibox`), the Linux package and the Windows installer already put `cranibox` on PATH.
- **macOS desktop app:** open **This device → Settings → Desktop app → Install command line tool**. This links the app to `/usr/local/bin/cranibox`.

Check it from a new terminal:

```bash
command -v cranibox
```

GUI apps such as Cursor may not see the PATH from your shell profile. If the agent reports that `cranibox` cannot be found, replace `"cranibox"` in the snippet with the full path printed by `command -v cranibox` (on Windows, `where cranibox`).

## Cursor

Add to `~/.cursor/mcp.json` (all projects) or `.cursor/mcp.json` in a project:

```json
{
  "mcpServers": {
    "cranibox": {
      "type": "stdio",
      "command": "cranibox",
      "args": ["mcp"]
    }
  }
}
```

Merge the `cranibox` entry into `mcpServers` if the file already exists. Then enable it under Customize and check *Available Tools*. Reference: [Cursor MCP docs](https://cursor.com/docs/context/mcp).

## Gemini CLI

```bash
gemini mcp add --scope user cranibox cranibox mcp
```

Or add to `~/.gemini/settings.json` (all projects) or `.gemini/settings.json` in a project:

```json
{
  "mcpServers": {
    "cranibox": {
      "command": "cranibox",
      "args": ["mcp"]
    }
  }
}
```

Run `/mcp` in Gemini CLI to see the server and its tools. Gemini CLI only starts stdio servers in trusted folders. Reference: [Gemini CLI MCP docs](https://geminicli.com/docs/tools/mcp-server/).

## OpenCode

Add to `opencode.jsonc` in a project, or to the global OpenCode config:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "cranibox": {
        "type": "local",
        "command": ["cranibox", "mcp"]
      }
    }
  }
}
```

Check it with `opencode mcp list`. Current OpenCode documents servers under `mcp.servers`; older configs that put `cranibox` directly under `mcp` are still read. Reference: [OpenCode MCP docs](https://opencode.ai/docs/mcp-servers/).

## Hermes Agent

Add to `~/.hermes/config.yaml` (and to `~/.hermes/profiles/<name>/config.yaml` for other profiles):

```yaml
mcp_servers:
  cranibox:
    command: "cranibox"
    args: ["mcp"]
```

Check it with `hermes mcp test cranibox`, then start a new `hermes chat` (or run `/reload-mcp` in a running session). Hermes needs its MCP extra (`pip install 'hermes-agent[mcp]'`); the standard installer includes it. The tools appear as `mcp_cranibox_<tool>`. Reference: [Hermes Agent MCP docs](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp).

## Teach the agent the safe way to use devices

The [Cranibox skill](../plugins/cranibox/skills/cranibox/SKILL.md) explains how to pick a device and when to wait for approval. Agents that read `SKILL.md` folders can use it directly, for example by copying it to `~/.hermes/skills/cranibox`.
