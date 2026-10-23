import { openNetworkSettings } from './network-page';
import { expect, test, type Page } from '@playwright/test';

for (const platform of ['windows', 'macos']) test(platform + ' desktop setup checks the real local greeting before reviewing sharing', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'status') { await route.fulfill({ json: { platform, supported: false, display: '', installed: false, port: 5900 } }); return; }
    await route.continue();
  });
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: null });
  await page.reload();
  const sharing = page.getByRole('region', { name: 'Local desktop sharing', exact: true });
  const guide = sharing.getByRole('region', { name: 'Desktop sharing setup guide', exact: true });
  await expect(guide).toContainText(platform === 'windows' ? 'UltraVNC Server' : 'macOS Screen Sharing');
  await expect(guide).toContainText('does not yet install or manage VNC');
  await expect(guide.getByRole('link').first()).toHaveAttribute('rel', 'noopener noreferrer');
  if (platform === 'macos') await expect(guide).toContainText('does not change the system service');
  else await expect(guide).toContainText('native Windows client');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const viewport of [{ width: 375, height: 1000 }, { width: 812, height: 375 }, { width: 1440, height: 1000 }]) {
    await page.setViewportSize(viewport);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      expect(await guide.getByRole('link').first().evaluate(element => element.getBoundingClientRect().height)).toBeGreaterThanOrEqual(44);
      if ([375, 1440].includes(viewport.width)) {
        await guide.evaluate(element => window.scrollTo(0, scrollY + element.getBoundingClientRect().top - (document.querySelector('.topbar')?.getBoundingClientRect().height || 0) - 16));
        await guide.screenshot({ path: '/tmp/cranibox-' + platform + '-setup-' + viewport.width + '-' + theme + '.png', animations: 'disabled' });
      }
    }
  }
  await guide.getByRole('button', { name: 'Server ready, check port', exact: true }).click();
  const address = guide.getByLabel('VNC address to check', { exact: true });
  await expect(address).toBeFocused();
  const check = guide.getByRole('button', { name: 'Check VNC server', exact: true });
  await address.fill('192.168.1.10:5900'); await check.click();
  await expect(guide.getByRole('alert')).toContainText('127.0.0.1:port');
  expect(commands.filter(command => command.operation === 'check')).toEqual([]);
  await address.fill('127.0.0.1:19111'); await check.click();
  await expect(guide.getByRole('alert')).toBeVisible();
  await expect(address).toHaveValue('127.0.0.1:19111');
  await address.fill('127.0.0.1:19110'); await check.click();
  await expect(guide.getByRole('status')).toContainText('VNC greeting received');
  await expect(guide.getByRole('heading')).toBeFocused();
  await expect(guide).toContainText('does not verify authentication');
  expect((await api(page, { operation: 'list' })).desktop).toBeNull();
  await guide.getByRole('button', { name: 'Review and enable sharing', exact: true }).click();
  const form = sharing.getByRole('form', { name: 'Review desktop sharing', exact: true });
  await expect(form.getByLabel('Local VNC address', { exact: true })).toHaveValue('127.0.0.1:19110');
  await expect(form.getByLabel('Local VNC address', { exact: true })).toBeFocused();
  expect((await api(page, { operation: 'list' })).desktop).toBeNull();
  await form.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  try {
    await expect(guide).toContainText('Sharing is enabled for this address');
    expect(commands.filter(command => command.operation === 'check')).toEqual([{ operation: 'check', address: '127.0.0.1:19111' }, { operation: 'check', address: '127.0.0.1:19110' }, { operation: 'check', address: '127.0.0.1:19110' }]);
    expect(commands.some(command => ['install', 'start', 'credentials'].includes(command.operation))).toBe(false);
    await guide.getByRole('button', { name: 'Open device access', exact: true }).click();
    await expect(page).toHaveURL(/#\/nodes$/);
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('leaving a pending platform setup check cannot open the sharing review', async ({ page }) => {
  let started!: () => void, release!: () => void;
  const ready = new Promise<void>(resolve => { started = resolve; }), delayed = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'status') { await route.fulfill({ json: { platform: 'windows', supported: false, display: '', port: 5900 } }); return; }
    if (command.operation === 'check') { started(); await delayed; await route.fulfill({ json: { ready: true } }); return; }
    await route.continue();
  });
  await page.goto('/#/desktop');
  const guide = page.getByRole('region', { name: 'Desktop sharing setup guide', exact: true });
  await guide.getByRole('button', { name: 'Server ready, check port', exact: true }).click();
  await guide.getByRole('button', { name: 'Check VNC server', exact: true }).click(); await ready;
  await expect(guide.getByRole('button', { name: 'Checking…', exact: true })).toBeDisabled();
  await openNetworkSettings(page); release(); await page.goto('/#/desktop');
  await expect(guide.getByRole('button', { name: 'Server ready, check port', exact: true })).toBeVisible();
  await expect(page.getByRole('form', { name: 'Review desktop sharing', exact: true })).toHaveCount(0);
});

async function api(page: Page, command: Record<string, unknown>) {
  return page.evaluate(async command => {
    const response = await fetch('/api/peers', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify(command) });
    const value = await response.json(); if (!response.ok) throw Error(JSON.stringify(value)); return value;
  }, command);
}
const desktop = (page: Page) => page.frameLocator('iframe.vnc-frame');
async function pixel(page: Page) {
  return desktop(page).locator('#noVNC_container canvas').evaluate((canvas: HTMLCanvasElement) => Array.from(canvas.getContext('2d')!.getImageData(10, 10, 1, 1).data).slice(0, 3));
}
/** Records "Open in browser" links instead of opening tabs. */
async function captureTabs(page: Page) {
  await page.addInitScript(() => {
    (window as any).openedTabs = [];
    window.open = () => ({ opener: window, close() {}, location: { replace(url: string) { (window as any).openedTabs.push(url); } } }) as unknown as Window;
  });
}

test('a Selkies desktop shows its own page, opens in the browser and reconnects', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', e => errors.push(e.message));
  await captureTabs(page);
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  await section.getByRole('button', { name: 'Configure sharing', exact: true }).click();
  await section.getByLabel('Desktop protocol', { exact: true }).selectOption('selkies');
  await expect(section.getByLabel('Local Selkies address', { exact: true })).toHaveValue('127.0.0.1:8082');
  await section.getByLabel('Local Selkies address', { exact: true }).fill('127.0.0.1:19118');
  await section.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(section.getByRole('status').filter({ hasText: 'Local sharing enabled' })).toBeVisible();
  const status = section.locator('.vnc-toolbar [role="status"]');
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(status).toHaveText('Connected');
    await expect(section.locator('.vnc-protocol')).toHaveText('Selkies');
    await expect(section.locator('iframe.vnc-frame')).toHaveAttribute('src', /^http:\/\/127\.0\.0\.1:19121\/cranibox-app-[0-9a-f-]+-page\/\?open=/);
    await expect(desktop(page).locator('#mode')).toHaveText('MODE websockets');
    await expect(desktop(page).locator('#status')).toHaveText('status true');
    await desktop(page).locator('body').click(); await page.keyboard.press('a');
    await expect(desktop(page).locator('#input')).toHaveText('input:kd,a');
    // The framed page runs on its own origin and cannot read this console.
    expect(await desktop(page).locator('body').evaluate(() => { try { return String(window.parent.document.title); } catch { return 'isolated'; } })).toBe('isolated');
    await expect(section.getByRole('button', { name: 'Fullscreen desktop', exact: true })).toBeEnabled();
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await section.screenshot({ path: '/tmp/cranibox-selkies-' + width + '-' + theme + '.png' });
      }
    }
    await section.getByRole('button', { name: 'Open in browser', exact: true }).click();
    await expect(status).toHaveText('Opened in browser');
    await expect(section.locator('iframe.vnc-frame')).toHaveCount(0);
    const tabs: string[] = await page.evaluate(() => (window as any).openedTabs);
    expect(tabs).toHaveLength(1);
    // The link works once, in any browser on this device.
    const tab = await page.context().newPage();
    await tab.goto(tabs[0]);
    await expect(tab.locator('#mode')).toHaveText('MODE websockets');
    await tab.close();
    await section.getByRole('button', { name: 'Show here', exact: true }).click();
    await expect(desktop(page).locator('#mode')).toHaveText('MODE websockets');
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await expect(status).toHaveText('Disconnected');
    await section.getByRole('button', { name: 'Reconnect', exact: true }).click();
    await expect(desktop(page).locator('#mode')).toHaveText('MODE websockets');
    await section.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Connect desktop', exact: true })).toBeDisabled();
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('a VNC desktop opens in noVNC, forwards input and opens in the browser', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await captureTabs(page);
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: { address: '127.0.0.1:19110' } });
  await page.reload();
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  const status = section.locator('.vnc-toolbar [role="status"]');
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(status).toHaveText('Connected');
    await expect(section.locator('.vnc-protocol')).toHaveText('noVNC');
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    await desktop(page).locator('#noVNC_container canvas').click(); await page.keyboard.press('a');
    await expect.poll(() => pixel(page)).toEqual([212, 100, 56]);
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await section.screenshot({ path: '/tmp/cranibox-vnc-' + width + '-' + theme + '.png', animations: 'disabled' });
      }
    }
    // A desktop accepts one controller: the embedded viewer closes before the browser opens.
    await section.getByRole('button', { name: 'Open in browser', exact: true }).click();
    await expect(status).toHaveText('Opened in browser');
    const tabs: string[] = await page.evaluate(() => (window as any).openedTabs);
    const tab = await page.context().newPage();
    await tab.goto(tabs[0]);
    await expect.poll(() => tab.locator('#noVNC_container canvas').evaluate((canvas: HTMLCanvasElement) => Array.from(canvas.getContext('2d')!.getImageData(10, 10, 1, 1).data).slice(0, 3))).toEqual([64, 139, 83]);
    await tab.close();
    // Each connection starts a fresh fixture frame.
    await section.getByRole('button', { name: 'Show here', exact: true }).click();
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    await section.getByRole('button', { name: 'Disconnect', exact: true }).click();
    await expect(status).toHaveText('Disconnected');
    await section.getByRole('button', { name: 'Stop sharing', exact: true }).click();
    await expect(section.getByRole('button', { name: 'Connect desktop', exact: true })).toBeDisabled();
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('noVNC asks for VNC credentials itself and retries after a rejected password', async ({ page }) => {
  await page.goto('/#/desktop');
  await api(page, { operation: 'desktop', desktop: { address: '127.0.0.1:19114' } });
  await page.reload();
  const section = page.getByRole('region', { name: 'Cross-device desktops', exact: true });
  try {
    await section.getByRole('button', { name: 'Connect desktop', exact: true }).click();
    const frame = desktop(page), username = frame.locator('#noVNC_username_input'), password = frame.locator('#noVNC_password_input');
    await expect(password).toBeVisible();
    await username.fill('cranibox'); await password.fill('incorrect-password'); await password.press('Enter');
    await expect(frame.locator('#noVNC_status')).toContainText('Invalid fixture credentials');
    // noVNC offers its own Connect button and asks again; nothing is remembered.
    await frame.locator('#noVNC_connect_button').dispatchEvent('click');
    await expect(password).toBeVisible();
    await expect(password).toHaveValue('');
    await username.fill('cranibox'); await password.fill('wayland8'); await password.press('Enter');
    await expect.poll(() => pixel(page)).toEqual([64, 139, 83]);
    expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain('wayland8');
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

test('existing desktop discovery retries, reviews without sharing and checks again before saving', async ({ page }) => {
  const commands: Record<string, unknown>[] = [], errors: string[] = [];
  let attempt = 0;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation !== 'discover') { await route.continue(); return; }
    if (++attempt === 1) { await route.fulfill({ status: 503, json: { error: 'Local node is reconnecting. Retry shortly.' } }); return; }
    await route.fulfill({ json: { checked: 4, services: attempt === 2 ? [] : [
      { address: '127.0.0.1:19110', protocol: 'vnc', configured: false },
      { address: '127.0.0.1:19118', protocol: 'selkies', configured: false },
      { address: '[::1]:5900', protocol: 'vnc', configured: false },
    ] } });
  });
  await page.goto('/#/desktop');
  const sharing = page.getByRole('region', { name: 'Local desktop sharing', exact: true });
  const discovery = sharing.getByRole('region', { name: 'Existing desktop services', exact: true });
  const detect = discovery.getByRole('button', { name: 'Detect existing services', exact: true });
  const review = discovery.getByRole('button', { name: 'Review Selkies · 127.0.0.1:19118', exact: true });
  const form = sharing.getByRole('form', { name: 'Review desktop sharing', exact: true });
  expect(commands.some(command => command.operation === 'discover')).toBe(false);
  await detect.click(); await expect(discovery.getByRole('alert')).toContainText('reconnecting');
  await detect.click(); await expect(discovery.getByRole('status')).toContainText('No ready services found');
  await expect(discovery.getByRole('alert')).toHaveCount(0);
  await detect.click(); await expect(discovery.getByRole('status')).toContainText('Found 3 services');
  await review.focus(); await page.keyboard.press('Enter');
  const address = form.getByLabel('Local Selkies address', { exact: true });
  await expect(address).toHaveValue('127.0.0.1:19118'); await expect(address).toBeFocused();
  await expect(form.getByLabel('Desktop protocol', { exact: true })).toHaveValue('selkies');
  await expect(sharing.getByRole('button', { name: 'Stop sharing', exact: true })).toHaveCount(0);
  expect(commands.some(command => command.operation === 'check')).toBe(false);
  await form.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(sharing.getByRole('button', { name: 'Configure sharing', exact: true })).toBeFocused();
  await review.click();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const viewport of [{ width: 375, height: 1000 }, { width: 768, height: 1000 }, { width: 1024, height: 1000 }, { width: 1440, height: 1000 }, { width: 812, height: 375 }]) {
    await page.setViewportSize(viewport);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if ([375, 1440].includes(viewport.width)) await sharing.screenshot({ path: '/tmp/cranibox-desktop-discovery-' + viewport.width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await address.fill('127.0.0.1:19111');
  await form.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(page.locator('.remote-desktops > [role="alert"]')).toContainText('unavailable');
  await expect(address).toHaveValue('127.0.0.1:19111');
  await expect(sharing.getByRole('button', { name: 'Stop sharing', exact: true })).toHaveCount(0);
  await address.fill('127.0.0.1:19118');
  await form.getByRole('button', { name: 'Save and enable sharing', exact: true }).click();
  await expect(page.locator('.remote-desktops > [role="status"]')).toContainText('Local sharing enabled');
  try {
    await page.getByRole('region', { name: 'Cross-device desktops', exact: true }).getByRole('button', { name: 'Connect desktop', exact: true }).click();
    await expect(page.locator('.vnc-toolbar [role="status"]')).toHaveText('Connected');
    expect(commands.filter(command => command.operation === 'check')).toEqual([
      { operation: 'check', address: '127.0.0.1:19111', protocol: 'selkies' },
      { operation: 'check', address: '127.0.0.1:19118', protocol: 'selkies' },
    ]);
    expect(errors).toEqual([]);
  } finally { await api(page, { operation: 'desktop', desktop: null }); }
});

test('leaving discovery ignores its pending result and manual configuration stays available', async ({ page }) => {
  let started!: () => void, release!: () => void;
  const ready = new Promise<void>(resolve => { started = resolve; });
  const delayed = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/desktop/service', async route => {
    if (route.request().postDataJSON().operation !== 'discover') { await route.continue(); return; }
    started(); await delayed;
    await route.fulfill({ json: { services: [{ address: '127.0.0.1:5900', protocol: 'vnc', configured: false }], checked: 4 } });
  });
  await page.goto('/#/desktop');
  const discovery = page.getByRole('region', { name: 'Existing desktop services', exact: true });
  await discovery.getByRole('button', { name: 'Detect existing services', exact: true }).click();
  await ready; await expect(discovery.getByRole('button', { name: 'Detecting…', exact: true })).toBeDisabled();
  await page.getByRole('region', { name: 'Local desktop sharing', exact: true }).getByRole('button', { name: 'Configure sharing', exact: true }).click();
  await expect(page.getByLabel('Local VNC address', { exact: true })).toBeVisible();
  await openNetworkSettings(page); release();
  await page.goto('/#/desktop');
  await expect(discovery.getByRole('button', { name: 'Detect existing services', exact: true })).toBeEnabled();
  await expect(discovery.locator('li')).toHaveCount(0);
});

test('managed VNC reviews installation and preserves startup settings for recovery', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  const status = { platform: 'linux', supported: true, installed: false, can_install: true, display: ':42', wayland: false, managed: false, enabled: false, running: false, phase: 'stopped', port: 5900, error: null };
  let rejectStart = true;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    let value: unknown = status;
    if (command.operation === 'plan') value = { id: 'x11-install-plan', expires_in_seconds: 600, commands: 'pkexec /bin/sh -c \'apt-get install x11vnc\'', summary: 'Install x11vnc from this system’s package repositories.' };
    if (command.operation === 'install') { expect(command.plan_id).toBe('x11-install-plan'); status.phase = 'installing'; }
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
    await expect(section).toContainText(target === 'linux' ? 'Wayland session' : target === 'macos' ? 'Enable macOS Screen Sharing' : 'Prepare UltraVNC Server on Windows');
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
  await expect(section).toContainText('choose Selkies in Configure sharing');
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

test('VNC installation explains interrupted work and requires a fresh plan after expiry', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  const status = { platform: 'linux', supported: true, installed: false, can_install: true, display: ':42', wayland: false, managed: false, enabled: false, running: false, phase: 'failed', port: 5900, error: 'The node restarted during VNC installation.', installation: { phase: 'interrupted' } };
  let reviews = 0;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/desktop/service', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'plan') return route.fulfill({ json: { id: 'plan-' + ++reviews, expires_in_seconds: 600, commands: 'pkexec /bin/sh -c apt-get', summary: 'Install x11vnc' } });
    if (command.operation === 'install') {
      if (command.plan_id === 'plan-1') return route.fulfill({ status: 400, json: { error: 'The VNC installation plan changed or expired. Review a new plan.' } });
      expect(command.plan_id).toBe('plan-2');
      Object.assign(status, { phase: 'installing', error: null, installation: { phase: 'running' } });
    }
    await route.fulfill({ json: status });
  });
  await page.goto('/#/desktop');
  const section = page.getByRole('region', { name: 'Local VNC service', exact: true });
  await expect(section.getByRole('alert')).toContainText('Installation will not repeat automatically');
  await page.reload();
  await expect(section.getByRole('alert')).toContainText('Check your package manager');
  expect(commands.every(command => command.operation === 'status')).toBe(true);
  const prepare = section.getByRole('button', { name: 'Prepare x11vnc installation', exact: true });
  await prepare.click();
  await expect(section.getByLabel('Review VNC installation')).toContainText('valid for ten minutes');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await section.screenshot({ path: '/tmp/cranibox-vnc-install-recovery-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await section.getByRole('button', { name: 'Confirm and install', exact: true }).click();
  await expect(section.getByLabel('Review VNC installation')).toHaveCount(0);
  await expect(section.getByRole('alert').filter({ hasText: 'expired' })).toBeVisible();
  await expect(prepare).toBeFocused();
  await prepare.click();
  await section.getByRole('button', { name: 'Confirm and install', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('Exiting the node leaves the system installer running');
  expect(commands.filter(command => command.operation === 'install')).toEqual([{ operation: 'install', plan_id: 'plan-1' }, { operation: 'install', plan_id: 'plan-2' }]);
  expect(commands.some(command => command.operation === 'start')).toBe(false);
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
    if (command.operation === 'plan') value = { id: 'wayland-install-plan', expires_in_seconds: 600, commands: "pkexec /bin/sh -c 'apt-get install wayvnc'", summary: 'Install wayvnc from system repositories.' };
    if (command.operation === 'install') { expect(command.plan_id).toBe('wayland-install-plan'); status.phase = 'installing'; }
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
