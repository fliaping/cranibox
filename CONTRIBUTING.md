# Contributing to Cranibox

Thanks for helping. Issues, questions and pull requests are all welcome, in English or Chinese (中文也可以).

## Where things live

| Path | What |
|---|---|
| `crates/workspace-node` | The node: identity, pairing, access control, peer server, Web UI backend, updates, virtual spaces |
| `crates/workspace-runtime` | Client side: CLI commands, MCP server (`cranibox mcp`), Computer Use and desktop clients |
| `crates/cranibox-net`, `cranibox-contracts`, `cranibox-agents`, `cranibox-llm` | iroh networking, wire contracts and fixtures, agent integrations, model proxy |
| `crates/workspacectl`, `workspace-services`, `workspace-web` | Container tools built into the [Agent Workspace](https://github.com/fliaping/agent-workspace) image (`workspace` feature) |
| `crates/cranibox` | The `cranibox` binary that ties them together |
| `apps/node-desktop` | Tauri desktop app (tray, windows, installers) |
| `addons/control-center/ui` | React UI shared by the desktop app and the CLI (`npm run build` regenerates `addons/control-center/*.html|js|css`) |
| `npm/`, `scripts/release/` | npm launcher and release tooling |
| `tests/` | Python integration tests that start real nodes |
| `docs/` | Design notes and user documentation (mostly Chinese, some bilingual) |

The workspace container image (Dockerfile, s6 services, image scripts) lives in [fliaping/agent-workspace](https://github.com/fliaping/agent-workspace).

## Build and test

You need a stable Rust toolchain with `rustfmt` and `clippy`, Python 3.12+ and Node.js 22+.

```bash
cargo build --locked                       # debug build of every crate
# Fast optimized iteration (incremental, no LTO); use release for published binaries.
cargo build --profile release-fast --locked -p cranibox
cargo fmt --all --check                    # CI fails on formatting
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --locked

# Integration tests start real nodes from target/debug
python3 -m unittest discover -s tests -p 'test_node_peers.py' -v

# UI
npm ci --prefix addons/control-center/ui
npm test --prefix addons/control-center/ui          # Playwright
node scripts/build-control-center-ui.cjs            # regenerate the embedded UI bundle
```

A few tests drive Cranibox binaries with scripts from the workspace image. Check out
[agent-workspace](https://github.com/fliaping/agent-workspace) next to this repository, or set
`AGENT_WORKSPACE_SRC`; without it those tests are skipped.

Run a node from source with `cargo run -p cranibox -- --help`. Use a separate config directory for experiments so you do not touch your real identity.

## Pull requests

- Keep each pull request to one change, with tests where the behaviour can be tested.
- Run `cargo fmt`, `clippy` and the tests you touched before pushing.
- If you change the UI, run `node scripts/build-control-center-ui.cjs` and commit the regenerated bundle; CI checks that it matches.
- Security-relevant changes (identity, pairing, access control, anything that runs commands on another device) need a short note on the threat they address or could introduce.

## Reporting security issues

Please do not open a public issue for vulnerabilities. Use GitHub's private vulnerability reporting on this repository (Security → Report a vulnerability).

## License

By contributing you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
