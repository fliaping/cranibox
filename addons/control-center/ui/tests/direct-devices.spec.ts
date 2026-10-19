import { expect, test } from '@playwright/test';

test('device paths recover failed input, show the checked address and allow explicit ordering', async ({ page }) => {
  const errors: string[] = [], actions: any[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const device = { node: 'same-device', url: 'http://10.0.0.2:9091', allow_private_http: true, paths: [{ url: 'http://10.0.0.2:9091', allow_private_http: true }] };
  const catalog = { node_id: 'viewer', routes: [device], grants: [] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'add_path') {
      if (command.url.includes('unreachable')) return route.fulfill({ status: 400, json: { error: 'The new path did not respond' } });
      device.paths.push({ url: command.url, allow_private_http: command.allow_private_http });
    }
    if (command.operation === 'prefer_path') {
      const index = device.paths.findIndex(path => path.url === command.url);
      device.paths.unshift(device.paths.splice(index, 1)[0]); device.url = device.paths[0].url;
    }
    if (command.operation === 'remove_path') device.paths = device.paths.filter(path => path.url !== command.url);
    if (command.operation === 'probe') return route.fulfill({ json: { nodes: [{ id: device.node, os: 'linux', arch: 'x86_64', exec: true }], connection: { url: device.paths.at(-1)!.url, fallback: true, path_count: device.paths.length } } });
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Device connection paths', exact: true });
  await panel.getByRole('button', { name: 'Add path', exact: true }).click();
  const form = panel.getByRole('form', { name: 'Add device path', exact: true });
  const address = form.getByLabel('Path address', { exact: true });
  await expect(address).toBeFocused();
  await address.fill('http://unreachable:9091');
  const secret = 'e'.repeat(64);
  await form.getByLabel(/^Existing target access token/).fill(secret);
  await form.getByRole('checkbox').check();
  await form.getByRole('button', { name: 'Check and save path', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('did not respond');
  await expect(address).toHaveValue('http://unreachable:9091');
  await expect(address).toBeFocused();
  await expect(form.getByLabel(/^Existing target access token/)).toHaveValue(secret);
  await address.fill('http://100.64.0.2:9091');
  await form.getByRole('button', { name: 'Check and save path', exact: true }).click();
  await expect(form).toHaveCount(0);
  await expect(panel.locator('li')).toHaveCount(2);
  await expect(panel.getByRole('status')).toContainText('Path saved');
  await page.getByRole('button', { name: 'Check connection', exact: true }).click();
  await expect(panel.locator('li').nth(1)).toContainText('Available at last check');
  await expect(panel).toContainText('The last check used an alternative');
  await panel.getByRole('button', { name: 'Prefer path: http://100.64.0.2:9091', exact: true }).click();
  await expect(panel.locator('li').first()).toContainText('http://100.64.0.2:9091');
  await expect(panel).not.toContainText('Available at last check');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await panel.screenshot({ path: '/tmp/cranibox-paths-' + width + '-' + theme + '.png' });
    }
  }
  await panel.getByRole('button', { name: 'Remove path: http://10.0.0.2:9091', exact: true }).click();
  await expect(panel.locator('li')).toHaveCount(1);
  await expect(panel.getByRole('button', { name: /^Remove path:/ })).toHaveCount(0);
  await panel.getByRole('button', { name: 'Add path', exact: true }).click();
  await expect(form.getByLabel(/^Existing target access token/)).toHaveValue('');
  await form.getByRole('button', { name: 'Close path form', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Add path', exact: true })).toBeFocused();
  await page.getByRole('button', { name: 'Edit connection', exact: true }).click();
  await page.getByRole('button', { name: 'Save connection', exact: true }).click();
  expect(actions.find(action => action.operation === 'connect')).toEqual({ operation: 'connect', node: device.node, url: device.url, token: '', allow_private_http: true });
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage, ...sessionStorage }))).not.toContain(secret);
  expect(actions.filter(action => action.operation === 'add_path')).toHaveLength(2);
  expect(actions.filter(action => action.operation === 'exec')).toHaveLength(0);
  expect(errors).toEqual([]);
});

test('space directory access can be granted without execution, credentials or other capabilities', async ({ page }) => {
  const commands: any[] = [];
  const catalog: any = { node_id: 'space-manager', routes: [], grants: [] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'grant') {
      catalog.grants = [{ node: command.node, permissions: command.permissions }];
      return route.fulfill({ json: { node: command.node, token: 'c'.repeat(64), catalog } });
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Incoming access', exact: true });
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await panel.getByRole('textbox', { name: /^Caller node ID/ }).fill('viewer');
  const submit = panel.getByRole('button', { name: 'Create access token', exact: true });
  await expect(submit).toBeDisabled();
  await expect(panel.locator('input[type=checkbox]:checked')).toHaveCount(0);
  await panel.getByRole('checkbox', { name: 'View virtual spaces managed by this device', exact: true }).check();
  await submit.click();
  await page.getByRole('dialog').getByRole('button', { name: 'Saved, close', exact: true }).click();
  await expect(panel.locator('.direct-grant-list')).toContainText('Space catalog');
  expect(commands.find(command => command.operation === 'grant')).toEqual({ operation: 'grant', node: 'viewer', require_verified: true, permissions: { exec: false, terminals: false, projects: false, desktop: false, computer: false, browser: false, messages: false, spaces: true } });
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await expect(panel.getByRole('checkbox', { name: 'View virtual spaces managed by this device', exact: true })).not.toBeChecked();
  await expect(submit).toBeDisabled();
});

test('incoming listener settings retain failed input, show actual status and confirm stopping', async ({ page }) => {
  const errors: string[] = [], actions: any[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const catalog = { node_id: 'local-id', routes: [], grants: [], listener: null as any, listener_runtime: { active: false, address: null as string | null, applied: true } };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'listener') {
      if (command.listener?.address === '127.0.0.1:9999') {
        await route.fulfill({ status: 400, json: { error: 'Address is already in use' } }); return;
      }
      catalog.listener = command.listener;
      catalog.listener_runtime = { active: !!command.listener, address: command.listener ? '127.0.0.1:19094' : null, applied: true };
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const listener = page.locator('.direct-listener');
  await expect(listener.getByRole('status')).toHaveText('Incoming access is stopped');
  await listener.getByRole('button', { name: 'Configure listener', exact: true }).click();
  const field = listener.getByLabel('Listen address', { exact: true });
  await expect(field).toBeFocused();
  await field.fill('127.0.0.1:9999');
  await listener.getByRole('button', { name: 'Save and enable', exact: true }).click();
  await expect(listener.getByRole('alert')).toContainText('Address is already in use');
  await expect(field).toHaveValue('127.0.0.1:9999');
  await expect(field).toBeFocused();
  await expect(listener.getByRole('status')).toHaveText('Incoming access is stopped');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375 || width === 1440) await listener.screenshot({ path: '/tmp/cranibox-listener-' + width + '-' + theme + '.png' });
    }
  }
  await field.fill('127.0.0.1:0');
  await listener.getByRole('button', { name: 'Save and enable', exact: true }).click();
  await expect(listener.getByRole('status')).toHaveText('Listening on 127.0.0.1:19094');
  await expect(listener.getByRole('button', { name: 'Configure listener', exact: true })).toBeFocused();
  expect(actions.filter(a => a.operation === 'listener').at(-1)).toEqual({ operation: 'listener', listener: { address: '127.0.0.1:0', allow_insecure: false } });
  await listener.getByRole('button', { name: 'Configure listener', exact: true }).click();
  page.once('dialog', dialog => dialog.dismiss());
  await listener.getByRole('button', { name: 'Stop incoming access', exact: true }).click();
  expect(actions.filter(a => a.operation === 'listener')).toHaveLength(2);
  page.once('dialog', dialog => dialog.accept());
  await listener.getByRole('button', { name: 'Stop incoming access', exact: true }).click();
  await expect(listener.getByRole('status')).toHaveText('Incoming access is stopped');
  expect(actions.filter(a => a.operation === 'listener').at(-1)).toEqual({ operation: 'listener', listener: null });
  expect(errors).toEqual([]);
});

test('direct devices retain job queries across reload without resubmitting commands', async ({ page }) => {
  const errors: string[] = [], actions: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const catalog = { node_id: null as string | null, routes: [] as any[] };
  const job = { id: '11111111-1111-4111-8111-111111111111', node: 'direct-laptop', outcome: 'running', stdout: 'output survives refresh', stderr: '', exit_code: null };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    actions.push(command.operation);
    if (command.operation === 'identity') catalog.node_id = 'access-point';
    if (command.operation === 'connect') catalog.routes = [{ node: command.node, url: command.url, allow_private_http: command.allow_private_http }];
    if (command.operation === 'forget') catalog.routes = [];
    if (command.operation === 'cancel') job.outcome = 'cancelled';
    const value = command.operation === 'probe' ? { nodes: [{ id: 'direct-laptop', os: 'macos', arch: 'aarch64', exec: true, terminal_sessions: false }] } : ['exec', 'job', 'cancel'].includes(command.operation) ? job : catalog;
    await route.fulfill({ json: value });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Direct devices', exact: true });
  await panel.getByRole('button', { name: 'Create direct identity', exact: true }).click();
  await expect(panel.locator('.direct-identity')).toContainText('access-point');
  await panel.getByRole('button', { name: 'Add direct device', exact: true }).click();
  await panel.getByLabel('Target node ID', { exact: true }).fill('direct-laptop');
  await panel.getByLabel('Direct URL', { exact: true }).fill('http://127.0.0.1:9140');
  await panel.getByLabel(/^Direct token/).fill('a'.repeat(64));
  await panel.getByRole('button', { name: 'Save connection', exact: true }).click();
  await expect(panel.locator('.direct-editor')).toHaveCount(0);
  await panel.getByRole('button', { name: 'Check connection', exact: true }).click();
  await expect(panel.locator('.direct-probe')).toContainText('Commands granted');
  await panel.getByLabel('Program', { exact: true }).fill('hostname');
  await panel.getByRole('button', { name: 'Run direct task', exact: true }).click();
  await expect(panel.getByLabel('Standard output', { exact: true })).toContainText('output survives refresh');
  await page.reload();
  await expect(panel.getByLabel('Standard output', { exact: true })).toContainText('output survives refresh');
  expect(actions.filter(a => a === 'exec')).toHaveLength(1);
  const saved = await page.evaluate(() => sessionStorage.getItem('cranibox.direct.jobs'));
  expect(JSON.parse(saved!)).toEqual([{ node: job.node, id: job.id }]);
  await panel.getByRole('button', { name: 'Cancel task', exact: true }).click();
  await expect(panel.locator('.direct-jobs [role=status]').first()).toHaveText('cancelled');
  await panel.getByRole('button', { name: 'Edit connection', exact: true }).click();
  await expect(panel.getByLabel(/^Direct token/)).toHaveValue('');
  await panel.getByRole('button', { name: 'Close connection form', exact: true }).click();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await panel.screenshot({ path: '/tmp/cranibox-direct-web-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Remove connection', exact: true }).click();
  await expect(panel.getByRole('heading', { name: 'No direct devices configured', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test('permission and submission failures stay visible and never retry an execution', async ({ page }) => {
  let executions = 0, granted = false;
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'exec') {
      executions++;
      await route.fulfill({ status: 400, json: { error: 'Target unavailable. Check connection before retrying.' } });
    } else await route.fulfill({ json: command.operation === 'probe' ? { nodes: [{ id: 'direct-laptop', os: 'linux', arch: 'x86_64', exec: granted, terminal_sessions: false }] } : { node_id: 'access-point', routes: [{ node: 'direct-laptop', url: 'https://laptop.example.com', allow_private_http: false }] } });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Direct devices', exact: true });
  await panel.getByRole('button', { name: 'Check connection', exact: true }).click();
  await expect(panel.locator('.direct-probe')).toContainText('Commands not granted');
  await expect(panel.getByRole('button', { name: 'Run direct task', exact: true })).toBeDisabled();
  await expect(panel.getByRole('button', { name: 'Open terminal', exact: true })).toBeDisabled();
  granted = true;
  await panel.getByRole('button', { name: 'Check connection', exact: true }).click();
  await panel.getByLabel('Program', { exact: true }).fill('hostname');
  await panel.getByRole('textbox', { name: 'Arguments (JSON array)', exact: true }).fill('{}');
  await panel.getByRole('button', { name: 'Run direct task', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Arguments must be a JSON array of strings');
  expect(executions).toBe(0);
  await panel.getByRole('textbox', { name: 'Arguments (JSON array)', exact: true }).fill('[]');
  await panel.getByRole('button', { name: 'Run direct task', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Target unavailable');
  await panel.getByRole('button', { name: 'Refresh direct devices', exact: true }).click();
  expect(executions).toBe(1);
  await expect(panel.locator('.direct-jobs')).toHaveCount(0);
});

test('direct device opens the shared terminal and routes management to its physical identity', async ({ page }) => {
  const actions: any[] = [], inputs: any[] = [], errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const shell = { id: '11111111-1111-4111-8111-111111111111', name: 'Direct Shell', cwd: '/home/me/project', status: 'running' };
  await page.route('**/api/peers', route => route.fulfill({ json: route.request().postDataJSON().operation === 'probe' ? { nodes: [{ id: 'direct-laptop', terminal_sessions: true, exec: false }] } : { node_id: 'local-id', routes: [{ node: 'direct-laptop', url: 'https://laptop.example.com', allow_private_http: false }], grants: [] } }));
  await page.route('**/api/terminal/sessions', async route => {
    actions.push(route.request().postDataJSON());
    await route.fulfill({ json: { installed: true, sessions: [shell] } });
  });
  await page.route('**/api/terminal/ticket', async route => { actions.push(route.request().postDataJSON()); await route.fulfill({ json: { ticket: 'direct-ticket' } }); });
  await page.routeWebSocket('**/api/terminal/ws?ticket=direct-ticket', socket => {
    socket.send(JSON.stringify({ type: 'ready' }));
    socket.send(new TextEncoder().encode('direct browser Shell'));
    socket.onMessage(data => inputs.push(JSON.parse(String(data))));
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Direct devices', exact: true });
  await panel.getByRole('button', { name: 'Check connection', exact: true }).click();
  await panel.getByRole('button', { name: 'Open terminal', exact: true }).click();
  const floating = page.locator('#floating-terminal');
  await expect(floating.getByRole('tab', { name: 'Direct Shell', exact: true })).toBeVisible();
  await expect(floating.locator('.terminal-pane-head')).toContainText('Connected');
  await floating.getByRole('button', { name: 'Ctrl+C', exact: true }).click();
  await expect.poll(() => inputs.some(input => input.type === 'input' && input.data === '\x03')).toBe(true);
  expect(actions.every(action => action.node === 'direct-laptop' && action.direct === true)).toBe(true);
  expect(actions.some(action => action.operation === 'create' || action.operation === 'end')).toBe(false);
  await floating.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await page.getByRole('button', { name: 'Open terminal window', exact: true }).click();
  await expect(floating.locator('.terminal-pane-head')).toContainText('Connected');
  expect(errors).toEqual([]);
});

test('incoming access issues tokens once, keeps scopes explicit and supports keyboard dismissal', async ({ page }) => {
  const errors: string[] = [], actions: any[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const token = 'b'.repeat(64);
  const catalog = { node_id: 'target-direct-id', routes: [], grants: [] as any[], listener: null, listener_runtime: { active: false, address: null, applied: true } };
  await page.addInitScript(() => Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => { throw Error('Clipboard unavailable'); } } }));
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'grant') {
      catalog.grants = [{ node: command.node, permissions: command.permissions }];
      await route.fulfill({ json: { catalog, node: command.node, token } }); return;
    }
    if (command.operation === 'revoke') catalog.grants = [];
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Incoming access', exact: true });
  await expect(panel).toContainText('Incoming access is stopped');
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await expect(panel.getByLabel(/^Caller node ID/)).toBeFocused();
  const create = panel.getByRole('button', { name: 'Create access token', exact: true });
  await expect(create).toBeDisabled();
  await expect(panel.getByLabel('Execute commands', { exact: true })).not.toBeChecked();
  await expect(panel.getByLabel('Manage and attach terminal sessions', { exact: true })).not.toBeChecked();
  await expect(panel.getByLabel('Read and manage this device’s project catalog', { exact: true })).not.toBeChecked();
  await expect(panel.getByLabel('View and control this device’s desktop', { exact: true })).not.toBeChecked();
  await expect(panel.getByLabel('Device messages and collaboration tasks', { exact: true })).not.toBeChecked();
  await panel.getByLabel(/^Caller node ID/).fill('caller-direct-id');
  await panel.getByLabel('Manage and attach terminal sessions', { exact: true }).check();
  await create.click();
  const dialog = page.getByRole('dialog', { name: 'Save access token', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel('New access token', { exact: true })).toHaveValue(token);
  await expect(dialog).toContainText('target-direct-id');
  await expect(dialog).toContainText('caller-direct-id');
  expect(actions.find(a => a.operation === 'grant')).toEqual({ operation: 'grant', node: 'caller-direct-id', require_verified: true, permissions: { exec: false, terminals: true, projects: false, desktop: false, computer: false, browser: false, messages: false, spaces: false } });
  await dialog.getByRole('button', { name: 'Copy token', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('selected for manual copying');
  expect(await dialog.getByLabel('New access token', { exact: true }).evaluate((element: HTMLTextAreaElement) => element.selectionEnd - element.selectionStart)).toBe(64);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 850 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const bounds = await dialog.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      if (width === 375 || width === 1440) await page.screenshot({ path: '/tmp/cranibox-incoming-' + width + '-' + theme + '.png' });
    }
  }
  await page.keyboard.press('Escape');
  await expect(dialog).not.toBeVisible();
  await expect(panel.getByRole('button', { name: 'Grant device access', exact: true })).toBeFocused();
  expect(await page.locator('textarea').evaluateAll((elements, secret) => elements.some(element => (element as HTMLTextAreaElement).value === secret), token)).toBe(false);
  const stored = await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]));
  expect(stored).not.toContain(token);
  await page.reload();
  await expect(panel.getByLabel('Granted devices', { exact: true })).toContainText('caller-direct-id');
  await expect(page.getByRole('dialog', { name: 'Save access token', exact: true })).not.toBeVisible();
  expect(actions.filter(a => a.operation === 'grant')).toHaveLength(1);
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Revoke access caller-direct-id', exact: true }).click();
  expect(actions.filter(a => a.operation === 'revoke')).toHaveLength(0);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Revoke access caller-direct-id', exact: true }).click();
  await expect(panel.getByLabel('Granted devices', { exact: true })).toContainText('No devices have incoming access');
  expect(errors).toEqual([]);
});
