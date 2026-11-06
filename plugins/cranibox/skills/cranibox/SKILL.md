---
name: cranibox
description: Use the user's other devices (Mac, Windows, Linux servers, containers) through Cranibox. Use when a task needs another machine than the one you run on, such as running tests on Windows, reading a log on a server, building on a Mac, copying files between devices, or checking what devices are available.
---

# Cranibox: work on the user's other devices

Cranibox connects the user's own devices peer to peer. Through the `cranibox` MCP server (or the `cranibox` CLI) you can list those devices, run programs on them, and copy files to and from them. Every device decides for itself what it allows, so expect some requests to be refused or to wait for the user's approval.

## Workflow

1. **Find the device.** Call `workspace_nodes` (CLI: `cranibox nodes`). Pick the target by its name and OS, and use its exact NodeID. Check `online` before you start; do not guess IDs.
2. **Run a program.** Call `workspace_exec` with `node`, `program` and `args`. There is no implicit shell:
   - macOS / Linux: `program: "bash", args: ["-lc", "cd ~/proj && npm test"]` when you need pipes, globs or `cd`.
   - Windows: `program: "powershell", args: ["-NoProfile", "-Command", "..."]`. Windows output may be in the system code page; prefer ASCII output or write to a UTF-8 file and pull it.
   - Set `timeout_secs` (default 60, max 3600) for long builds or tests.
3. **Wait for it.** `workspace_exec` returns a job ID. Poll `workspace_job` until it has an exit code. Output is capped at 64 KiB and jobs expire after ten minutes: for long or noisy commands, redirect output to a file on the device and pull the file. Use `workspace_cancel` to stop a job you no longer need.
4. **Move files.** Use `workspace_push` (here → device) and `workspace_pull` (device → here) with `node`, `local`, `remote`, and `overwrite` only when replacing is intended. Transfers are chunked and SHA-256 verified, and work for folders. Do not paste file contents through command arguments.
5. **Report back** what ran where, the exit code, and where results now are.

CLI equivalents, for when the MCP server is not configured:

```bash
cranibox nodes
cranibox exec --node <NodeID> -- <program> <args...>
cranibox push --node <NodeID> <local> <remote>
cranibox pull --node <NodeID> <remote> <local>
```

## When a request is refused

- `capability_off`: that device has not turned this capability on for this device. Tell the user exactly which device and capability, for example: "On *Mac mini*, open Settings → Capabilities and allow commands for this device." Do not look for another way in.
- `approval_required` or a pending approval: the device is waiting for the user to approve. Tell the user what you asked to do and on which device, then wait. Do not resend the request in a loop.
- Device offline: say so and stop; do not retry indefinitely.

## Rules

- Commands run with the privileges of the account that runs Cranibox on that device. Treat every device as the user's real machine with real data.
- Ask the user before anything destructive or hard to undo on another device (deleting files, force-pushing, reinstalling software, changing system settings), even when the capability is allowed.
- Never try to change another device's permissions, approve your own requests, or read Cranibox identity and key files.
- Prefer read-only checks first (`uname -a`, `git status`, `ls`) to learn the environment before changing it.
- Keep work inside the project directories the user pointed you to; use `workspace_projects` to find registered projects across devices.
