import { test, expect, type Page } from '@playwright/test';
import type { NodeUpdate, UpdateRequest, UpdateStatus } from '../src/types';

const remote = 'node-0a1b2c3d4e5f60718293a4b5c6d7e8f9';
const available = { latest: '1.2.0', next: '1.3.0-rc.1', dev: '1.2.1-dev.7' };
function update(phase: UpdateStatus['phase'] = 'resolving'): UpdateStatus {
  return { id: 'update-1', spec: 'latest', phase, from: '1.1.0', to: '1.2.0', error: null, downloaded: 25, total: 100, started_at: 1792843200, updated_at: 1792843200 };
}
async function mock(page: Page, node = 'self', managed = true) {
  await page.route('**/api/v1/app/access', route => route.fulfill({ json: { peers: [], pending: [], approvals: [], sessions: [] } }));
  const model = {
    data: { node: node === 'self' ? 'local-device' : node, self: node === 'self', version: '1.1.0', runtime: { managed, state: managed ? 'confirmed' : null, version: managed ? '1.1.0' : null, os: 'linux', arch: 'x86_64', can_update: managed, reasons: managed ? [] : ['Unmanaged runtime'] }, status: null } as NodeUpdate,
    commands: [] as UpdateRequest[], queries: [] as string[], statusReads: 0, overallReads: 0,
    offline: false, missing: false, postError: '', availableError: '', available: { ...available },
  };
  await page.route('**/api/v1/app/update**', async route => {
    const request = route.request(), url = new URL(request.url());
    if (request.method() === 'POST') {
      const body = request.postDataJSON() as UpdateRequest;
      model.commands.push(body);
      expect(body.node).toBe(node);
      expect(request.headers()['x-workspace-csrf']).toBeTruthy();
      if (model.postError) return route.fulfill({ status: 409, json: { code: 'conflict', message: model.postError, status: 409 } });
      if (body.action === 'adopt') return route.fulfill({ json: { adopted: '1.1.0', needs_restart: true } });
      model.data.status = update(body.action === 'rollback' ? 'activated' : 'resolving');
      if (body.action === 'rollback') model.data.status.to = '1.0.0';
      else { model.data.status.spec = body.spec; model.data.status.to = available[body.spec as keyof typeof available]; }
      return route.fulfill({ status: body.action === 'apply' ? 202 : 200, json: body.action === 'apply' ? { id: 'update-1' } : model.data.status });
    }
    expect(url.searchParams.get('node')).toBe(node);
    const individual = url.pathname.endsWith('/update-1');
    if (individual) model.statusReads++; else model.overallReads++;
    model.queries.push(url.search);
    if (model.offline) return route.fulfill({ status: 502, json: { code: 'remote_error', message: 'Device is unreachable or refused the request', status: 502 } });
    if (individual) return route.fulfill({ status: model.missing ? 404 : 200, json: model.missing ? { code: 'not_found', message: 'Update not found', status: 404 } : model.data.status });
    return route.fulfill({ json: { ...model.data, ...(url.searchParams.get('check') === '1' ? { available: model.available, available_error: model.availableError || null } : {}) } });
  });
  return model;
}
async function open(page: Page, node = 'self', zh = false) {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.clock.install({ time: new Date('2026-10-24T12:00:00Z') });
  await page.clock.pauseAt(new Date('2026-10-24T12:00:01Z'));
  await page.goto((zh ? 'http://127.0.0.1:19082/' : '/') + '#' + (node === 'self' ? '/app/settings' : '/app/device/' + node));
  const panel = page.getByRole('region', { name: zh ? '更新' : 'Updates', exact: true });
  await expect(panel).toBeVisible();
  return panel;
}

test('self managed: channels, progress, restart failures and confirmed target version', async ({ page }) => {
  const model = await mock(page), panel = await open(page);
  await expect(panel).toContainText('1.1.0');
  await expect(panel).toContainText('linux / x86_64');
  await expect(panel).toContainText('Confirmed');
  expect(model.queries).toEqual(['?node=self']);
  await panel.getByRole('button', { name: 'Check for updates' }).click();
  await expect(panel.getByLabel('Update channel')).toHaveText(/latest · 1.2.0.*next · 1.3.0-rc.1.*dev · 1.2.1-dev.7/);
  expect(model.queries.at(-1)).toBe('?node=self&check=1');
  await panel.getByRole('button', { name: 'Update to 1.2.0', exact: true }).click();
  await expect(panel).toContainText('Resolving version…');
  expect(model.commands).toEqual([{ node: 'self', action: 'apply', spec: 'latest' }]);
  await expect(panel.getByRole('button', { name: 'Check for updates' })).toBeDisabled();
  model.data.status!.phase = 'downloading';
  await page.clock.runFor(2000);
  await expect(panel.getByRole('progressbar', { name: 'Download progress' })).toHaveAttribute('value', '25');
  for (const [phase, text] of [['verifying', 'Verifying…'], ['staged', 'Staged']] as const) {
    model.data.status!.phase = phase;
    await page.clock.runFor(2000);
    await expect(panel).toContainText(text);
  }
  model.data.status!.phase = 'activated';
  await page.clock.runFor(2000);
  await expect(panel).toContainText('Restarting…');
  model.offline = true;
  await page.clock.runFor(2000);
  await expect(panel).toContainText('Restarting…');
  await expect(panel.getByRole('alert')).toHaveCount(0);
  model.offline = false; model.missing = true;
  model.data.version = '1.2.0'; model.data.runtime.version = '1.2.0'; model.data.runtime.state = 'pending';
  await page.clock.runFor(2000);
  await expect(panel).toContainText('Awaiting confirmation');
  await expect(panel.getByText('Updated', { exact: true })).toHaveCount(0);
  model.data.runtime.state = 'confirmed'; model.data.runtime.version = '1.1.0';
  await page.clock.runFor(2000);
  await expect(panel.getByText('Updated', { exact: true })).toHaveCount(0);
  model.data.runtime.version = '1.2.0';
  await page.clock.runFor(2000);
  await expect(panel.getByText('Updated', { exact: true })).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Check for updates' })).toBeEnabled();
  const reads = model.statusReads;
  await page.clock.runFor(6000);
  expect(model.statusReads).toBe(reads);
});

for (const shell of [false, true]) test('self unmanaged: adoption ' + (shell ? 'restarts the native shell' : 'shows the launch command'), async ({ page }) => {
  const model = await mock(page, 'self', false), operations: string[] = [];
  await page.addInitScript(shell => {
    let value: any;
    Object.defineProperty(window, 'workspaceWeb', { configurable: true, get: () => value, set: next => { value = { ...next, edition: 'node', shell }; } });
  }, shell);
  await page.route('**/api/shell', route => {
    const operation = route.request().postDataJSON().operation; operations.push(operation);
    return route.fulfill({ json: operation === 'status' ? { autostart: false, version: '1.1.0', platform: 'linux' } : {} });
  });
  const panel = await open(page);
  await expect(panel).toContainText('Remote updates require Cranibox to manage versions');
  await panel.getByRole('button', { name: 'Enable remote updates' }).click();
  await expect(panel).toContainText('Remote updates are enabled');
  expect(model.commands).toEqual([{ node: 'self', action: 'adopt' }]);
  if (shell) await expect.poll(() => operations.filter(value => value === 'restart')).toEqual(['restart']);
  else { await expect(panel).toContainText('cranibox update launch'); expect(operations).toEqual([]); }
  await expect(panel.getByRole('button', { name: 'Enable remote updates' })).toBeDisabled();
});

test('remote managed: selected channel and target confirmation', async ({ page }) => {
  const model = await mock(page, remote), panel = await open(page, remote);
  await panel.getByRole('button', { name: 'Check for updates' }).click();
  await panel.getByLabel('Update channel').selectOption('next');
  await panel.getByRole('button', { name: 'Update to 1.3.0-rc.1', exact: true }).click();
  await expect(panel).toContainText('Resolving version…');
  expect(model.commands).toEqual([{ node: remote, action: 'apply', spec: 'next' }]);
  model.data.status!.phase = 'done'; model.data.version = model.data.runtime.version = '1.3.0-rc.1';
  await page.clock.runFor(2000);
  await expect(panel.getByText('Updated', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Home server', exact: true })).toBeVisible();
});

test('remote unmanaged: enablement belongs on the target device', async ({ page }) => {
  await mock(page, remote, false);
  const panel = await open(page, remote);
  await expect(panel).toContainText('Enable remote updates on that device itself');
  await expect(panel.getByRole('button', { name: 'Enable remote updates' })).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Roll back' })).toHaveCount(0);
});

for (const phase of ['failed', 'rolled_back'] as const) {
  for (const existing of [true, false]) test(phase + (existing ? ' last status' : ' while polling') + ' displays the error and stops polling', async ({ page }) => {
    const model = await mock(page);
    if (existing) model.data.status = { ...update(phase), error: 'Health probe did not respond' };
    const panel = await open(page);
    if (!existing) {
      await panel.getByRole('button', { name: 'Check for updates' }).click();
      await panel.getByRole('button', { name: 'Update to 1.2.0', exact: true }).click();
      await expect(panel).toContainText('Resolving version…');
      model.data.status = { ...update(phase), error: 'Health probe did not respond' };
      await page.clock.runFor(2000);
    }
    await expect(panel.getByRole('alert')).toContainText('Health probe did not respond');
    if (phase === 'rolled_back') await expect(panel.getByRole('alert')).toContainText('The new version was not healthy and was restored automatically');
    await expect(panel.getByRole('button', { name: 'Check for updates' })).toBeEnabled();
    const reads = model.statusReads;
    await page.clock.runFor(6000);
    expect(model.statusReads).toBe(reads);
  });
}

test('cannot update: reasons disable apply, registry errors are notices', async ({ page }) => {
  const model = await mock(page);
  model.data.runtime.can_update = false; model.data.runtime.reasons = ['Read-only installation', 'No managed launcher'];
  model.availableError = 'Registry unavailable for dev';
  const panel = await open(page);
  await panel.getByRole('button', { name: 'Check for updates' }).click();
  await expect(panel.getByRole('button', { name: 'Update to 1.2.0', exact: true })).toBeDisabled();
  await expect(panel.getByRole('alert')).toContainText('Read-only installation');
  await expect(panel.getByRole('alert')).toContainText('No managed launcher');
  await expect(panel.getByRole('status')).toContainText('Registry unavailable for dev');
  expect(model.commands).toEqual([]);
});

test('unreachable remote: API error stays inside the panel and can be retried', async ({ page }) => {
  const model = await mock(page, remote); model.offline = true;
  const panel = await open(page, remote);
  await expect(panel.getByRole('alert')).toContainText('Device is unreachable or refused the request');
  await expect(page.getByRole('region', { name: 'System', exact: true })).toContainText('Device ID');
  model.offline = false;
  await panel.getByRole('button', { name: 'Retry' }).click();
  await expect(panel).toContainText('1.1.0');
  await expect(panel.getByRole('alert')).toHaveCount(0);
});

test('apply refusal keeps the API message visible', async ({ page }) => {
  const model = await mock(page); model.postError = 'The launcher is not writable';
  const panel = await open(page);
  await panel.getByRole('button', { name: 'Check for updates' }).click();
  await panel.getByRole('button', { name: 'Update to 1.2.0', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('The launcher is not writable');
  expect(model.statusReads).toBe(0);
});

test('rollback requires confirmation and follows the returned update status', async ({ page }) => {
  const model = await mock(page), panel = await open(page);
  await panel.getByRole('button', { name: 'Roll back', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Roll back to the previous version?' });
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect(model.commands).toEqual([]);
  await panel.getByRole('button', { name: 'Roll back', exact: true }).click();
  await dialog.getByRole('button', { name: 'Confirm rollback' }).click();
  await expect(dialog).not.toBeVisible();
  expect(model.commands).toEqual([{ node: 'self', action: 'rollback' }]);
  model.data.version = model.data.runtime.version = '1.0.0';
  await page.clock.runFor(2000);
  await expect(panel.getByText('Updated', { exact: true })).toBeVisible();
});

test('restart times out after five minutes even if a poll never answers', async ({ page }) => {
  const model = await mock(page); model.data.status = update('activated');
  const panel = await open(page);
  await expect(panel).toContainText('Activated');
  await page.route('**/api/v1/app/update/update-1?*', () => {});
  await page.clock.runFor(2000);
  await page.clock.fastForward(298001);
  await expect(panel.getByRole('alert')).toContainText('could not be confirmed within 5 minutes');
  await expect(panel.getByRole('button', { name: 'Check for updates' })).toBeEnabled();
});

test('recovery reads automatic rollback from the overall status after a restart', async ({ page }) => {
  const model = await mock(page); model.data.status = update('activated'); model.missing = true;
  const panel = await open(page);
  await expect(panel).toContainText('Activated');
  model.data.status = { ...update('rolled_back'), error: 'Confirmation deadline expired' };
  await page.clock.runFor(2000);
  await expect(panel.getByRole('alert')).toContainText('Confirmation deadline expired');
  await expect(panel.getByText('Updated', { exact: true })).toHaveCount(0);
});

test('a lost activation response can recover without a per-update record', async ({ page }) => {
  const model = await mock(page), panel = await open(page);
  await panel.getByRole('button', { name: 'Check for updates' }).click();
  await panel.getByRole('button', { name: 'Update to 1.2.0', exact: true }).click();
  await expect(panel).toContainText('Resolving version…');
  model.missing = true; model.data.status = null;
  model.data.version = model.data.runtime.version = '1.2.0';
  await page.clock.runFor(2000);
  await expect(panel.getByText('Updated', { exact: true })).toBeVisible();
});

test('leaving the panel stops its polling', async ({ page }) => {
  const model = await mock(page); model.data.status = update('downloading');
  const panel = await open(page);
  await expect(panel).toContainText('Downloading…');
  await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('link', { name: 'Devices & network' }).click();
  await expect(panel).toHaveCount(0);
  const reads = model.statusReads;
  await page.clock.runFor(6000);
  expect(model.statusReads).toBe(reads);
});

test('Chinese update controls remain usable at mobile width', async ({ page }, testInfo) => {
  await mock(page);
  await page.setViewportSize({ width: 375, height: 850 });
  const panel = await open(page, 'self', true);
  await panel.getByRole('button', { name: '检查更新' }).click();
  await expect(panel.getByRole('button', { name: '更新到 1.2.0', exact: true })).toBeEnabled();
  await panel.getByLabel('更新频道').selectOption('dev');
  await expect(panel.getByRole('button', { name: '更新到 1.2.1-dev.7', exact: true })).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await panel.screenshot({ path: testInfo.outputPath('updates-mobile.png'), animations: 'disabled' });
});
