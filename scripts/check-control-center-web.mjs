import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'workspace-web-test-'));
const command = path.join(directory, 'workspacectl');
const calls = path.join(directory, 'calls');
await fs.writeFile(command, [
  '#!' + process.execPath,
  "require('fs').appendFileSync(" + JSON.stringify(calls) + ", JSON.stringify(process.argv.slice(2)) + '|');",
  "if (process.argv[2] === 'status') setTimeout(() => console.log(JSON.stringify({",
  "schema_version: 1, summary: {healthy:true}, locale:{current:'en'},",
  "mcp:{agents:[{id:'codex',label:'Codex',installed:true,configured:0}]}",
  '})), 200);',
  "else if (process.argv[2] === 'logs' && process.argv[3] === 'long-job') { require('fs').writeFileSync(" + JSON.stringify(path.join(directory,'long-job.pid')) + ", String(process.pid)); console.log('long job started'); setInterval(() => {}, 1000); }",
  "else { console.log('started ' + process.argv.slice(2).join(' ')); setTimeout(() => console.log('done'), 400); }"
].join('\n'), { mode: 0o700 });
const binary = process.env.WORKSPACE_WEB_BIN || path.resolve('target/debug/workspace-web');
const server = spawn(binary, ['--config-root', directory, '--command', command, '--port', '0', '--public-url', 'https://workspace.example.com'], {stdio:['ignore','pipe','pipe']});
let stderr = '';
server.stderr.on('data', chunk => { stderr += chunk; });
const base = await new Promise((resolve, reject) => {
  let text = '';
  const timer = setTimeout(() => reject(Error('Startup timeout: ' + stderr)), 10000);
  server.once('exit', code => { clearTimeout(timer); reject(Error('Server exited ' + code + ': ' + stderr)); });
  server.stdout.on('data', chunk => {
    text += chunk;
    const match = text.match(/http:\/\/127\.0\.0\.1:\d+/);
    if (match) { clearTimeout(timer); resolve(match[0]); }
  });
});
let cookie, csrf;
async function request(route, body, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request(base + route, { method: body === undefined ? 'GET' : 'POST',
    headers: { Host: 'localhost:0', Cookie: cookie || '', 'X-Workspace-CSRF': csrf || '',
      'Content-Type': 'application/json', ...headers } }, res => {
      let text = '';
      res.on('data', chunk => { text += chunk; });
      res.on('end', () => resolve({ status: res.statusCode, text: async () => text,
        json: async () => JSON.parse(text), headers: { get: name => {
          const value = res.headers[name.toLowerCase()]; return Array.isArray(value) ? value[0] : value;
        } } }));
    });
    req.on('error', reject);
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
}
async function until(check) {
  const end = Date.now() + 8000;
  while (Date.now() < end) { const value = await check(); if (value) return value; await new Promise(r => setTimeout(r, 50)); }
  throw Error('Timed out');
}
const poll = async () => (await request('/api/poll', {})).json();
try {
  assert.equal((await request('/', undefined, { Host: 'attacker.example' })).status, 403);
  assert.equal((await request('/', undefined, { Origin: 'https://evil.example' })).status, 403);
  assert.equal((await request('/', undefined, { 'Sec-Fetch-Site': 'cross-site', 'Sec-Fetch-Mode': 'navigate', 'Sec-Fetch-Dest': 'document' })).status, 200);
  assert.equal((await request('/api/poll', {}, { 'Sec-Fetch-Site': 'cross-site' })).status, 403);
  assert.equal((await request('/api/poll', {})).status, 401);
  const page = await request('/');
  const html = await page.text();
  cookie = page.headers.get('set-cookie').split(';')[0];
  csrf = JSON.parse(html.split('window.workspaceWeb=')[1].split(';</script>')[0]).csrf;
  assert.ok(html.includes('/web.js'));
  assert.ok(html.includes('href="/web.css"'));
  assert.ok(html.includes('id="root"'), 'React mount point is present');
  assert.ok(html.includes('src="/web.js" defer'), 'compiled UI loads after boot configuration');
  assert.equal((await request('/web.js')).status, 200);
  assert.equal((await request('/web.css')).status, 200);
  assert.equal((await request('/api/poll', {}, { 'X-Workspace-CSRF': 'wrong' })).status, 401);
  assert.equal((await request('/api/action', {type:'terminal',command:'touch /tmp/unsafe'})).status, 400);
  assert.equal((await request('/api/action', {type:'serviceAction',id:'--help'})).status, 400);
  assert.equal((await request('/api/action', {type:'showLogs',id:'../../etc/passwd'})).status, 400);
  const state = await until(async () => { const s = await poll(); return s.snapshot && s; });
  assert.equal(state.snapshot.summary.healthy, true);
  await Promise.all(Array.from({ length: 8 }, () => poll()));
  assert.equal((await fs.readFile(calls, 'utf8')).split('|').filter(Boolean).length, 1, 'snapshot collection is coalesced and cached');
  await request('/api/action', { type: 'addRoute' });
  const prompt = await until(async () => (await poll()).prompt);
  assert.equal(prompt.kind, 'input');
  assert.equal((await request('/api/answer', { id: prompt.id, value: '--help' })).status, 400);
  assert.equal((await request('/api/answer', { id: prompt.id, cancel: true })).status, 200);
  await until(async () => !(await poll()).busy);
  await request('/api/action', { type:'serviceAction', id:'test-service', kind:'systemd', action:'start' });
  assert.equal((await request('/api/action', { type:'checkProxy' })).status, 409);
  assert.equal((await request('/')).headers.get('set-cookie').split(';')[0], cookie);
  const done = await until(async () => { const s = await poll(); return !s.busy && s; });
  assert.match(done.output, /started service start test-service/);
  assert.match(done.output, /done/);
  assert.equal((await request('/api/action', { type: 'updateOpenClaw' })).status, 202);
  const updatePrompt = await until(async () => (await poll()).prompt);
  assert.equal(updatePrompt.kind, 'confirm');
  assert.equal((await request('/api/answer', { id: updatePrompt.id, value: 'Update Gateway' })).status, 200);
  await until(async () => (await fs.readFile(calls, 'utf8')).includes('["agent","update","openclaw"]'));
  await until(async () => !(await poll()).busy);
  await request('/api/action', {type:'addRoute'});
  await until(async () => (await poll()).prompt);
  assert.equal((await request('/api/cancel', {})).status, 200);
  await until(async () => !(await poll()).busy);
  assert.equal((await request('/api/action', {type:'desktopEmergencyStop'})).status, 202);
  await until(async () => (await fs.readFile(calls, 'utf8')).includes('["desktop","emergency-stop"]'));
  await request('/api/action', {type:'showLogs', id:'long-job'});
  const pidFile = path.join(directory, 'long-job.pid');
  await until(async () => fs.readFile(pidFile, 'utf8').catch(() => false));
  const jobPid = Number(await fs.readFile(pidFile, 'utf8'));
  assert.equal((await request('/api/cancel', {})).status, 200);
  await until(async () => !(await poll()).busy);
  await until(() => { try { process.kill(jobPid, 0); return false; } catch { return true; } });
  const other = await request('/', undefined, { Cookie: '' });
  assert.notEqual(other.headers.get('set-cookie').split(';')[0], cookie);
  await request('/api/action', { type: 'addRoute' });
  assert.equal((await request('/api/action', { type: 'desktopEmergencyStop' })).status, 202);
  const pending = await until(async () => (await poll()).prompt);
  await request('/api/answer', { id: pending.id, cancel: true });
  await until(async () => (await fs.readFile(calls, 'utf8')).includes('["desktop","emergency-stop"]'));
  await new Promise(r => setTimeout(r, 700));
  console.log('Web Control Center: access guards, React assets, snapshot cache, prompts, jobs, reload recovery and emergency stop OK');
} finally {
  server.kill('SIGTERM');
  await new Promise(resolve => server.once('exit', resolve));
  await fs.rm(directory, { recursive: true, force: true });
}
