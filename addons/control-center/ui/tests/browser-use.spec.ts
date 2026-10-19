import { expect, test } from '@playwright/test';

test('Browser Use requires a separate grant and local confirmation to resume', async ({ page }) => {
  const catalog: any = { node_id: 'target', routes: [], grants: [], browser: null, browser_paused: false };
  const actions: any[] = [];
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'browser_sharing') {
      if (command.browser?.address.startsWith('192.')) return route.fulfill({ status: 400, json: { error: 'Browser sharing requires a loopback IP and nonzero port' } });
      catalog.browser = command.browser;
    }
    if (command.operation === 'browser_owner') {
      if (command.command.action !== 'status') catalog.browser_paused = command.command.action === 'emergency_stop';
      return route.fulfill({ json: { paused: catalog.browser_paused, backend_available: true, browser: { version: 'TestChromium/1' } } });
    }
    if (command.operation === 'grant') return route.fulfill({ json: { node: command.node, token: 'c'.repeat(64), catalog: { ...catalog, grants: [{ node: command.node, permissions: command.permissions }] } } });
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  await page.locator('summary').filter({ hasText: 'Agent Browser Use' }).click();
  const panel = page.getByRole('region', { name: 'Agent Browser Use', exact: true });
  await expect(panel.getByRole('button', { name: 'Stop browser control', exact: true })).toHaveCount(0);
  const address = panel.getByLabel('Local Chromium CDP address', { exact: true });
  await address.fill('192.0.2.1:9222');
  await panel.getByRole('button', { name: 'Save browser sharing', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('loopback');
  await expect(address).toHaveValue('192.0.2.1:9222');
  await address.fill('127.0.0.1:9222');
  await panel.getByRole('button', { name: 'Save browser sharing', exact: true }).click();
  await expect(panel).toContainText('Browser sharing saved.');
  await panel.getByRole('button', { name: 'Check browser status', exact: true }).click();
  await expect(panel).toContainText('Browser ready: TestChromium/1');
  await panel.getByRole('button', { name: 'Stop browser control', exact: true }).click();
  await expect(panel).toContainText('Emergency stop active.');
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Resume browser sharing on this device', exact: true }).click();
  expect(actions.some(a => a.command?.action === 'resume')).toBe(false);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Resume browser sharing on this device', exact: true }).click();
  await expect(panel).toContainText('Browser ready: TestChromium/1');
  await page.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await expect(page.getByLabel('Allow Agent Browser Use', { exact: true })).not.toBeChecked();
  await page.getByRole('textbox', { name: /^Caller node ID/ }).fill('authorized-agent');
  await page.getByLabel('Allow Agent Browser Use', { exact: true }).check();
  await page.getByRole('button', { name: 'Create access token', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  expect(actions.find(a => a.operation === 'grant').permissions).toEqual({ exec: false, terminals: false, projects: false, desktop: false, computer: false, browser: true, messages: false, activity: false, spaces: false });
});

test('browser emergency stop supersedes a slow status and fits mobile themes', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  let release!: () => void;
  const wait = new Promise<void>(resolve => { release = resolve; });
  const catalog = { node_id: 'target', routes: [], grants: [], browser: { address: '127.0.0.1:9222' }, browser_paused: false };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'browser_owner') {
      if (command.command.action === 'status') { await wait; return route.fulfill({ json: { paused: false, backend_available: true } }); }
      catalog.browser_paused = true;
      return route.fulfill({ json: { paused: true } });
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Agent Browser Use', exact: true });
  await panel.getByRole('button', { name: 'Check browser status', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Check browser status', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Stop browser control', exact: true }).click();
  await expect(panel).toContainText('Emergency stop active.'); release();
  for (const theme of ['light', 'dark']) {
    await page.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
    await expect(panel).toContainText('Emergency stop active.');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await panel.screenshot({ path: '/tmp/cranibox-browser-use-mobile-' + theme + '.png' });
  }
  await page.setViewportSize({ width: 812, height: 375 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('target browser uses the selected identity and never exposes resume remotely', async ({ page }) => {
  const actions: any[] = [];
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'probe') return route.fulfill({ json: { nodes: [{ id: command.node, os: 'linux', arch: 'x86_64', browser: true }] } });
    if (command.operation === 'browser') return route.fulfill({ json: { node: command.node, result: { paused: command.command.operation === 'emergency_stop', backend_available: true } } });
    await route.fulfill({ json: { node_id: 'caller', grants: [], routes: [{ node: 'chosen-target', url: 'http://127.0.0.1:9223', allow_private_http: false }] } });
  });
  await page.goto('/#/nodes');
  await page.getByRole('button', { name: 'Check connection', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Target browser', exact: true });
  await expect(panel).toContainText('cranibox-node peer call --node chosen-target browser-mcp');
  await panel.getByRole('button', { name: 'Check target browser', exact: true }).click();
  await expect(panel).toContainText('Target browser ready.');
  await panel.getByRole('button', { name: 'Stop target browser control', exact: true }).click();
  await expect(panel).toContainText('Stopped. Resume on the target device.');
  expect(actions.filter(a => a.operation === 'browser').map(a => a.node)).toEqual(['chosen-target', 'chosen-target']);
  await expect(panel.getByRole('button', { name: /resume/i })).toHaveCount(0);
});
