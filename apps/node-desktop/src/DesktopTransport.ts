import { Channel, invoke } from '@tauri-apps/api/core';
import type { DesktopChannel, DesktopConnector, DesktopTarget } from '../../../addons/control-center/ui/src/desktop/transport';

type Message = { type: 'ready' | 'data' | 'error' | 'closed'; data?: string; sequence?: number; message?: string };
class NativeDesktopChannel implements DesktopChannel {
  readyState = 0;
  binaryType = 'arraybuffer';
  protocol = '';
  error?: string;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  private handler: ((event: MessageEvent) => void) | null = null;
  private pending: Message | null = null;
  private id: Promise<string>;
  private writes: Promise<void> = Promise.resolve();
  private buffered = 0;
  private released = false;
  get onmessage() { return this.handler; }
  set onmessage(handler: ((event: MessageEvent) => void) | null) { this.handler = handler; queueMicrotask(() => this.flush()); }
  constructor(target: DesktopTarget) {
    const events = new Channel<Message>();
    events.onmessage = message => {
      if (this.readyState >= 2) return;
      if (message.type === 'ready') { this.readyState = 1; this.onopen?.(new Event('open')); }
      else if (message.type === 'data') { this.pending = message; this.flush(); }
      else if (message.type === 'error') { this.error = message.message || 'Desktop connection failed'; this.onerror?.(new Event('error')); this.finish(this.error); }
      else this.finish();
    };
    this.id = invoke<string>('desktop_open', { options: target, events });
    void this.id.then(() => { if (this.readyState >= 2) this.release(); }).catch(error => { this.error = String(error); this.onerror?.(new Event('error')); this.finish(this.error); });
  }
  private flush() {
    if (!this.pending || !this.handler || this.readyState !== 1) return;
    const message = this.pending; this.pending = null;
    try {
      const binary = atob(message.data!);
      const bytes = Uint8Array.from(binary, char => char.charCodeAt(0));
      this.handler(new MessageEvent('message', { data: bytes.buffer }));
      void this.id.then(id => invoke('desktop_ack', { id, sequence: message.sequence })).catch(error => { this.finish(String(error)); this.close(); });
    } catch (error) { this.finish(String(error)); this.close(); }
  }
  send(data: ArrayBuffer | ArrayBufferView) {
    if (this.readyState !== 1) throw Error('Desktop disconnected');
    // noVNC reuses its send buffer immediately; copy before the first await.
    const bytes = (data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)).slice();
    if (bytes.length > 65536 || this.buffered + bytes.length > 1048576) { this.close(); throw Error('Desktop input queue is full'); }
    this.buffered += bytes.length;
    this.writes = this.writes.then(async () => {
      if (this.readyState !== 1) return;
      const id = await this.id;
      let binary = ''; for (const byte of bytes) binary += String.fromCharCode(byte);
      await invoke('desktop_input', { id, data: btoa(binary) });
    }).catch(error => { this.finish(String(error)); this.close(); }).finally(() => { this.buffered -= bytes.length; });
  }
  async ready() { await this.id; return this; }
  private finish(reason = '') { if (this.readyState >= 2) return; this.error = reason || this.error; this.readyState = 3; this.pending = null; const failure = this.error || ''; queueMicrotask(() => this.onclose?.(new CloseEvent('close', { code: failure ? 1011 : 1000, reason: failure, wasClean: !failure }))); }
  private release() { if (this.released) return; this.released = true; void this.id.then(id => invoke('desktop_close', { id })).catch(() => {}); }
  close() { this.finish(); this.release(); }
}
export const connectDesktop: DesktopConnector = async (target, signal) => {
  signal?.throwIfAborted();
  const channel = new NativeDesktopChannel(target);
  const cancel = () => channel.close();
  signal?.addEventListener('abort', cancel, { once: true });
  try { await channel.ready(); signal?.throwIfAborted(); return channel; }
  catch (error) { channel.close(); throw error; }
  finally { signal?.removeEventListener('abort', cancel); }
};
