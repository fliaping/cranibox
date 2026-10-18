// Real websocket-mode fixture with generated solid-color JPEGs, no desktop data.
import http from 'node:http';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';

export async function selkiesFixture(port) {
  const images = JSON.parse(await readFile(new URL('./selkies-frames.json', import.meta.url), 'utf8')).map(value => Buffer.from(value, 'base64'));
  const clients = new Map();
  const server = http.createServer(async (req, res) => {
    if (req.method !== 'POST' || req.url !== '/clipboard') { res.end(); return; }
    try {
      const chunks = []; let size = 0;
      for await (const chunk of req) { size += chunk.length; if (size > 256 * 1024) throw Error('Fixture body too large'); chunks.push(chunk); }
      const { messages } = JSON.parse(Buffer.concat(chunks));
      if (!Array.isArray(messages) || messages.some(text => typeof text !== 'string')) throw Error('Expected clipboard messages');
      for (const send of clients.values()) for (const message of messages) send(message);
      res.end('ok');
    } catch { res.statusCode = 400; res.end('Invalid fixture request'); }
  });
  server.on('upgrade', (req, socket) => {
    if (req.url !== '/websockets') { socket.destroy(); return; }
    clients.set(socket, () => {}); socket.on('error', () => {}); socket.on('close', () => clients.delete(socket));
    const accept = createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n');
    function send(data, opcode = 1) {
      data = Buffer.from(data); const header = Buffer.alloc(data.length < 126 ? 2 : data.length <= 65535 ? 4 : 10);
      header[0] = 0x80 | opcode; header[1] = data.length < 126 ? data.length : data.length <= 65535 ? 126 : 127;
      if (header.length === 4) header.writeUInt16BE(data.length, 2);
      if (header.length === 10) header.writeBigUInt64BE(BigInt(data.length), 2);
      socket.write(Buffer.concat([header, data]));
    }
    let id = 0, clipboard = Buffer.from('Remote clipboard fixture').toString('base64');
    function picture(typed) {
      const header = Buffer.from([3, 0, 0, ++id, 0, 0]); send(Buffer.concat([header, images[typed ? 1 : 0]]), 2);
    }
    send('MODE websockets');
    let buffer = Buffer.alloc(0);
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length >= 2) {
        const opcode = buffer[0] & 15, masked = !!(buffer[1] & 128);
        let size = buffer[1] & 127, offset = 2;
        if (size === 127) { if (buffer.length < 10) return; const length = buffer.readBigUInt64BE(2); if (length > 256n * 1024n) { socket.destroy(); return; } size = Number(length); offset = 10; }
        if (size === 126) { if (buffer.length < 4) return; size = buffer.readUInt16BE(2); offset = 4; }
        if (buffer.length < offset + (masked ? 4 : 0) + size) return;
        const mask = masked ? buffer.subarray(offset, offset + 4) : null; offset += masked ? 4 : 0;
        const payload = Buffer.from(buffer.subarray(offset, offset + size)); buffer = buffer.subarray(offset + size);
        if (mask) for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
        if (opcode === 8) { send(Buffer.alloc(0), 8); socket.end(); return; }
        if (opcode === 9) { send(payload, 10); continue; }
        if (opcode !== 1) continue;
        const text = payload.toString();
        if (text.startsWith('SETTINGS,')) {
          const settings = JSON.parse(text.slice(9));
          if (settings.encoder !== 'jpeg' || Object.keys(settings).some(k => /width|height/i.test(k))) { socket.destroy(); return; }
          send(JSON.stringify({ type: 'stream_resolution', width: 160, height: 100 })); picture(false);
          clients.set(socket, send);
        }
        if (text.startsWith('cw,')) { clipboard = text.slice(3); send('clipboard,' + clipboard); }
        if (text === 'cr') send('clipboard,' + clipboard);
        if (text === 'kd,97' || text.startsWith('m,') && text.split(',')[3] === '1') picture(true);
      }
    });
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return () => { for (const client of clients.keys()) client.destroy(); server.close(); };
}
