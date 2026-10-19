import { expect, test, type Page } from '@playwright/test';

async function fixture(page: Page) {
  const actions: any[] = [];
  const state: any = { owner: 'local-device', revision: 0, enabled: false, interval_seconds: 60, available_nodes: ['trusted-device', 'other-device'], nodes: [] };
  const control = { fail: false, wrongOwner: false };
  let identity: string | undefined;
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 0, projects: [] } }));
  await page.route('**/api/peers', async route => {
    const request = route.request().postDataJSON();
    if (request.operation !== 'project_sync') return route.fulfill({ json: { node_id: state.owner, routes: [], grants: [] } });
    if (!identity) { const response = await route.fetch({ postData: { operation: 'identity' } }); identity = (await response.json()).node_id; state.owner = identity; }
    const action = request.command; actions.push(action);
    if (action.operation !== 'status' && (control.fail || action.revision !== state.revision)) return route.fulfill({ status: 400, json: { error: 'Settings changed; reload and review selected devices.' } });
    if (action.operation === 'configure') { state.enabled = true; state.revision++; state.interval_seconds = action.interval_seconds; state.nodes = action.nodes.map((node: string) => ({ node, connection_valid: true, phase: 'waiting', last_attempt: null, last_success: null, catalog_revision: null, project_count: null })); }
    if (action.operation === 'pause') { state.enabled = false; state.revision++; state.nodes.forEach((node: any) => { node.phase = 'paused'; }); }
    return route.fulfill({ json: { ...state, owner: control.wrongOwner ? 'wrong-device' : state.owner } });
  });
  return { state, actions, control };
}
async function open(page: Page) {
  await page.goto('/#/projects');
  const panel = page.locator('.project-sync'); await panel.locator(':scope > summary').click();
  await expect(panel).toContainText('Background sync off');
  return panel;
}

test('background catalog sync is lazy, explicitly configured and paused across page reloads', async ({ page }) => {
  const { actions } = await fixture(page); await page.goto('/#/projects');
  await expect(page.locator('.project-sync')).toBeVisible(); expect(actions).toHaveLength(0);
  const panel = await open(page);
  await panel.getByRole('button', { name: 'Set up background sync', exact: true }).click();
  const save = panel.getByRole('button', { name: 'Enable background sync', exact: true }); await expect(save).toBeDisabled();
  await panel.getByRole('checkbox', { name: 'trusted-device', exact: true }).check();
  await panel.getByRole('combobox', { name: 'Sync interval', exact: true }).selectOption('30');
  await save.click(); await expect(panel.getByRole('status')).toContainText('Saved.');
  expect(actions.find(action => action.operation === 'configure')).toEqual({ operation: 'configure', revision: 0, nodes: ['trusted-device'], interval_seconds: 30 });
  await page.reload(); await panel.locator(':scope > summary').click();
  await expect(panel).toContainText('Background sync enabled');
  await panel.getByRole('button', { name: 'Pause sync', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Existing snapshots remain available');
  expect(actions.at(-1)).toEqual({ operation: 'pause', revision: 1 });
  await expect(panel.getByLabel('Device sync status')).toContainText('Paused');
});

test('sync conflicts keep device and interval edits and require a reviewed revision', async ({ page }) => {
  const { state, control, actions } = await fixture(page); const panel = await open(page);
  await panel.getByRole('button', { name: 'Set up background sync', exact: true }).click();
  await panel.getByRole('checkbox', { name: 'trusted-device', exact: true }).check();
  await panel.getByRole('combobox', { name: 'Sync interval', exact: true }).selectOption('300');
  control.fail = true; await panel.getByRole('button', { name: 'Enable background sync', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Settings changed');
  await expect(panel.getByRole('checkbox', { name: 'trusted-device', exact: true })).toBeChecked();
  await expect(panel.getByRole('combobox', { name: 'Sync interval', exact: true })).toHaveValue('300');
  control.fail = false; state.revision++;
  await panel.getByRole('button', { name: 'Refresh sync status', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Enable background sync', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Continue with current version', exact: true }).click();
  await panel.getByRole('button', { name: 'Enable background sync', exact: true }).click();
  expect(actions.at(-1)).toEqual({ operation: 'configure', revision: 1, nodes: ['trusted-device'], interval_seconds: 300 });
});

test('sync controls fit mobile and desktop themes with touch sized checkbox rows', async ({ page }) => {
  const { state } = await fixture(page); const panel = await open(page);
  const external = '<img src=x onerror=window.unsafe=true>' + 'x'.repeat(100); state.available_nodes.push(external);
  await panel.getByRole('button', { name: 'Refresh sync status', exact: true }).click();
  await panel.getByRole('button', { name: 'Set up background sync', exact: true }).click();
  await expect(panel).toContainText(external); await expect(panel.locator('img')).toHaveCount(0);
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const checkbox = panel.getByRole('checkbox', { name: 'trusted-device', exact: true });
      expect((await checkbox.boundingBox())!.width).toBe(20);
      expect((await checkbox.locator('..').boundingBox())!.height).toBeGreaterThanOrEqual(44);
    }
  }
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
});

test('sync policy with a mismatched owner cannot be edited', async ({ page }) => {
  const { control } = await fixture(page); control.wrongOwner = true;
  await page.goto('/#/projects'); const panel = page.locator('.project-sync'); await panel.locator(':scope > summary').click();
  await expect(panel.getByRole('alert')).toContainText('Unexpected catalog sync settings');
  await expect(panel.getByRole('button', { name: 'Set up background sync', exact: true })).toHaveCount(0);
});
