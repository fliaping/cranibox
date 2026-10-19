// The small display/input subset of Selkies websocket mode. No upstream web app
// or codecs are bundled: browser JPEG decoding handles the server's damage stripes.
// @ts-expect-error noVNC publishes its keyboard helper without TypeScript definitions.
import Keyboard from '@cranibox/novnc-keyboard';
import type { DesktopChannel } from './transport';
import { clipboardBytes, decodeClipboard, encodeClipboard, MAX_CLIPBOARD_BYTES } from './clipboard';

const MAX_FRAME = 16 * 1024 * 1024;
export class SelkiesViewer extends EventTarget {
  scaleViewport = true; resizeSession = false; focusOnClick = true; qualityLevel = 6; compressionLevel = 2;
  private canvas = document.createElement('canvas');
  private context: CanvasRenderingContext2D;
  private keyboard: { grab(): void; ungrab(): void; onkeyevent: (keysym: number, code: string, down: boolean) => void };
  private inputDisabled = false;
  private closed = false;
  private closing = false;
  private connected = false;
  private settingsSent = false;
  private header = new Uint8Array(5);
  private headerUsed = 0;
  private payload = new Uint8Array();
  private payloadUsed = 0;
  private queue: { kind: number; data: Uint8Array }[] = [];
  private queuedBytes = 0;
  private decoding = false;
  private buttons = 0;
  private keys = new Set<number>();
  private point = { x: 0, y: 0 };
  private pendingInput: Uint8Array[] = [];
  private pendingInputBytes = 0;
  private inputScheduled = false;
  private timer: ReturnType<typeof setTimeout>;
  private events = new AbortController();
  private clipboardParts: { bytes: Uint8Array[]; size: number; expected: number } | null = null;
  private clipboardTimer: ReturnType<typeof setTimeout> | undefined;
  private clipboardSecret = false;
  constructor(private host: HTMLElement, private channel: DesktopChannel) {
    super();
    const context = this.canvas.getContext('2d', { alpha: false });
    if (!context) throw Error('This browser does not support canvas desktop display');
    this.context = context; this.canvas.width = 0; this.canvas.height = 0;
    this.canvas.tabIndex = 0; this.canvas.setAttribute('aria-label', 'Remote desktop screen'); this.canvas.className = 'selkies-screen';
    this.host.append(this.canvas);
    this.keyboard = new Keyboard(this.canvas);
    this.keyboard.onkeyevent = (keysym, _code, down) => this.sendKey(keysym, undefined, down); this.keyboard.grab();
    const options = { signal: this.events.signal };
    this.canvas.addEventListener('pointerdown', e => {
      if (this.viewOnly) return; e.preventDefault(); this.canvas.focus({ preventScroll: true });
      this.canvas.setPointerCapture(e.pointerId); this.pointer(e);
    }, options);
    this.canvas.addEventListener('pointermove', e => { if (!this.viewOnly) this.pointer(e); }, options);
    this.canvas.addEventListener('pointerup', e => { if (!this.viewOnly) this.pointer(e); }, options);
    this.canvas.addEventListener('pointercancel', () => this.release(), options);
    this.canvas.addEventListener('lostpointercapture', () => { if (this.buttons) this.release(); }, options);
    this.canvas.addEventListener('contextmenu', e => e.preventDefault(), options);
    this.canvas.addEventListener('wheel', e => {
      if (this.viewOnly) return; e.preventDefault();
      const mask = e.deltaY < 0 ? 8 : 16; this.send(`m,${this.point.x},${this.point.y},${this.buttons | mask},1`); this.mouse();
    }, { ...options, passive: false });
    this.canvas.addEventListener('blur', () => this.release(), options);
    window.addEventListener('blur', () => this.release(), options);
    document.addEventListener('visibilitychange', () => { if (document.hidden) this.release(); }, options);
    this.timer = setTimeout(() => this.fail('Selkies did not deliver a JPEG desktop. Check its websocket mode and encoder configuration, then reconnect.'), 15000);
    channel.onmessage = event => {
      try { this.receive(new Uint8Array(event.data as ArrayBuffer)); } catch (error) { this.fail(error instanceof Error ? error.message : String(error)); }
    };
    channel.onclose = () => this.close(false);
    channel.onerror = () => this.fail(channel.error || 'Selkies connection failed');
  }
  get viewOnly() { return this.inputDisabled; }
  set viewOnly(value: boolean) { if (value && !this.inputDisabled) this.release(); this.inputDisabled = value; }
  focus() { this.canvas.focus({ preventScroll: true }); }
  sendCredentials() { /* Selkies is authenticated by the node's desktop grant. */ }
  sendKey(keysym: number, _code?: string, down?: boolean) {
    if (this.closed || this.viewOnly || !this.connected || !Number.isInteger(keysym)) return;
    if (down === undefined) { this.sendKey(keysym, undefined, true); this.sendKey(keysym, undefined, false); return; }
    if (down) this.keys.add(keysym); else this.keys.delete(keysym); this.send(`${down ? 'kd' : 'ku'},${keysym}`);
  }
  sendCtrlAltDel() {
    for (const key of [0xffe3, 0xffe9, 0xffff]) this.sendKey(key, undefined, true);
    for (const key of [0xffff, 0xffe9, 0xffe3]) this.sendKey(key, undefined, false);
  }
  disconnect() { this.close(true); }
  clipboardPasteFrom(text: string) {
    if (this.closed || !this.connected || this.viewOnly) throw Error('Desktop input is unavailable');
    this.send('cw,' + encodeClipboard(text));
  }
  requestClipboard() { if (!this.closed && this.connected) this.send('cr'); }
  private pointer(e: PointerEvent) {
    if (!this.connected || !this.canvas.width) return;
    const rect = this.canvas.getBoundingClientRect();
    this.point = { x: Math.max(0, Math.min(this.canvas.width - 1, Math.floor((e.clientX - rect.left) * this.canvas.width / rect.width))), y: Math.max(0, Math.min(this.canvas.height - 1, Math.floor((e.clientY - rect.top) * this.canvas.height / rect.height))) };
    this.buttons = (e.buttons & 1) | ((e.buttons & 4) >> 1) | ((e.buttons & 2) << 1); this.mouse();
  }
  private mouse() { this.send(`m,${this.point.x},${this.point.y},${this.buttons},0`); }
  private release() {
    if (this.closed) return;
    for (const key of this.keys) this.send(`ku,${key}`); this.keys.clear();
    if (this.buttons) { this.buttons = 0; this.mouse(); }
  }
  private send(text: string) {
    if (this.closed || this.channel.readyState !== 1) return;
    try {
      const body = new TextEncoder().encode(text), data = new Uint8Array(5 + body.length);
      new DataView(data.buffer).setUint32(1, body.length); data.set(body, 5);
      // IME/paste can produce hundreds of key events in one task. Carry their
      // framed commands together so the bounded node channel receives chunks,
      // rather than one queue entry for every key press and release.
      if (this.pendingInputBytes + data.length > 64 * 1024) this.flushInput();
      if (this.closed || this.channel.readyState !== 1) return;
      this.pendingInput.push(data); this.pendingInputBytes += data.length;
      if (!this.inputScheduled) {
        this.inputScheduled = true;
        queueMicrotask(() => { this.inputScheduled = false; this.flushInput(); });
      }
    } catch (error) { this.fail(error instanceof Error ? error.message : 'Desktop input failed'); }
  }
  private flushInput() {
    if (!this.pendingInputBytes) return;
    const data = new Uint8Array(this.pendingInputBytes); let offset = 0;
    for (const part of this.pendingInput) { data.set(part, offset); offset += part.length; }
    this.pendingInput = []; this.pendingInputBytes = 0;
    if (this.closed || this.channel.readyState !== 1) return;
    try { for (let offset = 0; offset < data.length; offset += 64 * 1024) this.channel.send(data.subarray(offset, offset + 64 * 1024)); }
    catch (error) { this.fail(error instanceof Error ? error.message : 'Desktop input failed'); }
  }
  private receive(chunk: Uint8Array) {
    while (chunk.length && !this.closed) {
      if (this.headerUsed < 5) {
        const n = Math.min(5 - this.headerUsed, chunk.length); this.header.set(chunk.subarray(0, n), this.headerUsed); this.headerUsed += n; chunk = chunk.subarray(n);
        if (this.headerUsed < 5) return;
        const size = new DataView(this.header.buffer).getUint32(1);
        if (this.header[0] > 1 || size > MAX_FRAME) throw Error('Invalid Selkies frame size or type');
        this.payload = new Uint8Array(size); this.payloadUsed = 0;
      }
      const n = Math.min(this.payload.length - this.payloadUsed, chunk.length); this.payload.set(chunk.subarray(0, n), this.payloadUsed); this.payloadUsed += n; chunk = chunk.subarray(n);
      if (this.payloadUsed === this.payload.length) {
        if (this.header[0] === 0) {
          const text = new TextDecoder('utf-8', { fatal: true }).decode(this.payload);
          if (text.startsWith('clipboard')) {
            // Clipboard controls are independent of the video resolution and
            // must not wait for an asynchronous JPEG decode. Keep their wire
            // order, including multipart and secret markers.
            this.headerUsed = 0; this.clipboardMessage(text); continue;
          }
        }
        this.queuedBytes += this.payload.length;
        if (this.queue.length >= 256 || this.queuedBytes > 2 * MAX_FRAME) throw Error('Selkies decoder cannot keep up. Reconnect using a lower server frame rate.');
        this.queue.push({ kind: this.header[0], data: this.payload }); this.headerUsed = 0;
        void this.decode();
      }
    }
  }
  private async decode() {
    if (this.decoding) return; this.decoding = true;
    try {
      while (this.queue.length && !this.closed) {
        const frame = this.queue.shift()!; this.queuedBytes -= frame.data.length;
        if (frame.kind === 0) { this.message(new TextDecoder('utf-8', { fatal: true }).decode(frame.data)); continue; }
        const data = frame.data;
        if (data[0] === 1) continue; // Audio is outside this viewer.
        if (!this.settingsSent) continue; // A superseded upstream stream may still be draining.
        if (data[0] !== 3) throw Error('Selkies must support the JPEG encoder for this viewer');
        if (data.length < 7 || !this.canvas.width) throw Error('Selkies sent an invalid JPEG stripe or resolution');
        const view = new DataView(data.buffer, data.byteOffset, data.byteLength), id = view.getUint16(2), y = view.getUint16(4);
        const bitmap = await createImageBitmap(new Blob([data.slice(6).buffer], { type: 'image/jpeg' }));
        try {
          if (this.closed) return;
          if (bitmap.width !== this.canvas.width || y + bitmap.height > this.canvas.height) throw Error('Selkies JPEG stripe exceeds the desktop bounds');
          this.context.drawImage(bitmap, 0, y); this.send(`CLIENT_FRAME_ACK ${id}`);
          if (!this.connected) { this.connected = true; clearTimeout(this.timer); this.dispatchEvent(new Event('connect')); }
        } finally { bitmap.close(); }
      }
    } catch (error) { this.fail(error instanceof Error ? error.message : 'Selkies decoding failed'); }
    finally { this.decoding = false; }
  }
  private message(text: string) {
    if (text.startsWith('clipboard')) { this.clipboardMessage(text); return; }
    if (text === 'MODE websockets' && !this.settingsSent) {
      this.settingsSent = true; this.send('SETTINGS,' + JSON.stringify({ encoder: 'jpeg', displayId: 'primary', framerate: 30, jpeg_quality: 65 })); return;
    }
    if (text.startsWith('KILL')) throw Error('Selkies disconnected this viewer. Another client may have taken control; reconnect when it is available.');
    if (!text.startsWith('{')) return;
    const message = JSON.parse(text) as { type?: string; width?: number; height?: number };
    if (message.type === 'stream_resolution') {
      const { width, height } = message;
      if (!Number.isInteger(width) || !Number.isInteger(height) || width! < 1 || height! < 1 || width! > 8192 || height! > 8192 || width! * height! > 32 * 1024 * 1024) throw Error('Unsupported Selkies desktop resolution');
      if (this.canvas.width !== width || this.canvas.height !== height) { this.canvas.width = width!; this.canvas.height = height!; }
    }
  }
  private clearClipboard() {
    clearTimeout(this.clipboardTimer); this.clipboardTimer = undefined; this.clipboardParts = null;
  }
  private clipboardMessage(text: string) {
    // Clipboard errors never tear down an otherwise usable desktop stream.
    try {
      if (text === 'clipboard_secret') { this.clearClipboard(); this.clipboardSecret = true; this.dispatchEvent(new Event('clipboarderror')); return; }
      if (text.startsWith('clipboard_reply,')) return;
      if (text.startsWith('clipboard_start,')) {
        this.clearClipboard();
        const fields = text.split(','), size = Number(fields[2]);
        if (fields.length !== 3 || fields[1] !== 'text/plain' || !/^\d+$/.test(fields[2]) || !Number.isSafeInteger(size) || size > MAX_CLIPBOARD_BYTES || this.clipboardSecret) throw Error('Unsupported clipboard');
        this.clipboardParts = { bytes: [], size: 0, expected: size };
        this.clipboardTimer = setTimeout(() => { this.clearClipboard(); this.dispatchEvent(new Event('clipboarderror')); }, 10000);
        return;
      }
      if (text.startsWith('clipboard_data,')) {
        if (!this.clipboardParts) return;
        const data = decodeClipboard(text.slice('clipboard_data,'.length)), parts = this.clipboardParts;
        if (!data.length || parts.bytes.length >= 256 || parts.size + data.length > parts.expected) throw Error('Invalid clipboard chunks');
        parts.bytes.push(data); parts.size += data.length; return;
      }
      if (text === 'clipboard_finish') {
        const parts = this.clipboardParts; this.clearClipboard(); this.clipboardSecret = false;
        if (!parts) return;
        if (parts.size !== parts.expected) throw Error('Incomplete clipboard');
        const data = new Uint8Array(parts.size); let offset = 0;
        for (const chunk of parts.bytes) { data.set(chunk, offset); offset += chunk.length; }
        this.emitClipboard(data); return;
      }
      const secret = this.clipboardSecret; this.clipboardSecret = false; this.clearClipboard();
      if (secret) throw Error('Secret clipboard ignored');
      if (text.startsWith('clipboard,')) this.emitClipboard(decodeClipboard(text.slice('clipboard,'.length)));
      else if (text.startsWith('clipboard_binary,text/plain,')) this.emitClipboard(decodeClipboard(text.slice('clipboard_binary,text/plain,'.length)));
      else throw Error('Unsupported clipboard');
    } catch { this.clearClipboard(); this.clipboardSecret = false; this.dispatchEvent(new Event('clipboarderror')); }
  }
  private emitClipboard(data: Uint8Array) {
    const text = new TextDecoder('utf-8', { fatal: true }).decode(data); clipboardBytes(text);
    this.dispatchEvent(new CustomEvent('clipboard', { detail: { text } }));
  }
  private fail(error: string) { if (!this.closed) { this.channel.error = error; this.close(false); } }
  private close(clean: boolean) {
    if (this.closed || this.closing) return; this.closing = true; this.release(); this.flushInput(); this.closed = true; clearTimeout(this.timer); this.events.abort(); this.keyboard.ungrab();
    this.queue = []; this.queuedBytes = 0; this.channel.close(); this.canvas.remove();
    this.clearClipboard(); this.clipboardSecret = false;
    this.dispatchEvent(new CustomEvent('disconnect', { detail: { clean } }));
  }
}
