import { test, expect, type Page } from '@playwright/test';

async function ready(page: Page, path: string) {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.goto('/#' + path);
  await expect(page.locator('main h1').first()).toBeVisible();
}
const capabilities = {
  platform: 'macos',
  items: [
    { id: 'terminal', provider: { kind: 'pty', installed: true, running: true, version: '1.0', detail: null }, permissions: [], sharing: { enabled: true, address: null }, actions: ['disable', 'refresh'], error: null },
    { id: 'computer', provider: { kind: 'cua-driver', installed: false, running: false, version: null, detail: 'CuaDriver.app' }, permissions: [{ id: 'screen_recording', state: 'denied' }, { id: 'accessibility', state: 'unknown' }], sharing: { enabled: false, address: null }, actions: ['install_plan', 'request_permissions', 'open_settings', 'refresh'], error: null },
    { id: 'desktop', provider: { kind: 'macos-screen-sharing', installed: true, running: true, version: null, detail: null }, permissions: [{ id: 'screen_recording', state: 'granted' }], sharing: { enabled: false, address: '127.0.0.1:5900' }, actions: ['enable', 'open_settings', 'refresh'], error: null },
    { id: 'browser', provider: { kind: 'chrome', installed: true, running: false, version: '141', detail: null }, permissions: [{ id: 'screen_recording', state: 'not_required' }], sharing: { enabled: false, address: null }, actions: ['start', 'refresh'], error: 'Profile locked' },
  ],
};

test('one navigation: home shows peers, approvals and no hub wording', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('dialog')).toContainText('Welcome to Cranibox');
  await page.getByRole('button', { name: 'Use on this device first', exact: true }).click();
  await page.getByRole('button', { name: 'Add later, go home', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.reload();
  await expect(page.locator('main h1')).toHaveText('Every device you own, ready for your agents');
  await expect(page.locator('.cx-brand small')).toHaveText('The agent workspace across your devices');
  await expect(page).toHaveTitle(/Cranibox/);
  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  for (const name of ['Home', 'Inbox', 'Agents & models', 'This device']) await expect(nav.getByRole('link', { name: new RegExp('^' + name) })).toBeVisible();
  await expect(page.locator('body')).not.toContainText(/\bHub\b|常驻节点|Advanced/);
  const server = page.getByRole('region', { name: 'Home server', exact: true });
  await expect(server).toContainText('Relayed · 38 ms');
  await expect(server).toContainText('Computer Use');
  await expect(page.getByRole('region', { name: 'Studio', exact: true })).toContainText('Offline');
  await expect(page.locator('.cx-approval-strip')).toContainText('Verify a new device');
  await page.getByRole('link', { name: 'Review' }).click();
  await expect(page).toHaveURL(/#\/app\/inbox$/);
});

test('device detail has overview, desktop, browser and access tabs', async ({ page }) => {
  await ready(page, '/app/home');
  await page.getByRole('region', { name: 'Home server', exact: true }).getByRole('button', { name: 'Desktop', exact: true }).click();
  await expect(page).toHaveURL(/#\/app\/device\/node-0a1b[0-9a-f]+\/desktop$/);
  await expect(page.locator('main h1')).toHaveText('Home server');
  await expect(page.getByRole('button', { name: 'Connect to desktop', exact: true })).toBeEnabled();
  const tabs = page.getByRole('navigation', { name: 'Device sections' });
  await tabs.getByRole('link', { name: 'Browser', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Target browser', exact: true })).toBeVisible();
  await tabs.getByRole('link', { name: 'Overview', exact: true }).click();
  await expect(page.locator('.cx-facts')).toContainText('Relayed (still end-to-end encrypted)');
});

test('per-device grants use the grant contract in both directions', async ({ page }) => {
  let saved: unknown = null;
  await page.route('**/api/v1/app/devices/*/grant', route => {
    if (route.request().method() === 'PUT') { saved = route.request().postDataJSON(); return route.fulfill({ json: saved }); }
    return route.fulfill({ json: { permissions: { exec: true, terminals: true, projects: true, desktop: true, computer: false, browser: true, messages: true, activity: true, spaces: true, llm: true, vault: true } } });
  });
  await ready(page, '/app/device/node-0a1b2c3d4e5f60718293a4b5c6d7e8f9/access');
  const mine = page.getByRole('region', { name: 'What it can do on this device', exact: true });
  await expect(mine.getByRole('switch', { name: 'Computer Use', exact: true })).not.toBeChecked();
  await mine.getByText('Computer Use', { exact: true }).click();
  await expect.poll(() => saved).toMatchObject({ permissions: { computer: true, terminals: true } });
  await expect(page.getByRole('region', { name: 'What I can do on it', exact: true })).toContainText('Allowed');
});

test('guest access falls back to device capabilities when grants are unavailable', async ({ page }) => {
  await ready(page, '/app/device/node-3d4e5f60718293a4b5c6d7e8f90a1b2c/access');
  const mine = page.getByRole('region', { name: 'What it can do on this device', exact: true });
  await expect(mine.getByRole('switch', { name: 'Messages', exact: true })).toBeChecked();
  await expect(mine.getByRole('switch', { name: 'Model access', exact: true })).toHaveCount(0);
  await expect(mine.getByRole('button', { name: 'Revoke access', exact: true })).toBeVisible();
});

test('capabilities page shows providers, permissions, sharing and reviews install plans', async ({ page }) => {
  const calls: string[] = [];
  await page.route('**/api/v1/app/capabilities', route => route.fulfill({ json: capabilities }));
  await page.route('**/api/v1/app/capabilities/*/*', route => {
    const url = new URL(route.request().url()), [, , , , , id, action] = url.pathname.split('/');
    calls.push(id + ':' + action + ':' + JSON.stringify(route.request().postDataJSON()));
    if (action === 'install_plan') return route.fulfill({ json: { plan_id: 'plan-1', commands: ['curl -fsSL https://example.test/install.sh | sh'], url: 'https://example.test/install.sh', expires_at: Math.floor(Date.now() / 1000) + 600 } });
    const item = capabilities.items.find(entry => entry.id === id)!;
    if (action === 'install') return route.fulfill({ json: { ...item, provider: { ...item.provider, installed: true, running: true }, actions: ['refresh'] } });
    if (action === 'enable') return route.fulfill({ json: { ...item, sharing: { ...item.sharing, enabled: true } } });
    return route.fulfill({ json: item });
  });
  await ready(page, '/app/this');
  await expect(page.locator('main h1')).toHaveText('Capabilities & permissions');
  await expect(page.getByText('macOS', { exact: true })).toBeVisible();
  const computer = page.getByRole('region', { name: 'Computer Use', exact: true });
  await expect(computer).toContainText('Not installed');
  await expect(computer).toContainText('Screen recording');
  await expect(computer).toContainText('Not granted');
  await computer.getByRole('button', { name: 'Grant access', exact: true }).click();
  await expect.poll(() => calls.at(-1)).toBe('computer:request_permissions:{}');
  await computer.getByRole('button', { name: 'Install…', exact: true }).click();
  const sheet = page.getByRole('dialog', { name: 'Review the install plan' });
  await expect(sheet).toContainText('curl -fsSL https://example.test/install.sh | sh');
  await sheet.getByRole('button', { name: 'Run install', exact: true }).click();
  await expect.poll(() => calls.at(-1)).toBe('computer:install:{"plan_id":"plan-1"}');
  await expect(computer).toContainText('Running');
  const desktop = page.getByRole('region', { name: 'Remote desktop', exact: true });
  await desktop.getByText('Available to my other devices').click();
  await expect.poll(() => calls.at(-1)).toBe('desktop:enable:{}');
  await expect(desktop).toContainText('Shared');
  await expect(page.getByRole('region', { name: 'Browser', exact: true })).toContainText('Profile locked');
});

test('capabilities page explains when the node does not offer it yet', async ({ page }) => {
  await ready(page, '/app/this');
  await expect(page.getByRole('heading', { name: 'Capability management is not available on this node yet' })).toBeVisible();
});

test('device pairing uses the invitation and SAS confirmation contract', async ({ page }) => {
  await ready(page, '/nodes');
  await page.getByRole('button', { name: 'Add device', exact: true }).click();
  await page.getByRole('button', { name: 'Create QR code and link', exact: true }).click();
  const link = await page.getByLabel('Invitation link', { exact: true }).inputValue();
  expect(link.startsWith('cranibox://pair/')).toBe(true);
  await page.getByLabel('Pairing link', { exact: true }).fill(link);
  await page.getByRole('button', { name: 'Continue to verification', exact: true }).click();
  await expect(page.locator('.cx-sas code')).toHaveText(/[0-9]{4}-[0-9]{4}-[0-9]{4}/);
  const confirmed = page.waitForRequest(request => request.url().includes('/confirm') && request.method() === 'POST');
  await page.getByRole('button', { name: 'Codes match', exact: true }).click();
  expect((await confirmed).postDataJSON()).toEqual({ match: true });
  await expect(page.getByRole('status').filter({ hasText: 'Device paired' })).toBeVisible();
});

test('subscription setup sends regional key and enables one chosen device', async ({ page }) => {
  await ready(page, '/app/models');
  await page.getByRole('button', { name: 'Add subscription', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'API key', exact: true }).click();
  await dialog.getByRole('combobox', { name: 'Service region', exact: true }).selectOption('cn');
  await dialog.getByLabel('Name (optional)', { exact: true }).fill('Personal test account');
  await dialog.getByLabel('API key', { exact: true }).fill('fake-personal-test-key');
  const saved = page.waitForRequest(request => request.url().endsWith('/models/accounts') && request.method() === 'POST');
  await dialog.getByRole('button', { name: 'Save subscription', exact: true }).click();
  expect((await saved).postDataJSON()).toMatchObject({ kind: 'api_key', preset: 'deepseek', region: 'cn', key: 'fake-personal-test-key' });
  await expect(page.getByRole('heading', { name: 'Personal test account', exact: true })).toBeVisible();
  const account = page.locator('.cx-card').filter({ has: page.getByRole('heading', { name: 'DeepSeek', exact: true }) });
  await account.getByText('Choose devices individually', { exact: true }).click();
  const enable = page.waitForRequest(request => request.url().includes('/enable') && request.method() === 'POST');
  await account.getByRole('checkbox', { name: 'Studio', exact: true }).click();
  expect((await enable).postDataJSON()).toMatchObject({ on: true, all_nodes: null });
  await expect(page.locator('body')).not.toContainText('fake-personal-test-key');
});

test('all six agents can select an available model with DSH recommended', async ({ page }) => {
  await ready(page, '/app/models');
  await expect(page.locator('.pa-agent-row')).toHaveCount(6);
  await expect(page.locator('.pa-agent-row').first()).toContainText('Recommended · Default');
  const saved = page.waitForRequest(request => request.url().endsWith('/agents/pi') && request.method() === 'PUT');
  await page.getByRole('combobox', { name: 'Pi model', exact: true }).selectOption('deepseek/deepseek-chat');
  expect((await saved).postDataJSON()).toEqual({ managed: true, model: 'deepseek/deepseek-chat' });
});

test('inbox approval cards remove a request only after a decision', async ({ page }) => {
  await ready(page, '/app/inbox');
  await expect(page.getByRole('navigation', { name: 'Sections' })).toContainText('Tasks');
  const card = page.locator('.cx-approval').filter({ hasText: 'Agent requests a tool' });
  await expect(card).toContainText('rm -rf build/');
  const decision = page.waitForRequest(request => request.url().endsWith('/approvals/apr-2') && request.method() === 'POST');
  await card.getByRole('button', { name: 'Deny', exact: true }).click();
  expect((await decision).postDataJSON()).toEqual({ decision: 'deny' });
  await expect(card).toHaveCount(0);
  await page.getByRole('button', { name: 'Connected', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Current status' })).toBeVisible();
});

test('settings switch theme and language for every page and fit 360px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 780 });
  await ready(page, '/app/settings');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', { name: 'Dark', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByRole('combobox', { name: 'Language' }).selectOption('zh-cn');
  await expect(page.locator('main h1')).toHaveText('设置');
  await page.goto('/#/app/home');
  await expect(page.locator('main h1')).toHaveText('你的每台设备，都是 Agent 的工作台');
  await page.getByRole('button', { name: '打开导航' }).click();
  await expect(page.getByRole('dialog', { name: '导航' }).getByRole('link', { name: '本机', exact: true })).toBeVisible();
  await page.evaluate(() => localStorage.removeItem('cranibox.language'));
});

test('workspace consoles live under This device and old bookmarks keep working', async ({ page }) => {
  await ready(page, '/app/advanced');
  await expect(page).toHaveURL(/#\/overview$/);
  await expect(page.getByRole('navigation', { name: 'Workspace' })).toContainText('Services');
  await page.goto('/#/app/devices');
  await expect(page).toHaveURL(/#\/nodes$/);
  await expect(page.locator('main h1')).toHaveText('Devices & network');
});

test('desktop app settings use the shell API and confirm before quitting', async ({ page }) => {
  await page.addInitScript(() => {
    let value: any;
    Object.defineProperty(window, 'workspaceWeb', { configurable: true, get: () => value, set: next => { value = { ...next, edition: 'node', shell: true }; } });
  });
  const operations: any[] = [];
  await page.route('**/api/shell', route => {
    const body = route.request().postDataJSON(); operations.push(body);
    if (body.operation === 'status' || body.operation === 'autostart') return route.fulfill({ json: { autostart: body.operation === 'autostart' ? body.enabled : false, version: '0.9.0', platform: 'macos' } });
    if (body.operation === 'mcp_config') return route.fulfill({ json: { mcpServers: { cranibox: { command: 'cranibox', args: ['mcp'] } } } });
    return route.fulfill({ json: {} });
  });
  await ready(page, '/app/settings');
  const app = page.getByRole('region', { name: 'Desktop app', exact: true });
  await expect(app).toContainText('Cranibox 0.9.0 · macos');
  await app.getByText('Start at login').click();
  await expect.poll(() => operations.at(-1)).toEqual({ operation: 'autostart', enabled: true });
  await expect(app.getByRole('switch', { name: /Start at login/ })).toBeChecked();
  const mcp = page.getByRole('region', { name: 'Agent access (MCP config)', exact: true });
  await mcp.getByRole('checkbox', { name: 'Include managed run tools' }).check();
  await mcp.getByRole('button', { name: 'Generate configuration' }).click();
  await expect(mcp.getByLabel('MCP configuration')).toContainText('"mcpServers"');
  expect(operations.at(-1)).toEqual({ operation: 'mcp_config', include_runs: true });
  await app.getByRole('button', { name: 'Quit Cranibox' }).click();
  await page.getByRole('dialog', { name: 'Quit Cranibox?' }).getByRole('button', { name: 'Cancel' }).click();
  expect(operations.some(item => item.operation === 'quit')).toBe(false);
  await expect(page.getByRole('link', { name: 'Workspace', exact: true })).toHaveCount(0);
});
