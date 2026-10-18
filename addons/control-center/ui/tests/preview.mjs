import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import { snapshot } from './fixture.mjs';
import { rfbFixture } from './rfb-fixture.mjs';

const uiRoot = fileURLToPath(new URL('..', import.meta.url));
const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'cranibox-ui-preview-'));
const command = path.join(directory, 'workspacectl');
snapshot.locale.current = process.env.UI_PREVIEW_LANGUAGE || 'en';
await fs.mkdir(path.join(directory, '.local/state/agent-workspace'), { recursive: true });
await fs.writeFile(path.join(directory, '.local/state/agent-workspace/locale'), snapshot.locale.current);
await fs.writeFile(path.join(directory, 'snapshot.json'), JSON.stringify(snapshot));
await fs.writeFile(command, [
  '#!' + process.execPath,
  "const fs = require('node:fs'), path = require('node:path');",
  "const args = process.argv.slice(2), root = __dirname;",
  "fs.appendFileSync(path.join(root, 'calls'), JSON.stringify(args) + '|');",
  "if (args[0] === 'status') console.log(fs.readFileSync(path.join(root, 'snapshot.json'), 'utf8'));",
  "else if (args[0] === 'logs' && args[1] === 'long-job') { console.log('long job started'); setInterval(() => console.log('operation still running'), 500); }",
  "else { console.log('preview: ' + args.join(' ')); setTimeout(() => console.log('done'), 400); }",
].join('\n'), { mode: 0o700 });
// Keep network previews independent of any Tailscale daemon on the test host.
await fs.writeFile(path.join(directory, 'tailscale'), '#!' + process.execPath + '\nconsole.log(JSON.stringify({BackendState:"Stopped",Peer:{}}));\n', { mode: 0o700 });
const binary = process.env.CRANIBOX_BIN || path.resolve(uiRoot, '../../../target/debug/cranibox');
const node = process.env.CRANIBOX_NODE_BIN || path.resolve(uiRoot, '../../../target/debug/cranibox-node');
const runtime = process.env.WORKSPACE_RUNTIME_BIN || path.resolve(uiRoot, '../../../target/debug/workspace-runtime');
const webPort = Number(process.env.UI_PREVIEW_PORT || '19080');
const runtimePort = webPort + 10;
const closeDesktopFixture = await rfbFixture(webPort + 30);
const closeAuthenticatedDesktopFixture = await rfbFixture(webPort + 34, { username: 'cranibox', password: 'wayland8' });
const credentials = path.join(directory, '.config/agent-workspace/runtime.json');
await fs.mkdir(path.dirname(credentials), { recursive: true });
await fs.writeFile(path.join(path.dirname(credentials), 'web.json'), JSON.stringify({ runtimeUrl: 'http://127.0.0.1:' + runtimePort }));
const processes = [];
let stopping = false;
async function stop(code = 0) {
  if (stopping) return;
  stopping = true;
  closeDesktopFixture();
  closeAuthenticatedDesktopFixture();
  for (const child of [...processes].reverse()) {
    if (child.exitCode === null && child.signalCode === null) {
      const ended = new Promise(resolve => child.once('exit', resolve));
      child.kill('SIGTERM');
      const timer = setTimeout(() => child.kill('SIGKILL'), 3000);
      await ended;
      clearTimeout(timer);
    }
  }
  if (process.env.CRANIBOX_HERDR_BIN) {
    for (const state of [path.join(directory, '.local/state/cranibox/terminal')]) {
      spawnSync(process.env.CRANIBOX_HERDR_BIN, ['server', 'stop'], { env: { ...process.env, HERDR_CONFIG_PATH: path.join(state, 'config.toml'), HERDR_SOCKET_PATH: path.join(state, 'h.sock') }, timeout: 8000 });
    }
  }
  await fs.rm(directory, { recursive: true, force: true });
  process.exit(code);
}
function start(command, args) {
  const environment = { ...process.env, PATH: directory + path.delimiter + (process.env.PATH || '') };
  const child = spawn(command, args, { stdio: 'inherit', env: command === node ? { ...environment, HOME: directory, CRANIBOX_HERDR_BIN: path.join(directory, 'missing-node-herdr') } : environment });
  processes.push(child);
  child.on('error', error => { console.error(error.message); void stop(1); });
  child.on('exit', code => { if (!stopping) void stop(code || 1); });
  return child;
}
async function wait(check) {
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if (await check()) return; } catch { /* Service is starting. */ }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error('Preview service did not become ready');
}
process.on('SIGTERM', () => void stop());
process.on('SIGINT', () => void stop());
try {
  for (const args of [['init', '--credentials', credentials], ['enroll', 'preview-node', '--credentials', credentials, '--token-file', path.join(directory, 'node.token')]]) {
    const result = spawnSync(runtime, args, { stdio: 'pipe' });
    if (result.status !== 0) throw Error('Preview gateway credential setup failed');
  }
  start(binary, ['serve', '--config-root', directory, '--command', command, '--port', String(webPort), '--gateway-port', String(runtimePort)]);
  await wait(async () => (await fetch('http://127.0.0.1:' + runtimePort + '/healthz')).ok);
  const host = JSON.parse(await fs.readFile(path.join(directory, '.config/cranibox-node/peers.json'), 'utf8')).node_id;
  for (const args of [
    ['desktop', '--address', '127.0.0.1:' + (webPort + 30)],
    ['grant', '--node', host, '--desktop', '--output-token', path.join(directory, 'desktop-grant.token')],
  ]) {
    const result = spawnSync(node, ['--config', path.join(directory, 'preview-node/settings.json'), 'peer', ...args], { stdio: 'pipe' });
    if (result.status !== 0) throw Error('Preview desktop setup failed: ' + result.stderr);
  }
  start(node, ['--config', path.join(directory, 'preview-node/settings.json'), '--id', 'preview-node', '--name', 'Preview Linux node', '--server', 'ws://127.0.0.1:' + runtimePort, '--token-file', path.join(directory, 'node.token')]);
  const controller = JSON.parse(await fs.readFile(credentials, 'utf8')).controller_token;
  await wait(async () => (await (await fetch('http://127.0.0.1:' + runtimePort + '/api/v1/nodes', { headers: { Authorization: 'Bearer ' + controller } })).json()).nodes.some(node => node.online));
} catch (error) {
  console.error(error.message);
  await stop(1);
}
