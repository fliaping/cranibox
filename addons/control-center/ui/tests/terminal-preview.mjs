import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';

// Serve the checked-in production bundles; each test mocks the application API.
const assets = new URL('../../', import.meta.url);
createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/healthz') { response.end('ok'); return; }
  if (url.pathname === '/web.js' || url.pathname === '/web.css') {
    try {
      const content = await readFile(new URL(url.pathname.slice(1), assets));
      response.setHeader('Content-Type', url.pathname.endsWith('.js') ? 'text/javascript' : 'text/css');
      response.end(content);
    } catch { response.writeHead(404).end(); }
    return;
  }
  if (url.pathname !== '/') { response.writeHead(404).end(); return; }
  const language = url.searchParams.get('lang') === 'zh-cn' ? 'zh-cn' : 'en';
  response.setHeader('Content-Type', 'text/html; charset=utf-8');
  response.end('<!doctype html><html lang="' + language + '"><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/web.css"></head><body><div id="root"></div><script>window.workspaceWeb=' + JSON.stringify({ csrf: '', language, cursor: 0, workspaceUrl: '', desktopUrl: '', edition: 'node' }) + '</script><script type="module" src="/web.js"></script></body></html>');
}).listen(19084, '127.0.0.1');
