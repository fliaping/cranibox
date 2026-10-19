// Cranibox's native Pi extension. Pi discovers this in its extensions directory.
// MCP stdio is newline-delimited JSON-RPC. No shell or third-party runtime needed.
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";

function connect(spec) {
  if (typeof spec.command !== "string" || !Array.isArray(spec.args ?? []))
    throw new Error("Invalid Cranibox MCP command");
  const child = spawn(spec.command, spec.args ?? [], {
    env: { ...process.env, ...spec.env }, stdio: ["pipe", "pipe", "ignore"],
  });
  const pending = new Map();
  let sequence = 0, buffer = "", closed = false;
  function close(error = new Error("Cranibox MCP server disconnected")) {
    if (closed) return;
    closed = true;
    for (const request of pending.values()) request.reject(error);
    pending.clear();
    child.kill();
  }
  function send(message) {
    if (closed) throw new Error("Cranibox MCP server is offline");
    child.stdin.write(JSON.stringify(message) + "\n");
  }
  child.on("error", close);
  child.on("exit", () => close());
  child.stdin.on("error", close);
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", chunk => {
    buffer += chunk;
    if (Buffer.byteLength(buffer) > 4 * 1024 * 1024) {
      close(new Error("Cranibox MCP response exceeds 4 MiB")); return;
    }
    let newline;
    while ((newline = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, newline); buffer = buffer.slice(newline + 1);
      if (!line.trim()) continue;
      let message;
      try { message = JSON.parse(line); }
      catch { close(new Error("Invalid Cranibox MCP JSON response")); return; }
      const request = pending.get(message.id);
      if (request) {
        if (message.error) request.reject(new Error(message.error.message ?? "MCP request failed"));
        else request.resolve(message.result);
      } else if (message.method && message.id !== undefined) {
        // Servers may ping; roots and sampling are deliberately not advertised.
        send(message.method === "ping"
          ? { jsonrpc: "2.0", id: message.id, result: {} }
          : { jsonrpc: "2.0", id: message.id, error: { code: -32601, message: "Method not supported" } });
      }
    }
  });
  function request(method, params, signal, timeout = 10000) {
    if (signal?.aborted) return Promise.reject(new Error("MCP request cancelled"));
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      let timer;
      function settle(callback, value) {
        if (!pending.delete(id)) return;
        clearTimeout(timer); signal?.removeEventListener("abort", abort);
        callback(value);
      }
      function abort() {
        try { send({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: id } }); } catch {}
        settle(reject, new Error("MCP request cancelled"));
      }
      pending.set(id, { resolve: value => settle(resolve, value), reject: error => settle(reject, error) });
      timer = setTimeout(() => settle(reject, new Error("Cranibox MCP request timed out")), timeout);
      signal?.addEventListener("abort", abort, { once: true });
      try { send({ jsonrpc: "2.0", id, method, params }); }
      catch (error) { settle(reject, error); }
    });
  }
  return { request, send, close };
}

function toolName(server, raw) {
  const original = "mcp__" + server + "__" + raw;
  const clean = original.replace(/[^a-zA-Z0-9_-]/g, "_");
  return clean === original && clean.length <= 64 ? clean
    : clean.slice(0, 51) + "_" + createHash("sha256").update(original).digest("hex").slice(0, 12);
}

export default async function craniboxMcp(pi) {
  const directory = process.env.PI_CODING_AGENT_DIR || join(homedir(), ".pi", "agent");
  let config;
  try { config = JSON.parse(await readFile(join(directory, "mcp.json"), "utf8")); }
  catch (error) { if (error.code === "ENOENT") return; throw error; }
  const connections = [];
  pi.on("session_shutdown", () => { for (const connection of connections) connection.close(); });
  try {
    for (const [server, spec] of Object.entries(config.mcpServers ?? {})) {
      const connection = connect(spec); connections.push(connection);
      await connection.request("initialize", {
        protocolVersion: "2024-11-05", capabilities: {},
        clientInfo: { name: "cranibox-pi", version: "0.1.0" },
      });
      connection.send({ jsonrpc: "2.0", method: "notifications/initialized" });
      let cursor;
      const seen = new Set();
      for (let page = 0; page < 64; page++) {
        const result = await connection.request("tools/list", cursor ? { cursor } : {});
        if (!Array.isArray(result.tools)) throw new Error("Invalid MCP tools list");
        for (const tool of result.tools) {
          if (typeof tool.name !== "string" || seen.has(tool.name)) throw new Error("Invalid or duplicate MCP tool name");
          seen.add(tool.name);
          pi.registerTool({
            name: toolName(server, tool.name), label: server + ": " + tool.name,
            description: tool.description ?? tool.name,
            parameters: tool.inputSchema ?? { type: "object", properties: {} },
            async execute(_id, args, signal) {
              const response = await connection.request("tools/call", { name: tool.name, arguments: args }, signal, 60000);
              if (response.isError) throw new Error((response.content ?? []).filter(c => c.type === "text").map(c => c.text).join("\n") || "MCP tool failed");
              return {
                content: (response.content ?? []).map(c => c.type === "text" || c.type === "image" ? c : { type: "text", text: JSON.stringify(c) }),
                details: response.structuredContent ?? {},
              };
            },
          });
        }
        cursor = result.nextCursor;
        if (!cursor) break;
        if (page === 63) throw new Error("MCP tools pagination exceeds 64 pages");
      }
    }
  } catch (error) {
    for (const connection of connections) connection.close();
    throw error;
  }
}
