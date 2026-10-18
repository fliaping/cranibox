import { expect, test, type Page } from '@playwright/test';

async function fixture(page: Page) {
  const owner = 'local-device', actions: any[] = [];
  const state: any = { owner, revision: 0, policy: null, senders: [{ node: 'trusted-device', selected: false, binding_active: false }], runs: [], ready: null };
  const control = { available: true, loseSave: false, saves: 0, conflict: false, wrongOwner: false };
  await page.route('**/api/peers', async route => {
    const request = route.request().postDataJSON();
    if (request.operation === 'agent_sessions') { await route.fulfill({ json: { owner: request.node || owner, observed_at: 1792252800, lease_secs: 300, capacity: 64, sessions: [] } }); return; }
    if (request.operation === 'executions') { await route.fulfill({ json: { owner, executions: [], used: 0, capacity: 128, parallel: 8, log_limit: 1048576 } }); return; }
    if (request.operation !== 'tasks') { await route.fulfill({ json: { node_id: owner, routes: [], grants: [] } }); return; }
    if (request.command.operation !== 'automatic') { await route.fulfill({ json: { owner, incoming: [], outgoing: [], used: 0, capacity: 128, max_attempts: 8 } }); return; }
    const action = request.command.command; actions.push(action);
    if (action.operation === 'prepare') {
      if (control.conflict) { await route.fulfill({ status: 400, json: { error: 'Worker policy changed; reload and review it again.' } }); return; }
      await route.fulfill({ json: { owner, revision: action.revision, options: action.options, digest: 'd'.repeat(64), available: control.available, launch: { program: '/tools/codex', args: ['exec', '--sandbox', 'read-only', '-'], cwd: action.options.agent.cwd } } }); return;
    }
    if (action.operation === 'save') {
      if (state.revision === action.revision) { state.policy = action.options; state.revision++; state.senders[0].selected = true; state.senders[0].binding_active = true; state.ready = { directory: true, agent: true }; control.saves++; }
      if (control.loseSave) { control.loseSave = false; await route.fulfill({ status: 503, json: { error: 'Save response interrupted; retry the reviewed policy.' } }); return; }
    }
    if (action.operation === 'pause') { state.policy.enabled = false; state.revision++; }
    await route.fulfill({ json: { ...state, owner: control.wrongOwner ? 'different-device' : owner } });
  });
  return { state, actions, control };
}

async function configure(page: Page) {
  await page.goto('/#/tasks');
  const panel = page.locator('.automatic-worker');
  await panel.locator(':scope > summary').click();
  await panel.getByRole('button', { name: 'Configure automatic worker', exact: true }).click();
  await panel.getByRole('checkbox', { name: 'Enable automatic acceptance after confirmation', exact: true }).check();
  await panel.getByRole('checkbox', { name: 'trusted-device', exact: true }).check();
  await panel.getByRole('textbox', { name: 'Fixed working directory', exact: true }).fill('/home/user/project');
  return panel;
}

test('automatic worker is opt-in, reviews the actual policy and safely retries a lost save response', async ({ page }) => {
  const { state, actions, control } = await fixture(page);
  await page.goto('/#/tasks');
  expect(actions).toHaveLength(0);
  const panel = await configure(page);
  expect(actions.every(action => action.operation === 'get')).toBe(true);
  control.loseSave = true;
  await panel.getByRole('button', { name: 'Review worker policy', exact: true }).click();
  const review = panel.getByRole('region', { name: 'Confirm automatic worker policy', exact: true });
  await expect(review.getByRole('heading')).toBeFocused();
  await expect(review.getByLabel('Reviewed sender devices')).toContainText('trusted-device');
  await expect(review).toContainText('Read only');
  await expect(review).toContainText('/home/user/project');
  expect(state.policy).toBeNull();
  const save = review.getByRole('button', { name: 'Confirm and enable worker', exact: true });
  await expect(save).toBeDisabled();
  await review.getByRole('checkbox').check(); await save.click();
  await expect(panel.getByRole('alert')).toContainText('Save response interrupted');
  await save.click();
  await expect(panel.getByRole('status')).toContainText('Automatic worker policy saved');
  expect(control.saves).toBe(1);
  const saves = actions.filter(action => action.operation === 'save');
  expect(saves).toHaveLength(2); expect(saves[0]).toEqual(saves[1]);
  await page.reload(); await panel.locator(':scope > summary').click();
  await expect(panel).toContainText('Automatic acceptance enabled');
  await panel.getByRole('button', { name: 'Pause new launches', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Existing processes continue');
  expect(actions.at(-1)).toEqual({ operation: 'pause', revision: 1 });
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('/home/user/project');
});

test('unavailable CLI and stale revisions retain the editable worker policy', async ({ page }) => {
  const { state, actions, control } = await fixture(page); const panel = await configure(page);
  control.available = false;
  await panel.getByRole('button', { name: 'Review worker policy', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Agent CLI not found');
  await panel.getByRole('checkbox', { name: 'I reviewed the senders, directory, access and reporting policy', exact: true }).check();
  await expect(panel.getByRole('button', { name: 'Confirm and enable worker', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Back to edit', exact: true }).click();
  await expect(panel.getByRole('textbox', { name: 'Fixed working directory', exact: true })).toHaveValue('/home/user/project');
  control.available = true; control.conflict = true;
  await panel.getByRole('button', { name: 'Review worker policy', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Worker policy changed');
  await expect(panel.getByRole('textbox', { name: 'Fixed working directory', exact: true })).toHaveValue('/home/user/project');
  expect(actions.filter(action => action.operation === 'save')).toHaveLength(0);
  control.conflict = false; state.revision++;
  await panel.getByRole('button', { name: 'Refresh worker status', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Review worker policy', exact: true })).toBeDisabled();
  await expect(panel.getByRole('textbox', { name: 'Fixed working directory', exact: true })).toHaveValue('/home/user/project');
  await panel.getByRole('button', { name: 'Keep edits and review again', exact: true }).click();
  expect(actions.filter(action => action.operation === 'prepare').at(-1)).toMatchObject({ revision: 1, options: { agent: { cwd: '/home/user/project' } } });
  await expect(panel.getByRole('heading', { name: 'Confirm background acceptance', exact: true })).toBeFocused();
  await expect(panel.getByRole('button', { name: 'Confirm and enable worker', exact: true })).toBeDisabled();
});

test('worker review stays usable on narrow screens and displays external text without HTML', async ({ page }) => {
  const { state } = await fixture(page); const panel = await configure(page);
  state.runs = [{ task: 'a'.repeat(100), title: '<img src=x onerror=window.unsafe=true>', phase: 'review_required', state: 'succeeded', error: null }];
  await panel.getByRole('button', { name: 'Refresh worker status', exact: true }).click();
  await expect(panel.getByLabel('Automatic task runs')).toContainText('<img src=x');
  await expect(panel.locator('img')).toHaveCount(0);
  await panel.getByRole('combobox', { name: 'Worker access', exact: true }).selectOption('workspace_write');
  await panel.getByRole('combobox', { name: 'Result reporting', exact: true }).selectOption('agent_proposal');
  await panel.getByRole('button', { name: 'Review worker policy', exact: true }).click();
  await expect(panel.getByRole('region', { name: 'Confirm automatic worker policy' })).toContainText('File changes allowed');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
});

test('worker policy from a different device cannot be edited', async ({ page }) => {
  const { control } = await fixture(page); control.wrongOwner = true;
  await page.goto('/#/tasks'); const panel = page.locator('.automatic-worker'); await panel.locator(':scope > summary').click();
  await expect(panel.getByRole('alert')).toContainText('Unexpected worker policy identity');
  await expect(panel.getByRole('button', { name: 'Configure automatic worker', exact: true })).toBeDisabled();
});
