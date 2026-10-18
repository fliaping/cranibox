// Minimal RFB 3.8 server for exercising the real noVNC canvas and input path.
import net from 'node:net';
import { createDecipheriv, createHash, getDiffieHellman } from 'node:crypto';

export async function rfbFixture(port, credentials) {
  const connections = new Set();
  const server = net.createServer(socket => {
    connections.add(socket);
    socket.on('error', () => {});
    socket.on('close', () => connections.delete(socket));
    let buffer = Buffer.alloc(0), phase = 'version', dirty = true, typed = false, red = 2, green = 1, blue = 0;
    let exchange;
    socket.write('RFB 003.008\n');
    function frame() {
      const header = Buffer.alloc(16);
      header.writeUInt16BE(1, 2); header.writeUInt16BE(160, 8); header.writeUInt16BE(100, 10);
      const pixels = Buffer.alloc(160 * 100 * 4);
      for (let index = 0; index < pixels.length; index += 4) {
        pixels[index + blue] = typed ? 56 : 83;
        pixels[index + green] = typed ? 100 : 139;
        pixels[index + red] = typed ? 212 : 64;
      }
      socket.write(Buffer.concat([header, pixels])); dirty = false;
    }
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      while (buffer.length) {
        if (phase === 'version') {
          if (buffer.length < 12) return;
          buffer = buffer.subarray(12); phase = 'security'; socket.write(Buffer.from([1, credentials ? 30 : 1]));
        } else if (phase === 'security') {
          const method = buffer[0]; buffer = buffer.subarray(1);
          if (method !== (credentials ? 30 : 1)) { socket.destroy(); return; }
          if (credentials) {
            // Apple DH is the username/password method also offered by wayvnc.
            exchange = getDiffieHellman('modp14');
            const publicKey = exchange.generateKeys(), prime = exchange.getPrime();
            const header = Buffer.alloc(4); header.writeUInt16BE(2, 0); header.writeUInt16BE(prime.length, 2);
            socket.write(Buffer.concat([header, prime, publicKey])); phase = 'auth';
          } else { phase = 'init'; socket.write(Buffer.alloc(4)); }
        } else if (phase === 'auth') {
          const keyLength = exchange.getPrime().length;
          if (buffer.length < 128 + keyLength) return;
          let accepted = false;
          try {
            const shared = exchange.computeSecret(buffer.subarray(128, 128 + keyLength));
            const key = createHash('md5').update(shared).digest();
            const cipher = createDecipheriv('aes-128-ecb', key, null); cipher.setAutoPadding(false);
            const plaintext = Buffer.concat([cipher.update(buffer.subarray(0, 128)), cipher.final()]);
            const text = start => plaintext.subarray(start, start + 64).toString('utf8').split('\0')[0];
            accepted = text(0) === credentials.username && text(64) === credentials.password;
          } catch { /* Invalid DH values or credentials fail authentication. */ }
          buffer = buffer.subarray(128 + keyLength);
          if (!accepted) {
            const reason = Buffer.from('Invalid fixture credentials'), header = Buffer.alloc(8);
            header.writeUInt32BE(1, 0); header.writeUInt32BE(reason.length, 4);
            socket.end(Buffer.concat([header, reason])); return;
          }
          phase = 'init'; socket.write(Buffer.alloc(4));
        } else if (phase === 'init') {
          buffer = buffer.subarray(1); phase = 'messages';
          const name = Buffer.from('Cranibox RFB fixture'), header = Buffer.alloc(24);
          header.writeUInt16BE(160, 0); header.writeUInt16BE(100, 2);
          Buffer.from([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]).copy(header, 4);
          header.writeUInt32BE(name.length, 20); socket.write(Buffer.concat([header, name]));
        } else {
          const type = buffer[0];
          let size;
          if (type === 0) size = 20;
          else if (type === 2) { if (buffer.length < 4) return; size = 4 + 4 * buffer.readUInt16BE(2); }
          else if (type === 3) size = 10;
          else if (type === 4) size = 8;
          else if (type === 5) size = 6;
          else if (type === 6) { if (buffer.length < 8) return; size = 8 + buffer.readUInt32BE(4); }
          else { socket.destroy(); return; }
          if (buffer.length < size) return;
          if (type === 0) { red = buffer[14] / 8; green = buffer[15] / 8; blue = buffer[16] / 8; }
          if (type === 4 && buffer[1] === 1) { typed = true; dirty = true; frame(); }
          if (type === 3 && (dirty || buffer[1] === 0)) frame();
          if (type === 6) {
            const header = Buffer.alloc(8); header[0] = 3; header.writeUInt32BE(size - 8, 4);
            socket.write(Buffer.concat([header, buffer.subarray(8, size)]));
          }
          buffer = buffer.subarray(size);
        }
      }
    });
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return () => { for (const socket of connections) socket.destroy(); server.close(); };
}
