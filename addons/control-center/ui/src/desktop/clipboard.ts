// Shared limits for the Web and Tauri viewers; clipboard data is never persisted.
export const MAX_CLIPBOARD_BYTES = 64 * 1024;
export const MAX_CLIPBOARD_BASE64 = 4 * Math.ceil(MAX_CLIPBOARD_BYTES / 3);
export function clipboardBytes(text: string) {
  if (text.length > MAX_CLIPBOARD_BYTES) throw Error('Clipboard text exceeds 64 KiB');
  const bytes = new TextEncoder().encode(text);
  if (bytes.length > MAX_CLIPBOARD_BYTES) throw Error('Clipboard text exceeds 64 KiB');
  return bytes;
}
export function encodeClipboard(text: string) {
  let binary = '';
  for (const byte of clipboardBytes(text)) binary += String.fromCharCode(byte);
  return btoa(binary);
}
export function decodeClipboard(encoded: string) {
  if (encoded.length > MAX_CLIPBOARD_BASE64 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded)) throw Error('Invalid or oversized clipboard text');
  const bytes = Uint8Array.from(atob(encoded), char => char.charCodeAt(0));
  if (bytes.length > MAX_CLIPBOARD_BYTES) throw Error('Clipboard text exceeds 64 KiB');
  return bytes;
}
