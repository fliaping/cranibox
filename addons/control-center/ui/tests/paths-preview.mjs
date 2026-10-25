// Fixture-only preview: no node processes, credentials, or external services.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';

const fixtures = new URL('../../../../crates/cranibox-contracts/fixtures/', import.meta.url);
const assets = new URL('../../', import.meta.url);
const fixture = async name => JSON.parse(await readFile(new URL(name + '.json', fixtures), 'utf8'));
const appFixtures = {
  '/status': 'app_status', '/devices': 'app_devices', '/approvals': 'app_approvals',
  '/agents': 'app_agents', '/models/models': 'llm_models', '/models/accounts': 'llm_accounts',
};
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://127.0.0.1').pathname;
  const send = (value, status = 200) => {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(value));
  };
  try {
    if (path === '/healthz') return send({ ok: true });
    if (path === '/') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      return res.end('<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/web.css"></head><body><div id="root"></div><script type="module" src="/web.js"></script></body></html>');
    }
    if (path === '/web.js' || path === '/web.css') {
      res.writeHead(200, { 'content-type': path.endsWith('.js') ? 'text/javascript' : 'text/css' });
      return res.end(await readFile(new URL(path.slice(1), assets)));
    }
    if (path === '/api/network') return send({ iroh: { lan: true, relays: [], n0_fallback: true, learned_relays: [], this_relay: { enabled: false, public_url: null } } });
    const app = path.replace(/^\/api\/v1\/app/, '');
    if (app === '/network/paths') {
      const value = await fixture('app_network_paths');
      for (const path of value.paths) path.at = Date.now() - (1_800_000_000_000 - path.at);
      return send(value);
    }
    if (appFixtures[app]) return send(await fixture(appFixtures[app]));
    if (app === '/models/presets') return send([]);
    if (app === '/events') { res.writeHead(204); return res.end(); }
    return send({ error: 'No fixture for ' + path }, 404);
  } catch (error) { send({ error: String(error) }, 500); }
});
server.listen(19084, '127.0.0.1');
process.on('SIGTERM', () => server.close(() => process.exit()));
