import type { DesktopChannel } from './transport';

export const SELKIES_MAGIC = new TextEncoder().encode('CRANIBOX-SELKIES/1\n');
/** Preserve the first bytes while choosing the viewer. This also works with the
 * Tauri IPC channel, where a native WebSocket URL is deliberately unavailable. */
export function openDesktop(transport: string | DesktopChannel, signal: AbortSignal): Promise<{ channel: DesktopChannel; protocol: 'vnc' | 'selkies' }> {
  const socket = typeof transport === 'string' ? new WebSocket(transport) as DesktopChannel : transport;
  socket.binaryType = 'arraybuffer';
  return new Promise((resolve, reject) => {
    let settled = false, bytes = new Uint8Array(), queued: ArrayBuffer[] = [], size = 0;
    let listener: DesktopChannel['onmessage'] = null;
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', abort); };
    const fail = (error: Error) => {
      cleanup(); queued = []; size = 0; socket.close();
      if (!settled) { settled = true; reject(error); }
      else { channel.error = error.message; channel.onclose?.(new CloseEvent('close', { wasClean: false })); }
    };
    const abort = () => fail(new DOMException('Desktop connection cancelled', 'AbortError'));
    const timer = setTimeout(() => fail(Error('Desktop server did not report its protocol. Check sharing and reconnect.')), 10000);
    const flush = () => {
      while (listener && queued.length) { const data = queued.shift()!; size -= data.byteLength; listener(new MessageEvent('message', { data })); }
    };
    const channel: DesktopChannel = {
      get readyState() { return socket.readyState; },
      get binaryType() { return socket.binaryType; }, set binaryType(value) { socket.binaryType = value; },
      get protocol() { return socket.protocol; },
      get onmessage() { return listener; }, set onmessage(value) { listener = value; queueMicrotask(flush); },
      onopen: null, onclose: null, onerror: null,
      send(data) { socket.send(data); },
      close() { cleanup(); queued = []; size = 0; socket.onmessage = null; socket.onclose = null; socket.onerror = null; socket.close(); },
    };
    socket.onmessage = event => {
      if (!(event.data instanceof ArrayBuffer)) { fail(Error('Invalid desktop data')); return; }
      let data = event.data;
      if (!settled) {
        const merged = new Uint8Array(bytes.length + data.byteLength); merged.set(bytes); merged.set(new Uint8Array(data), bytes.length); bytes = merged;
        let protocol: 'vnc' | 'selkies';
        if (bytes.length < 4) return;
        if (bytes.length >= 4 && new TextDecoder().decode(bytes.subarray(0, 4)) === 'RFB ') { protocol = 'vnc'; data = bytes.buffer; }
        else {
          if (!bytes.subarray(0, Math.min(bytes.length, SELKIES_MAGIC.length)).every((b, i) => b === SELKIES_MAGIC[i])) { fail(Error('Unsupported desktop protocol')); return; }
          if (bytes.length < SELKIES_MAGIC.length) return;
          protocol = 'selkies'; data = bytes.slice(SELKIES_MAGIC.length).buffer;
        }
        settled = true; cleanup(); resolve({ channel, protocol }); bytes = new Uint8Array();
      }
      if (data.byteLength) { queued.push(data); size += data.byteLength; }
      if (size > 2 * 1024 * 1024) { fail(Error('Desktop viewer is not consuming data')); return; }
      flush();
    };
    socket.onclose = event => {
      cleanup();
      if (!settled) { settled = true; reject(Error(socket.error || 'Desktop connection closed before opening')); }
      else { channel.error ||= socket.error; channel.onclose?.(event); }
    };
    socket.onerror = () => { channel.error = socket.error || 'Desktop connection failed'; channel.onerror?.(new Event('error')); };
    socket.onopen = event => channel.onopen?.(event);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
  });
}
