import { expect, test, type Page } from '@playwright/test';

async function api(page: Page, command: Record<string, unknown>) {
  return page.evaluate(async command => {
    const response = await fetch('/api/peers', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify(command) });
    const value = await response.json(); if (!response.ok) throw Error(JSON.stringify(value)); return value;
  }, command);
}
async function pixel(page: Page) {
  return page.locator('.vnc-screen canvas').evaluate((canvas: HTMLCanvasElement) => Array.from(canvas.getContext('2d')!.getImageData(10, 10, 1, 1).data).slice(0, 3));
}
test('desktop text clipboard bounds Unicode, supports chunks and clears on reconnect', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    (window as any).clipboardWrites = [];
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (text: string) => { (window as any).clipboardWrites.push(text); } } });
  });
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: { address: '127.0.0.1:19118', protocol: 'selkies' } });
  await page.reload();
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  const status = section.locator('.vnc-toolbar [role="status"]');
  const push = async (messages: string[]) => { expect((await page.request.post('http://127.0.0.1:19118/clipboard', { data: { messages } })).ok()).toBe(true); };
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(status).toHaveText('Connected');
    await section.getByRole('button', { name: 'Clipboard', exact: true }).click();
    const panel = section.getByRole('region', { name: 'Desktop text clipboard', exact: true });
    const outgoing = panel.getByLabel('Send to remote', { exact: true }), incoming = panel.getByLabel('Remote clipboard', { exact: true });
    await panel.getByRole('button', { name: 'Fetch remote text', exact: true }).click();
    await expect(incoming).toHaveValue('Remote clipboard fixture');
    expect(await page.evaluate(() => (window as any).clipboardWrites)).toEqual([]);
    const unicode = '中文 👋\nremote command';
    await outgoing.fill(unicode); await panel.getByRole('button', { name: 'Send text', exact: true }).click();
    await expect(incoming).toHaveValue(unicode);
    await panel.getByRole('button', { name: 'Copy to this device', exact: true }).click();
    expect(await page.evaluate(() => (window as any).clipboardWrites)).toEqual([unicode]);
    const limit = 'a'.repeat(65536);
    await outgoing.fill(limit); await panel.getByRole('button', { name: 'Send text', exact: true }).click();
    await expect(incoming).toHaveValue(limit);
    await outgoing.fill('中'.repeat(21846));
    await expect(outgoing).toHaveAttribute('aria-invalid', 'true');
    await expect(panel.getByRole('button', { name: 'Send text', exact: true })).toBeDisabled();
    await outgoing.fill('clipboard draft');
    await section.getByRole('button', { name: 'Input enabled', exact: true }).click();
    await expect(panel.getByRole('button', { name: 'Send text', exact: true })).toBeDisabled();
    await section.getByRole('button', { name: 'View only', exact: true }).click();
    const bytes = Buffer.from(unicode);
    // Split inside the first Chinese character: UTF-8 is decoded after assembly.
    await push(['clipboard_start,text/plain,' + bytes.length, 'clipboard_data,' + bytes.subarray(0, 1).toString('base64'), 'clipboard_data,' + bytes.subarray(1).toString('base64'), 'clipboard_finish']);
    await expect(incoming).toHaveValue(unicode);
    for (const message of ['clipboard,%%%%', 'clipboard,/w==', 'clipboard_start,text/plain,65537', 'clipboard_binary,image/png,AAAA']) {
      await push([message]); await expect(panel.getByRole('alert')).toContainText('ignored');
      await expect(incoming).toHaveValue(''); await expect(panel.getByRole('button', { name: 'Copy to this device', exact: true })).toBeDisabled();
      await expect(status).toHaveText('Connected');
    }
    await push(['clipboard_secret', 'clipboard,' + Buffer.from('secret fixture').toString('base64')]);
    await expect(incoming).toHaveValue('');
    await push(['clipboard,']); await expect(panel.getByRole('alert')).toHaveCount(0);
    await expect(panel.getByRole('button', { name: 'Copy to this device', exact: true })).toBeEnabled();
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await panel.screenshot({ path: '/tmp/cranibox-clipboard-' + width + '-' + theme + '.png' });
      }
    }
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await expect(panel).toHaveCount(0);
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click(); await expect(status).toHaveText('Connected');
    await section.getByRole('button', { name: 'Clipboard', exact: true }).click();
    await expect(outgoing).toHaveValue(''); await expect(incoming).toHaveValue('');
    await expect(panel.getByRole('button', { name: 'Copy to this device', exact: true })).toBeDisabled();
    // An insecure origin still allows manual selection when copying is blocked.
    await push(['clipboard,' + Buffer.from(unicode).toString('base64')]); await expect(incoming).toHaveValue(unicode);
    await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw Error('Blocked'); }; document.execCommand = () => false; });
    await panel.getByRole('button', { name: 'Copy to this device', exact: true }).click();
    await expect(panel.getByRole('alert')).toContainText('text is selected');
    expect(await incoming.evaluate((field: HTMLTextAreaElement) => field.selectionEnd - field.selectionStart)).toBe(unicode.length);
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});
test('Selkies shares a JPEG desktop with keyboard, pointer, view only and reconnect', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', e => errors.push(e.message));
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  await section.getByRole('button', { name: 'Configure sharing', exact: true }).click();
  await section.getByLabel('Desktop protocol', { exact: true }).selectOption('selkies');
  await expect(section.getByLabel('Local Selkies address', { exact: true })).toHaveValue('127.0.0.1:8082');
  await section.getByLabel('Local Selkies address', { exact: true }).fill('127.0.0.1:19118');
  await section.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(section.getByRole('status').filter({ hasText: 'Local sharing enabled' })).toBeVisible();
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    const initial = await pixel(page); expect(initial[1]).toBeGreaterThan(initial[0]);
    await section.getByRole('button', { name: 'Keyboard', exact: true }).click();
    await section.getByLabel('Remote keyboard input', { exact: true }).fill('cranibox-selkies-input-20261018');
    await expect.poll(async () => (await pixel(page))[0]).toBeGreaterThan(180);
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    await expect.poll(async () => (await pixel(page))[0]).toBeLessThan(100);
    await section.getByRole('button', { name: 'Input enabled', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Keyboard', exact: true })).toBeDisabled();
    await section.locator('canvas').click();
    expect((await pixel(page))[0]).toBeLessThan(100);
    await section.getByRole('button', { name: 'View only', exact: true }).click();
    await section.locator('canvas').click();
    await expect.poll(async () => (await pixel(page))[0]).toBeGreaterThan(180);
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await section.screenshot({ path: '/tmp/cranibox-selkies-' + width + '-' + theme + '.png' });
      }
    }
    await section.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Connect desktop', exact: true })).toBeDisabled();
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});
test('an available Selkies desktop opens without requiring the editor extension', async ({ page }) => {
  let runtime = true;
  const actions: unknown[] = [];
  await page.route('**/api/events', route => route.fulfill({ status: 503, body: '' }));
  await page.route('**/api/poll', async route => {
    const response = await route.fetch();
    const state = await response.json();
    state.snapshot.desktop = { ...state.snapshot.desktop, installed: false, runtime_available: runtime };
    await route.fulfill({ json: state });
  });
  await page.route('**/api/action', async route => { actions.push(route.request().postDataJSON()); await route.fulfill({ json: {} }); });
  await page.addInitScript(() => {
    (window as any).openedDesktopUrls = [];
    window.open = (url?: string | URL) => { (window as any).openedDesktopUrls.push(String(url)); return null; };
  });
  await page.goto('/#/desktop');
  await page.getByRole('button', { name: 'Open desktop', exact: true }).click();
  expect(await page.evaluate(() => (window as any).openedDesktopUrls)).toEqual([await page.evaluate(() => new URL((window as any).workspaceWeb.desktopUrl).href)]);
  expect(actions).toEqual([]);
  runtime = false;
  await page.reload();
  await expect(page.getByRole('button', { name: 'Install desktop integration', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open desktop', exact: true })).toHaveCount(0);
});
test('leaving a pending desktop releases its unused ticket before reconnecting', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  let issued!: () => void, release!: () => void;
  const ready = new Promise<void>(resolve => { issued = resolve; });
  const delayed = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/desktop/ticket', async route => {
    const response = await route.fetch();
    expect(response.ok()).toBe(true); issued();
    await delayed; await route.fulfill({ response });
  }, { times: 1 });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  await section.getByRole('button').filter({ hasText: 'Preview Linux node' }).click();
  await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
  await ready;
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  const cancelled = page.waitForResponse(response => response.url().endsWith('/api/desktop/cancel'));
  release();
  expect(await (await cancelled).json()).toEqual({ cancelled: true });
  await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
  expect(errors).toEqual([]);
});
test('an outbound-only device opens its real desktop through the Workspace relay', async ({ page }) => {
  const requests: Record<string, unknown>[] = [], errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (request.url().endsWith('/api/desktop/ticket')) requests.push(request.postDataJSON()); });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  await section.getByRole('button').filter({ hasText: 'Preview Linux node' }).click();
  await expect(section.locator('.desktop-route-options')).toContainText('without opening ports');
  await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
  await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
  expect(requests).toEqual([{ node: 'preview-node', local: false, relay: true }]);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const [width, height] of [[375, 900], [900, 375], [1440, 1000]]) {
    await page.setViewportSize({ width, height });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await section.screenshot({ path: '/tmp/cranibox-relay-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
  await page.goto('/#/nodes');
  expect(errors).toEqual([]);
});
test('a real VNC desktop renders, forwards input and reconnects without closing the desktop', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: { address: '127.0.0.1:19110' } });
  await page.reload();
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    await section.getByRole('button', { name: 'Clipboard', exact: true }).click();
    const clipboard = section.getByRole('region', { name: 'Desktop text clipboard', exact: true });
    await clipboard.getByLabel('Send to remote', { exact: true }).fill('VNC clipboard\nLatin-1: café');
    await clipboard.getByRole('button', { name: 'Send text', exact: true }).click();
    await expect(clipboard.getByLabel('Remote clipboard', { exact: true })).toHaveValue('VNC clipboard\nLatin-1: café');
    await expect(clipboard).toContainText('Older VNC servers');
    await clipboard.getByLabel('Send to remote', { exact: true }).fill('v'.repeat(65536));
    await clipboard.getByRole('button', { name: 'Send text', exact: true }).click();
    await expect(clipboard.getByLabel('Remote clipboard', { exact: true })).toHaveValue('v'.repeat(65536));
    await section.getByRole('button', { name: 'Keyboard', exact: true }).click();
    await section.getByLabel('Remote keyboard input', { exact: true }).fill('a');
    await expect.poll(() => pixel(page)).toEqual([212, 100, 56]);
    await section.getByRole('button', { name: 'Input enabled', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Keyboard', exact: true })).toBeDisabled();
    await section.getByRole('button', { name: 'View only', exact: true }).click();
    for (const width of [375, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        if (width === 375 || width === 1440) await section.screenshot({ path: '/tmp/cranibox-vnc-' + width + '-' + theme + '.png', animations: 'disabled' });
      }
    }
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Disconnected');
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    await section.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Connect desktop', exact: true })).toBeDisabled();
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('sharing setup validates addresses and retains input for correction', async ({ page }) => {
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Local desktop sharing', exact: true });
  await section.getByRole('button', { name: 'Configure sharing', exact: true }).click();
  const field = section.getByLabel('Local VNC address', { exact: true });
  await field.fill('192.168.1.23:5900');
  await section.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(page.locator('.remote-desktops > [role="alert"]')).toContainText('loopback');
  await expect(field).toHaveValue('192.168.1.23:5900');
  await field.fill('127.0.0.1:19111');
  await section.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(page.locator('.remote-desktops > [role="alert"]')).toContainText('Cannot connect');
  await expect(field).toHaveValue('127.0.0.1:19111');
  await expect(section.getByRole('button', { name: 'Stop sharing', exact: true })).toHaveCount(0);
  await field.fill('127.0.0.1:19110');
  await section.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(page.locator('.remote-desktops > [role="status"]')).toContainText('Local sharing enabled');
  await section.getByRole('button', { name: 'Stop sharing', exact: true }).click();
});

test('username and password authentication rejects invalid credentials and reconnects privately', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: { address: '127.0.0.1:19114' } });
  await page.reload();
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    const auth = section.locator('.vnc-auth');
    await expect(auth.getByLabel('Username', { exact: true })).toBeFocused();
    await auth.getByLabel('Username', { exact: true }).fill('cranibox');
    await auth.getByLabel('VNC password', { exact: true }).fill('incorrect-password');
    await auth.getByRole('button', { name: 'Authenticate', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Disconnected');
    await expect(section.getByRole('alert')).toContainText('authentication failed');
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
    await expect(auth.getByLabel('Username', { exact: true })).toHaveValue('');
    await expect(auth.getByLabel('VNC password', { exact: true })).toHaveValue('');
    await page.setViewportSize({ width: 375, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await auth.getByLabel('Username', { exact: true }).fill('cranibox');
    await auth.getByLabel('VNC password', { exact: true }).fill('wayland8');
    await auth.getByRole('button', { name: 'Authenticate', exact: true }).click();
    await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain('wayland8');
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
    await expect(auth.getByLabel('VNC password', { exact: true })).toHaveValue('');
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('managed VNC reviews installation and preserves startup settings for recovery', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  const status = { platform: 'linux', supported: true, installed: false, can_install: true, display: ':42', wayland: false, managed: false, enabled: false, running: false, phase: 'stopped', port: 5900, error: null };
  let rejectStart = true;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    let value: unknown = status;
    if (command.operation === 'plan') value = { commands: 'pkexec /bin/sh -c \'apt-get install x11vnc\'', summary: 'Install x11vnc from this system’s package repositories.' };
    if (command.operation === 'install') status.phase = 'installing';
    if (command.operation === 'start') {
      if (rejectStart) { await route.fulfill({ status: 400, json: { error: 'X11 display is unavailable. Check XAUTHORITY and retry.' } }); return; }
      Object.assign(status, { managed: true, enabled: true, running: true, phase: 'running', display: command.display, port: command.port });
    }
    if (command.operation === 'credentials') value = { password: 'abc123XY' };
    if (command.operation === 'stop') Object.assign(status, { enabled: false, running: false, phase: 'stopped' });
    await route.fulfill({ json: value });
  });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Local VNC service', exact: true });
  await section.getByRole('button', { name: 'Prepare x11vnc installation', exact: true }).click();
  await expect(section.getByLabel('Review VNC installation')).toContainText('apt-get install x11vnc');
  await expect(section.getByRole('button', { name: 'Confirm and install', exact: true })).toBeFocused();
  await section.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(section.getByRole('button', { name: 'Prepare x11vnc installation', exact: true })).toBeFocused();
  expect(commands.some(command => command.operation === 'install')).toBe(false);
  await section.getByRole('button', { name: 'Prepare x11vnc installation', exact: true }).click();
  await section.getByRole('button', { name: 'Confirm and install', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('Installing x11vnc');
  Object.assign(status, { installed: true, phase: 'stopped' });
  const display = section.getByLabel('X11 display', { exact: true });
  await expect(display).toHaveValue(':42');
  await display.fill(':7.0');
  await section.getByLabel('Local VNC port', { exact: true }).fill('5911');
  await section.getByRole('button', { name: 'Start and share desktop', exact: true }).click();
  await expect(section.getByRole('alert')).toContainText('XAUTHORITY');
  await expect(display).toHaveValue(':7.0');
  await expect(section.getByLabel('Local VNC port', { exact: true })).toHaveValue('5911');
  rejectStart = false;
  await section.getByRole('button', { name: 'Start and share desktop', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('Running in background');
  expect(commands.some(command => command.operation === 'credentials')).toBe(false);
  await section.getByRole('button', { name: 'Show VNC password', exact: true }).click();
  await expect(section.getByLabel('VNC password', { exact: true })).toHaveValue('abc123XY');
  await section.getByRole('button', { name: 'Hide VNC password', exact: true }).click();
  await expect(section.getByLabel('VNC password', { exact: true })).toHaveCount(0);
  await page.reload();
  await expect(section.getByRole('status')).toContainText('Running in background');
  await expect(section.getByLabel('VNC password', { exact: true })).toHaveCount(0);
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375 || width === 1440) await section.screenshot({ path: '/tmp/cranibox-managed-vnc-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await section.getByRole('button', { name: 'Stop managed service', exact: true }).click();
  await expect(section.getByRole('button', { name: 'Start and share desktop', exact: true })).toBeVisible();
  expect(commands.filter(command => command.operation === 'start')).toEqual([{ operation: 'start', display: ':7.0', port: 5911 }, { operation: 'start', display: ':7.0', port: 5911 }]);
  expect(errors).toEqual([]);
});

test('platform guidance keeps existing VNC setup available on Windows macOS and Wayland', async ({ page }) => {
  let platform = 'windows', wayland = false;
  const commands: any[] = [];
  await page.route('**/api/desktop/service', async route => {
    commands.push(route.request().postDataJSON());
    await route.fulfill({ json: { platform, supported: platform === 'linux', wayland, installed: false, display: '', port: 5900 } });
  });
  for (const target of ['windows', 'macos', 'linux']) {
    platform = target; wayland = target === 'linux';
    await page.goto('/#/desktop');
    await page.reload();
    const section = page.getByRole('region', { name: 'Local VNC service', exact: true });
    await expect(section).toContainText(target === 'linux' ? 'Wayland session' : target === 'macos' ? 'macOS:' : 'Windows:');
    await expect(section.getByRole('button', { name: 'Start and share desktop', exact: true })).toHaveCount(0);
    const sharing = page.getByRole('region', { name: 'Local desktop sharing', exact: true });
    await sharing.getByRole('button', { name: 'Configure sharing', exact: true }).click();
    await expect(sharing.getByLabel('Local VNC address', { exact: true })).toBeVisible();
  }
  expect(commands.every(command => command.operation === 'status')).toBe(true);
});

test('Selkies Wayland guidance avoids VNC installation and keeps existing server setup available', async ({ page }) => {
  const commands: Record<string, unknown>[] = [], errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    commands.push(route.request().postDataJSON());
    await route.fulfill({ json: { platform: 'linux', provider: 'wayvnc', supported: false, wayland: true, selkies_wayland: true, installed: true, display: '', port: 5900 } });
  });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Local VNC service', exact: true });
  await expect(section).toContainText('Choose Selkies below');
  await expect(section.getByRole('button', { name: /installation|Start and share/ })).toHaveCount(0);
  await section.getByRole('button', { name: 'Check VNC environment', exact: true }).click();
  const sharing = page.getByRole('region', { name: 'Local desktop sharing', exact: true });
  await sharing.getByRole('button', { name: 'Configure sharing', exact: true }).click();
  await expect(sharing.getByLabel('Local VNC address', { exact: true })).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await section.screenshot({ path: '/tmp/cranibox-selkies-guidance-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  expect(commands.length).toBeGreaterThanOrEqual(2);
  expect(commands.every(command => command.operation === 'status')).toBe(true);
  expect(errors).toEqual([]);
});

test('Wayland sharing reviews wayvnc and retains display output and private credentials', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  const status = { provider: 'wayvnc', package: 'wayvnc', platform: 'linux', supported: true, installed: false, can_install: true, display: 'wayland-0', output: '', wayland: true, username: 'cranibox', managed: false, enabled: false, running: false, phase: 'stopped', port: 5900 };
  let rejectStart = true;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    let value: unknown = status;
    if (command.operation === 'plan') value = { commands: "pkexec /bin/sh -c 'apt-get install wayvnc'", summary: 'Install wayvnc from system repositories.' };
    if (command.operation === 'install') status.phase = 'installing';
    if (command.operation === 'start') {
      if (rejectStart) { await route.fulfill({ status: 400, json: { error: 'Wayland output is unavailable. Check the wlroots login session.' } }); return; }
      Object.assign(status, { managed: true, enabled: true, running: true, phase: 'running', display: command.display, output: command.output, port: command.port });
    }
    if (command.operation === 'credentials') value = { username: 'cranibox', password: 'wayland8' };
    if (command.operation === 'stop') Object.assign(status, { enabled: false, running: false, phase: 'stopped' });
    await route.fulfill({ json: value });
  });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Local VNC service', exact: true });
  await section.getByRole('button', { name: 'Prepare wayvnc installation', exact: true }).click();
  await expect(section.getByLabel('Review VNC installation')).toContainText('apt-get install wayvnc');
  await section.getByRole('button', { name: 'Confirm and install', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('Installing wayvnc');
  Object.assign(status, { installed: true, phase: 'stopped' });
  const display = section.getByLabel('Wayland display', { exact: true });
  const output = section.getByLabel('Desktop output (optional)', { exact: true });
  await expect(display).toHaveValue('wayland-0');
  await display.fill('wayland-42'); await output.fill('DP-1');
  await section.getByLabel('Local VNC port', { exact: true }).fill('5912');
  await section.getByRole('button', { name: 'Start and share desktop', exact: true }).click();
  await expect(section.getByRole('alert')).toContainText('wlroots login session');
  await expect(display).toHaveValue('wayland-42'); await expect(output).toHaveValue('DP-1');
  rejectStart = false;
  await section.getByRole('button', { name: 'Start and share desktop', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('wayland-42 / DP-1');
  expect(commands.some(command => command.operation === 'credentials')).toBe(false);
  await section.getByRole('button', { name: 'Show VNC password', exact: true }).click();
  await expect(section.getByLabel('VNC username', { exact: true })).toHaveValue('cranibox');
  await expect(section.getByLabel('VNC password', { exact: true })).toHaveValue('wayland8');
  await section.getByRole('button', { name: 'Hide VNC password', exact: true }).click();
  await expect(section.getByLabel('VNC username', { exact: true })).toHaveCount(0);
  await page.reload();
  await expect(section.getByRole('status')).toContainText('Running in background');
  await expect(section.getByLabel('VNC password', { exact: true })).toHaveCount(0);
  await page.setViewportSize({ width: 375, height: 1000 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await section.screenshot({ path: '/tmp/cranibox-wayland-' + theme + '.png', animations: 'disabled' });
  }
  await section.getByRole('button', { name: 'Stop managed service', exact: true }).click();
  await expect(section.getByLabel('Desktop output (optional)', { exact: true })).toHaveValue('DP-1');
  expect(commands.filter(command => command.operation === 'start')).toEqual(Array(2).fill({ operation: 'start', provider: 'wayvnc', display: 'wayland-42', output: 'DP-1', port: 5912 }));
  expect(errors).toEqual([]);
});
