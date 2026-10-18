import { expect, test } from '@playwright/test';

test('Computer Use is opt in, retains failed input, and requires local confirmation to resume', async ({ page }) => {
  const catalog: any = { node_id: 'target', routes: [], grants: [], computer: null, computer_paused: false };
  const actions: any[] = [];
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'computer_sharing') {
      if (command.computer?.address.startsWith('192.')) return route.fulfill({ status: 400, json: { error: 'Computer Use provider must use a loopback address' } });
      catalog.computer = command.computer;
    }
    if (command.operation === 'computer_owner') {
      if (command.command.action !== 'status') catalog.computer_paused = command.command.action === 'emergency_stop';
      return route.fulfill({ json: { paused: catalog.computer_paused, backend_available: true } });
    }
    if (command.operation === 'grant') return route.fulfill({ json: { node: command.node, token: 'c'.repeat(64), catalog: { ...catalog, grants: [{ node: command.node, permissions: command.permissions }] } } });
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Agent Computer Use', exact: true });
  await expect(panel.getByRole('button', { name: 'Stop Agent input', exact: true })).toHaveCount(0);
  const address = panel.getByLabel('Local bridge address', { exact: true });
  await address.fill('192.0.2.1:8765');
  await panel.getByRole('button', { name: 'Save desktop bridge', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('loopback');
  await expect(address).toHaveValue('192.0.2.1:8765');
  await address.fill('127.0.0.1:8765');
  await panel.getByRole('button', { name: 'Save desktop bridge', exact: true }).click();
  await expect(panel).toContainText('Desktop bridge saved.');
  await panel.getByRole('button', { name: 'Check bridge status', exact: true }).click();
  await expect(panel).toContainText('Desktop bridge ready.');
  await panel.getByRole('button', { name: 'Stop Agent input', exact: true }).click();
  await expect(panel).toContainText('Emergency stop active.');
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Resume on this device', exact: true }).click();
  expect(actions.filter(a => a.operation === 'computer_owner' && a.command.action === 'resume')).toHaveLength(0);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Resume on this device', exact: true }).click();
  await expect(panel).toContainText('Desktop bridge ready.');
  await page.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await expect(page.getByLabel('Allow Agent Computer Use', { exact: true })).not.toBeChecked();
  await page.getByRole('textbox', { name: /^Caller node ID/ }).fill('authorized-agent');
  await page.getByLabel('Allow Agent Computer Use', { exact: true }).check();
  await page.getByRole('button', { name: 'Create access token', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  const grant = actions.find(a => a.operation === 'grant');
  expect(grant.permissions.computer).toBe(true);
  expect(grant.permissions.desktop).toBe(false);
  expect(grant.permissions.exec).toBe(false);
});

test('emergency stop remains available during a slow status check on a small screen', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  let releaseStatus: (() => void) | undefined;
  const wait = new Promise<void>(resolve => { releaseStatus = resolve; });
  const catalog = { node_id: 'local', routes: [], grants: [], computer: { address: '127.0.0.1:8765' }, computer_paused: false };
  const actions: any[] = [];
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'computer_owner') {
      if (command.command.action === 'status') await wait;
      if (command.command.action === 'emergency_stop') catalog.computer_paused = true;
      return route.fulfill({ json: { paused: catalog.computer_paused, backend_available: true } });
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Agent Computer Use', exact: true });
  await panel.getByRole('button', { name: 'Check bridge status', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Check bridge status', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Stop Agent input', exact: true }).click();
  await expect(panel).toContainText('Emergency stop active.');
  releaseStatus!();
  expect(actions.some(a => a.command?.action === 'emergency_stop')).toBe(true);
  for (const theme of ['dark', 'light']) {
    await page.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await panel.screenshot({ path: '/tmp/cranibox-computer-use-mobile-' + theme + '.png' });
  }
  await page.setViewportSize({ width: 812, height: 375 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
