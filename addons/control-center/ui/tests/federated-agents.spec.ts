import { expect, test, type Page } from '@playwright/test';

const session = '44444444-4444-4444-8444-444444444444';
async function directory(page: Page, peers = ['peer-a', 'peer-b']) {
  const now = Math.floor(Date.now() / 1000);
  const report = (name: string, status = 'ready') => ({ id: session, name, harness: 'fixture-cli', status, updated_at: now, expires_at: now + 300, revision: 1 });
  const control = {
    now, peers, reports: { local: [report('Local reviewer')], 'peer-a': [report('Remote reviewer')], 'peer-b': [report('Waiting reviewer', 'waiting')] } as Record<string, any[]>,
    errors: {} as Record<string, string>, owners: {} as Record<string, string>, held: new Set<string>(), releases: [] as (() => void)[], calls: [] as any[], active: 0, maxActive: 0,
  };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); control.calls.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { node_id: 'local', routes: control.peers.map(node => ({ node, url: 'https://' + node + '.example.com', allow_private_http: false })), grants: [] } });
    if (command.operation === 'executions') return route.fulfill({ json: { owner: 'local', executions: [], used: 0, capacity: 128, parallel: 8, log_limit: 1048576 } });
    if (command.operation === 'tasks') return route.fulfill({ json: { owner: 'local', incoming: [], outgoing: [], capacity: 128, used: 0, max_attempts: 8 } });
    if (command.operation === 'agent_sessions') {
      const node = command.node || 'local';
      const response = { owner: control.owners[node] || node, observed_at: control.now, capacity: 64, lease_secs: 300, sessions: structuredClone(control.reports[node] || []) };
      control.active++; control.maxActive = Math.max(control.active, control.maxActive);
      try {
        if (control.held.has(node)) await new Promise<void>(resolve => control.releases.push(resolve));
        if (control.errors[node]) return await route.fulfill({ status: 403, json: { error: control.errors[node] } });
        return await route.fulfill({ json: response });
      } finally { control.active--; }
    }
    return route.fulfill({ status: 400, json: { error: 'Unexpected mutation' } });
  });
  const panel = page.getByRole('region', { name: 'Agents across devices', exact: true });
  const source = (node: string) => panel.getByRole('article', { name: 'Device ' + node, exact: true });
  const release = () => { control.held.clear(); for (const resolve of control.releases.splice(0)) resolve(); };
  return { control, panel, source, release };
}

test('all devices share one Agent view and selection revalidates without replacing a task draft', async ({ page }) => {
  const { control, panel, source, release } = await directory(page, ['peer-a', 'peer-b', 'peer-a', 'local']);
  control.reports['peer-a'].push({ ...control.reports['peer-a'][0], id: '55555555-5555-4555-8555-555555555555', name: '<img src=x onerror="window.unsafe=true">', updated_at: control.now - 400, expires_at: control.now - 100 });
  await page.goto('/#/tasks');
  await expect(panel.locator('.agent-federation-source')).toHaveCount(3);
  await expect(panel.getByRole('status')).toContainText('Current registrations: 3');
  await expect(source('peer-b')).toContainText('Reports waiting');
  await expect(source('peer-a').locator('.agent-session-card')).toHaveCount(1);
  await panel.getByRole('checkbox').check();
  const expired = source('peer-a').locator('.agent-session-card').nth(1);
  await expect(expired).toContainText('<img src=x');
  await expect(expired.getByRole('button')).toBeDisabled();
  await expect(panel.locator('img')).toHaveCount(0);
  await panel.getByRole('searchbox').fill('Waiting');
  await expect(panel.locator('.agent-session-card')).toHaveCount(1);
  await panel.getByRole('searchbox').fill('');
  const title = page.getByRole('textbox', { name: 'Task title', exact: true });
  const description = page.getByRole('textbox', { name: 'Description and completion criteria', exact: true });
  await title.fill('Preserve my draft'); await description.fill('Review the existing files and report evidence.');
  control.held.add('peer-a');
  await source('peer-a').getByRole('button', { name: 'Delegate to this Agent' }).first().click();
  await expect(title).toBeFocused(); await expect(title).toHaveValue('Preserve my draft');
  await expect(description).toHaveValue('Review the existing files and report evidence.');
  await expect(page.getByRole('combobox', { name: 'Target device', exact: true })).toHaveValue('peer-a');
  await expect(page.getByRole('combobox', { name: 'Target Agent', exact: true })).toHaveValue(session);
  const submit = page.getByRole('button', { name: 'Save and delegate', exact: true });
  await expect(submit).toBeDisabled();
  release(); await expect(submit).toBeEnabled();
  // Choosing the same card again still requires a fresh read of its lease.
  control.held.add('peer-a'); await source('peer-a').getByRole('button', { name: 'Delegate to this Agent' }).first().click();
  await expect(submit).toBeDisabled(); release(); await expect(submit).toBeEnabled();
  expect(control.calls.filter(command => command.operation === 'tasks').every(command => command.command.operation === 'list')).toBe(true);
  expect(control.calls.filter(command => command.operation === 'agent_sessions').every(command => command.command.operation === 'list')).toBe(true);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Preserve my draft');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
});

test('failed and mismatched directories clear only their source and removed peers reject late responses', async ({ page }) => {
  const { control, panel, source, release } = await directory(page);
  await page.goto('/#/tasks'); await expect(source('peer-a')).toContainText('Remote reviewer');
  await page.getByRole('button', { name: 'Pause updates', exact: true }).click();
  control.errors['peer-a'] = 'Message permission revoked';
  await panel.getByRole('button', { name: 'Refresh all Agents' }).click();
  await expect(source('peer-a')).toContainText('Message permission revoked');
  await expect(source('peer-a').locator('.agent-session-card')).toHaveCount(0);
  await expect(source('peer-b')).toContainText('Waiting reviewer');
  delete control.errors['peer-a']; control.owners['peer-a'] = 'peer-b';
  await source('peer-a').getByRole('button', { name: 'Retry directory' }).click();
  await expect(source('peer-a')).toContainText('Invalid session catalog or device identity');
  delete control.owners['peer-a']; control.reports['peer-a'].push({ ...control.reports['peer-a'][0] });
  await source('peer-a').getByRole('button', { name: 'Retry directory' }).click();
  await expect(source('peer-a')).toContainText('Invalid session catalog or device identity');
  control.reports['peer-a'].pop();
  await source('peer-a').getByRole('button', { name: 'Retry directory' }).click();
  await expect(source('peer-a')).toContainText('Remote reviewer');
  control.held.add('peer-a'); await panel.getByRole('button', { name: 'Refresh all Agents' }).click();
  await expect.poll(() => control.releases.length).toBe(1);
  control.peers = ['peer-b']; await page.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(source('peer-a')).toHaveCount(0); release();
  await expect.poll(() => control.active).toBe(0);
  await expect(source('peer-a')).toHaveCount(0); await expect(source('peer-b')).toContainText('Waiting reviewer');
});

test('paused reports still expire and manual renewal restores explicitly available targets', async ({ page }) => {
  await page.clock.install();
  const { control, panel, source } = await directory(page);
  await page.goto('/#/tasks'); await expect(source('peer-a')).toContainText('Remote reviewer');
  await page.getByRole('button', { name: 'Pause updates', exact: true }).click();
  const before = control.calls.filter(command => command.operation === 'agent_sessions').length;
  await page.clock.fastForward(301000);
  await expect(panel.getByRole('status')).toContainText('Current registrations: 0');
  expect(control.calls.filter(command => command.operation === 'agent_sessions')).toHaveLength(before);
  await panel.getByRole('checkbox').check();
  await expect(source('peer-a')).toContainText('Report expired');
  await expect(source('peer-a').getByRole('button')).toBeDisabled();
  control.now += 301; control.reports['peer-a'][0] = { ...control.reports['peer-a'][0], updated_at: control.now, expires_at: control.now + 300, revision: 2 };
  await panel.getByRole('button', { name: 'Refresh all Agents' }).click();
  await expect(source('peer-a')).toContainText('Reports ready');
  await expect(source('peer-a').getByRole('button')).toBeEnabled();
});

test('bounded reads let healthy devices refresh while three directories remain slow', async ({ page }) => {
  const { control, panel, source, release } = await directory(page, ['peer-a', 'peer-b', 'peer-c', 'peer-d', 'peer-e', 'peer-f']);
  control.held = new Set(['peer-a', 'peer-b', 'peer-c']);
  control.reports['peer-f'] = [{ ...control.reports.local[0], name: 'Fast reviewer' }];
  await page.goto('/#/tasks');
  await expect(source('peer-f')).toContainText('Fast reviewer');
  await expect.poll(() => control.releases.length).toBe(3);
  expect(control.maxActive).toBeLessThanOrEqual(4);
  control.reports['peer-f'][0].name = 'Updated fast reviewer';
  await panel.getByRole('button', { name: 'Refresh all Agents' }).click();
  await expect(source('peer-f')).toContainText('Updated fast reviewer');
  expect(control.maxActive).toBeLessThanOrEqual(4);
  release(); await expect(source('peer-a')).toContainText('Remote reviewer');
});
