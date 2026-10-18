export type DesktopTarget = { node: string; local: boolean; relay?: boolean };
export type DesktopNode = { id: string; name?: string; online: boolean; desktop_relay?: boolean };
/** noVNC also accepts an RTCDataChannel-like binary transport. */
export interface DesktopChannel {
  readyState: number;
  binaryType: string;
  protocol: string;
  error?: string;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: ArrayBuffer | ArrayBufferView): void;
  close(): void;
}
export type DesktopConnector = (target: DesktopTarget, signal?: AbortSignal) => Promise<string | DesktopChannel>;
