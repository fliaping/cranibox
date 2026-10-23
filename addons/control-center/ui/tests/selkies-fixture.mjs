// Selkies 2.x shape: its own page, API and websocket stream share one port.
import http from 'node:http';
import { createHash } from 'node:crypto';

const PAGE = `<!doctype html><html><head><meta charset="utf-8"><title>Selkies fixture</title></head>
<body style="background:#11151c;color:#fff;font:16px sans-serif">
<h1>Selkies fixture</h1><p id="mode">connecting</p><p id="status"></p><p id="input"></p>
<script>
const socket = new WebSocket(new URL('api/websockets', location.href).href.replace(/^http/, 'ws'));
socket.onmessage = event => { const text = String(event.data); document.getElementById(text.startsWith('input:') ? 'input' : 'mode').textContent = text; };
addEventListener('keydown', event => socket.send('kd,' + event.key));
fetch('api/status').then(response => response.json()).then(value => { document.getElementById('status').textContent = 'status ' + value.ok; });
</script></body></html>`;

export async function selkiesFixture(port) {
  const clients = new Set();
  const server = http.createServer((req, res) => {
    if (req.url === '/') { res.setHeader('content-type', 'text/html'); res.end(PAGE); return; }
    if (req.url === '/api/status') { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ ok: true })); return; }
    res.statusCode = 404; res.end();
  });
  server.on('upgrade', (req, socket) => {
    if (req.url !== '/api/websockets' && req.url !== '/websockets') { socket.destroy(); return; }
    clients.add(socket); socket.on('error', () => {}); socket.on('close', () => clients.delete(socket));
    const accept = createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n');
    function send(data, opcode = 1) {
      data = Buffer.from(data); const header = Buffer.alloc(data.length < 126 ? 2 : 4);
      header[0] = 0x80 | opcode; header[1] = data.length < 126 ? data.length : 126;
      if (header.length === 4) header.writeUInt16BE(data.length, 2);
      socket.write(Buffer.concat([header, data]));
    }
    send('MODE websockets');
    let buffer = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 2) {
        const opcode = buffer[0] & 15, masked = !!(buffer[1] & 128);
        let size = buffer[1] & 127, offset = 2;
        if (size === 127) { socket.destroy(); return; }
        if (size === 126) { if (buffer.length < 4) return; size = buffer.readUInt16BE(2); offset = 4; }
        if (buffer.length < offset + (masked ? 4 : 0) + size) return;
        const mask = masked ? buffer.subarray(offset, offset + 4) : null; offset += masked ? 4 : 0;
        const payload = Buffer.from(buffer.subarray(offset, offset + size)); buffer = buffer.subarray(offset + size);
        if (mask) for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
        if (opcode === 8) { send(Buffer.alloc(0), 8); socket.end(); return; }
        if (opcode === 9) { send(payload, 10); continue; }
        if (opcode === 1) send('input:' + payload.toString());
      }
    });
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return () => { for (const client of clients) client.destroy(); server.close(); };
}
