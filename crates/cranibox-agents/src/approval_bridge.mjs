// First-party local approval bridge. Only the human App API writes decisions.
// Files are private, requests expire, and every grant applies to one UUID.
import { mkdir, chmod, open, rename, readFile, unlink } from "node:fs/promises";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { setTimeout as delay } from "node:timers/promises";

const queue = __QUEUE_JSON__;
export async function request(agent, tool, detail, signal) {
  if (signal?.aborted) return false;
  await mkdir(queue, { recursive: true, mode: 0o700 });
  await chmod(queue, 0o700);
  const id = randomUUID();
  const path = join(queue, id + ".json"), decision = join(queue, id + ".decision");
  const temp = path + ".tmp";
  const created_at = Math.floor(Date.now() / 1000), expires_at = created_at + 300;
  const file = await open(temp, "wx", 0o600);
  try {
    await file.writeFile(JSON.stringify({ id, agent, tool: String(tool).slice(0, 256), detail: String(detail ?? "").slice(0, 8192), created_at, expires_at }));
    await file.sync();
  } finally { await file.close(); }
  await rename(temp, path);
  try {
    while (!signal?.aborted && Date.now() < expires_at * 1000) {
      try {
        const result = JSON.parse(await readFile(decision, "utf8"));
        return !signal?.aborted && Date.now() < expires_at * 1000 && result.id === id && result.decision === "allow_once";
      } catch (error) { if (error.code !== "ENOENT") return false; }
      try { await delay(150, undefined, { signal }); } catch { return false; }
    }
    return false;
  } finally {
    await unlink(path).catch(() => {});
    await unlink(decision).catch(() => {});
  }
}

if (process.argv[2] === "--claude-hook") {
  let allowed = false;
  try {
    let input = "";
    for await (const chunk of process.stdin) {
      input += chunk;
      if (Buffer.byteLength(input) > 64 * 1024) throw new Error("Hook input too large");
    }
    const event = JSON.parse(input);
    if (event.hook_event_name === "PreToolUse")
      allowed = await request("claude-code", event.tool_name, JSON.stringify(event.tool_input));
  } catch {}
  process.stdout.write(JSON.stringify({ hookSpecificOutput: {
    hookEventName: "PreToolUse", permissionDecision: allowed ? "allow" : "deny",
    permissionDecisionReason: allowed ? "Approved once in Cranibox" : "Cranibox approval denied, expired or unavailable",
  } }) + "\n");
}
