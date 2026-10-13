'use strict';
// No framework or build step: share the extension's renderer and action controller.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFile, spawn } = require('node:child_process');
const createControlCenter = require(fs.existsSync(path.join(__dirname, 'control-center.js'))
  ? './control-center.js' : '../../extensions/control-center/control-center.js');
const ACTIONS = new Set(['installAgent', 'updateOpenClaw', 'installMcpm', 'installDesktop', 'setupComputerUse',
  'desktopControl', 'desktopEmergencyStop', 'setupBrowser', 'openManagedBrowser',
  'installTailscale', 'serveTailscale', 'configureDomain', 'addSkill', 'updateSkills',
  'updateSkill', 'removeSkill', 'addMcp', 'removeMcp', 'serviceAction', 'showLogs',
  'installProxy', 'checkProxy', 'addRoute', 'removeRoute', 'completeOnboarding',
  'updateSource', 'switchLanguage']);
const random = () => crypto.randomBytes(32).toString('hex');
const jsonSafe = value => JSON.parse(JSON.stringify(value));

function createServer(options = {}) {
  const configRoot = options.configRoot || process.env.AGENT_WORKSPACE_CONFIG_ROOT || '/config';
  let saved = {};
  const configFile = path.join(configRoot, '.config/agent-workspace/web.json');
  if (fs.existsSync(configFile)) saved = JSON.parse(fs.readFileSync(configFile, 'utf8'));
  const config = { port: 9080, publicUrl: '', workspaceUrl: 'http://localhost:8443',
    desktopUrl: 'https://localhost:3001', ...saved, ...options };
  const command = config.command || path.join(configRoot, 'bin/workspacectl');
  const origins = new Set([`http://127.0.0.1:${config.port}`, `http://localhost:${config.port}`]);
  if (config.publicUrl) origins.add(new URL(config.publicUrl).origin);
  const hosts = new Set([...origins].map(origin => new URL(origin).host));
  if (config.publicUrl) hosts.add(new URL(config.publicUrl).hostname);
  const sessions = new Map();
  let snapshot, snapshotAt = 0, refreshing, snapshotError = '';

  function refresh() {
    if (refreshing) return refreshing;
    refreshing = new Promise(resolve => {
      execFile(command, ['status', '--json'], { timeout: 35000, maxBuffer: 8 * 1024 * 1024 }, (error, stdout, stderr) => {
        try {
          if (error) throw Error((stderr || error.message).slice(0, 2000));
          const value = JSON.parse(stdout);
          if (value.schema_version !== 1 || !value.summary) throw Error('Invalid workspace snapshot');
          snapshot = value;
          snapshotAt = Date.now();
          snapshotError = '';
        } catch (error) { snapshotError = error.message; }
        resolve(snapshot);
      });
    }).finally(() => { refreshing = undefined; });
    return refreshing;
  }

  function sessionFor(req) {
    const id = /(?:^|;\s*)aw_session=([a-f0-9]{64})(?:;|$)/.exec(req.headers.cookie || '')?.[1];
    const session = sessions.get(id);
    if (session) session.seen = Date.now();
    return session;
  }

  function createSession() {
    if (sessions.size >= 64) throw Object.assign(Error('Too many sessions'), { status: 503 });
    const session = { id: random(), csrf: random(), seq: 0, events: [], output: '', seen: Date.now(), busy: false };
    session.emit = event => {
      if (event.type === 'snapshot' || event.type === 'refreshing') return;
      session.events.push({ ...event, seq: ++session.seq });
      if (session.events.length > 256) session.events.shift();
    };
    const output = {
      appendLine: value => { session.output = (session.output + value + '\n').slice(-128 * 1024); },
      show: () => session.emit({ type: 'output' })
    };
    const notify = (level, message) => session.emit({ type: 'notice', level, message });
    async function ask(kind, payload, validate) {
      return new Promise(resolve => {
        const prompt = { id: random(), kind, ...payload };
        const timer = setTimeout(() => { session.prompt = undefined; resolve(undefined); }, 5 * 60 * 1000);
        timer.unref();
        session.prompt = { view: prompt, validate, resolve: value => {
          clearTimeout(timer); session.prompt = undefined; resolve(value);
        } };
      });
    }
    const host = {
      env: {
        get language() { return snapshot?.locale?.current || readLocale(); },
        openExternal: async url => session.emit({ type: 'openUrl', url: String(url) })
      },
      window: {
        createOutputChannel: () => output,
        createStatusBarItem: () => ({ show() {} }),
        showInputBox: options => ask('input', jsonSafe(options), value => {
          if (typeof value !== 'string') throw Error('Enter text');
          const error = options.validateInput?.(value);
          if (error) throw Error(error);
          return value;
        }),
        showQuickPick: (items, options) => ask('pick', { items, ...options }, value => {
          if (options.canPickMany) {
            if (!Array.isArray(value) || value.some(i => !Number.isInteger(i) || !items[i])) throw Error('Choose valid items');
            return [...new Set(value)].map(i => items[i]);
          }
          if (!Number.isInteger(value) || !items[value]) throw Error('Choose an item');
          return items[value];
        }),
        showWarningMessage: (message, options, ...choices) => options?.modal
          ? ask('confirm', { title: message, choices }, value => {
            if (!choices.includes(value)) throw Error('Invalid choice');
            return value;
          }) : notify('info', message),
        showErrorMessage: message => notify('error', message),
        showInformationMessage: message => notify('info', message),
        withProgress: (_options, action) => action()
      },
      workspace: { getConfiguration: () => ({ get: (key, fallback) => ({
        workspacectlPath: command, desktopUrl: config.desktopUrl
      }[key] ?? fallback) }) },
      commands: { getCommands: async () => [] },
      Uri: { parse: value => value },
      StatusBarAlignment: { Left: 1 }, ProgressLocation: { Notification: 1 }, ThemeColor: class {}
    };
    const shared = createControlCenter(host);
    const controller = new shared.ControlCenter({ subscriptions: [], globalState: { get: () => snapshot } });
    controller.refresh = async () => {
      controller.snapshot = await refresh();
      return controller.snapshot;
    };
    controller.operation = async (title, args, timeout = 15 * 60 * 1000) => {
      output.show();
      output.appendLine(`\n==> ${title}\n$ workspacectl ${args.join(' ')}`);
      const ok = await new Promise(resolve => {
        const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'], detached: true });
        session.child = child;
        let timedOut = false, killTimer;
        const timer = setTimeout(() => {
          timedOut = true;
          try { process.kill(-child.pid, 'SIGTERM'); } catch {}
          killTimer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }, 5000);
        }, timeout);
        const append = chunk => { session.output = (session.output + chunk.toString()).slice(-128 * 1024); };
        child.stdout.on('data', append);
        child.stderr.on('data', append);
        child.once('error', error => { output.appendLine(error.message); });
        child.once('close', code => {
          clearTimeout(timer); clearTimeout(killTimer); session.child = undefined;
          if (timedOut) output.appendLine('Operation timed out.');
          resolve(code === 0 && !timedOut);
        });
      });
      notify(ok ? 'info' : 'error', `${title}: ${ok ? '✓' : 'failed — see output'}`);
      await controller.refresh();
      return ok;
    };
    const switchLanguage = controller.switchLanguage.bind(controller);
    controller.switchLanguage = async () => {
      await switchLanguage();
      await controller.refresh();
    };
    session.controller = controller;
    session.render = () => shared.controlCenterHtml({ standalone: {
      csrf: session.csrf, workspaceUrl: config.workspaceUrl, desktopUrl: config.desktopUrl,
      language: host.env.language, cursor: session.seq
    } }, false);
    sessions.set(session.id, session);
    return session;
  }

  function readLocale() {
    try { return fs.readFileSync(path.join(configRoot, '.local/state/agent-workspace/locale'), 'utf8').trim() === 'zh-cn' ? 'zh-cn' : 'en'; }
    catch { return 'en'; }
  }

  async function readBody(req) {
    if (req.headers['content-type'] !== 'application/json') throw Object.assign(Error('JSON required'), { status: 415 });
    let body = '';
    for await (const chunk of req) {
      body += chunk;
      if (Buffer.byteLength(body) > 16384) throw Object.assign(Error('Request too large'), { status: 413 });
    }
    try { return JSON.parse(body); } catch { throw Object.assign(Error('Invalid JSON'), { status: 400 }); }
  }
  function send(res, status, body, type = 'application/json') {
    res.writeHead(status, { 'Content-Type': `${type}; charset=utf-8` });
    res.end(type === 'application/json' ? JSON.stringify(body) : body);
  }
  const server = http.createServer(async (req, res) => {
    res.setHeader('Cache-Control', 'no-store');
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.setHeader('X-Frame-Options', 'DENY');
    res.setHeader('Referrer-Policy', 'no-referrer');
    try {
      // Do not trust forwarded Host/Origin. Only explicitly configured gateway origins may enter.
      if (!hosts.has(req.headers.host)) return send(res, 403, { error: 'Host not allowed; configure workspacectl web first.' });
      // A link or authentication redirect may enter from another site. Only
      // allow its top-level document navigation, never a cross-site API call.
      const navigation = req.method === 'GET' && req.headers['sec-fetch-mode'] === 'navigate'
        && req.headers['sec-fetch-dest'] === 'document';
      if ((req.headers['sec-fetch-site'] === 'cross-site' && !navigation) || (req.headers.origin && !origins.has(req.headers.origin)))
        return send(res, 403, { error: 'Cross-origin request rejected' });
      const url = new URL(req.url, 'http://localhost');
      if (req.method === 'GET' && url.pathname === '/healthz') return send(res, 200, { ok: true });
      if (req.method === 'GET' && ['/web.js', '/web.css'].includes(url.pathname))
        return send(res, 200, fs.readFileSync(path.join(__dirname, url.pathname.slice(1)), 'utf8'),
          url.pathname.endsWith('.js') ? 'text/javascript' : 'text/css');
      if (req.method === 'GET' && url.pathname === '/') {
        const session = sessionFor(req) || createSession();
        const secure = config.publicUrl && [new URL(config.publicUrl).host, new URL(config.publicUrl).hostname].includes(req.headers.host) && config.publicUrl.startsWith('https:');
        res.setHeader('Set-Cookie', `aw_session=${session.id}; HttpOnly; SameSite=Strict; Path=/${secure ? '; Secure' : ''}`);
        if (!snapshot || Date.now() - snapshotAt > 15000) void refresh();
        return send(res, 200, session.render(), 'text/html');
      }
      const session = sessionFor(req);
      if (!session || req.headers['x-workspace-csrf'] !== session.csrf) return send(res, 401, { error: 'Session expired. Reload this page.' });
      if (req.method !== 'POST') return send(res, 405, { error: 'POST required' });
      const body = await readBody(req);
      if (!body || typeof body !== 'object' || Array.isArray(body)) return send(res, 400, { error: 'Object required' });
      if (url.pathname === '/api/poll') {
        if (body.refresh || !snapshot || Date.now() - snapshotAt > 15000) void refresh();
        return send(res, 200, { snapshot, snapshotAt, refreshing: Boolean(refreshing), error: snapshotError,
          busy: session.busy, forceWelcome: session.controller.forceWelcome,
          prompt: session.prompt?.view, output: session.output, cursor: session.seq,
          events: session.events.filter(event => event.seq > (Number(body.cursor) || 0)) });
      }
      if (url.pathname === '/api/answer') {
        const prompt = session.prompt;
        if (!prompt || body.id !== prompt.view.id) return send(res, 409, { error: 'Prompt expired' });
        try { prompt.resolve(body.cancel ? undefined : prompt.validate(body.value)); }
        catch (error) { return send(res, 400, { error: error.message }); }
        return send(res, 200, { ok: true });
      }
      if (url.pathname === '/api/action') {
        if (!ACTIONS.has(body.type)) return send(res, 400, { error: 'Unsupported action' });
        // Emergency stop remains usable while an installation or dialog is pending.
        if (body.type === 'desktopEmergencyStop') {
          execFile(command, ['desktop', 'emergency-stop'], { timeout: 30000 }, (error, stdout, stderr) => {
            session.output = (session.output + '\n' + stdout + stderr).slice(-128 * 1024);
            session.emit({ type: 'notice', level: error ? 'error' : 'info', message: error ? error.message : 'Computer Use: input stopped' });
            void refresh();
          });
          return send(res, 202, { ok: true });
        }
        for (const field of ['id', 'name']) {
          if (body[field] !== undefined && (typeof body[field] !== 'string' || !/^[A-Za-z0-9_@][A-Za-z0-9_.@-]*$/.test(body[field])))
            return send(res, 400, { error: `Invalid ${field}` });
        }
        if (session.busy) return send(res, 409, { error: 'An operation is already running. See output.' });
        session.busy = true;
        session.controller.snapshot = snapshot;
        void session.controller.handleMessage(body).catch(error => session.emit({ type: 'notice', level: 'error', message: error.message }))
          .finally(() => { session.busy = false; });
        return send(res, 202, { ok: true });
      }
      return send(res, 404, { error: 'Not found' });
    } catch (error) { send(res, error.status || 500, { error: error.message }); }
  });
  server.requestTimeout = 10000;
  server.headersTimeout = 10000;
  const cleanup = setInterval(() => {
    for (const [id, session] of sessions) {
      if (!session.busy && Date.now() - session.seen > 12 * 60 * 60 * 1000) sessions.delete(id);
    }
  }, 60000);
  cleanup.unref();
  server.on('close', () => {
    clearInterval(cleanup);
    for (const session of sessions.values()) session.prompt?.resolve(undefined);
  });
  server.workspaceConfig = config;
  return server;
}

module.exports = { createServer };
if (require.main === module) {
  const server = createServer();
  server.listen(server.workspaceConfig.port, '127.0.0.1', () => {
    console.log(`Agent Workspace Web: http://127.0.0.1:${server.workspaceConfig.port}`);
  });
}
