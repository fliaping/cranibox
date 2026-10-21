/** Native operations offered only by the desktop app (window.workspaceWeb.shell). */
export type ShellStatus = { autostart: boolean; version: string; platform: string };
export const inShell = () => window.workspaceWeb?.shell === true;
export async function shellCall<T = unknown>(operation: string, body: Record<string, unknown> = {}): Promise<T> {
  const response = await fetch('/api/shell', { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json', 'x-workspace-csrf': window.workspaceWeb.csrf }, body: JSON.stringify({ operation, ...body }) });
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw Error(value.message || value.error || 'Request failed (' + response.status + ')');
  return value as T;
}
