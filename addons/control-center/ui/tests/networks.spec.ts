import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';

const initial = () => ({ node_id: 'this-node', easytier: { installed: true, enabled: false, version: '2.6.4', phase: 'stopped', network: null, peer: null, listen_port: null, error: null, devices: [] as any[], connections: [] as string[] }, tailscale: { installed: true, state: 'Running', devices: [{ name: 'Personal laptop', addresses: ['100.64.0.2'], online: true, path: 'direct' }] } });

test('Tailscale setup requires a reviewed confirmation, retains errors and continues after reload', async ({ page }) => {
  const status: any = initial(), actions: any[] = [], errors: string[] = [];
  status.tailscale = { installed: false, state: 'NotInstalled', devices: [], setup: { phase: 'idle' } };
  let attempts = 0;
  page.on('pageerror', e => errors.push(e.message));
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'tailscale_setup_plan') return route.fulfill({ json: { available: true, id: 'reviewed-plan', platform: 'linux', action: 'install', requires_admin: true, expires_in_seconds: 600, commands: [{ program: '/usr/bin/pkexec', args: ['/bin/sh', '-c', '<bundled installation script>'] }], script: '#!/bin/sh\n# fixed official source\n', script_sha256: 'a'.repeat(64) } });
    if (command.operation === 'tailscale_setup') {
      if (++attempts === 1) return route.fulfill({ status: 400, json: { error: 'System permission is unavailable. Review administrator access.' } });
      status.tailscale.setup = { phase: 'running', action: 'install' };
    }
    return route.fulfill({ json: status });
  });
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('button', { name: 'Review Tailscale setup', exact: true }).click();
  await expect(ts.getByRole('heading', { name: 'Install system Tailscale', exact: true })).toBeFocused();
  const execute = ts.getByRole('button', { name: 'Confirm and run', exact: true });
  await expect(execute).toBeDisabled();
  await ts.getByText('View commands', { exact: true }).click();
  await expect(ts.locator('pre').first()).toContainText('/usr/bin/pkexec "/bin/sh"');
  expect(actions.some(action => action.operation === 'tailscale_setup')).toBe(false);
  await ts.getByLabel('I reviewed and allow these actions on this node', { exact: true }).check();
  await execute.click(); await expect(page.getByRole('alert')).toContainText('System permission is unavailable');
  await expect(execute).toBeEnabled();
  await execute.click(); await expect(ts).toContainText('Setup is running.');
  await expect(ts.getByRole('button', { name: 'Review Tailscale setup', exact: true })).toHaveCount(0);
  await page.reload(); await expect(ts).toContainText('Setup is running.');
  status.tailscale.state = 'NeedsLogin'; status.tailscale.installed = true; status.tailscale.setup = { phase: 'succeeded', action: 'install' };
  await page.reload(); await expect(ts).toContainText('Setup commands finished.');
  await expect(ts.getByRole('button', { name: 'Sign in to Tailscale', exact: true })).toBeEnabled();
  expect(actions.filter(action => action.operation !== 'status')).toEqual([{ operation: 'tailscale_setup_plan' }, { operation: 'tailscale_setup', plan_id: 'reviewed-plan' }, { operation: 'tailscale_setup', plan_id: 'reviewed-plan' }]);
  expect(errors).toEqual([]);
});

test('Tailscale manual setup explains container boundaries and mobile layouts remain usable', async ({ page }) => {
  const status: any = initial(), actions: any[] = [];
  status.tailscale = { installed: true, state: 'Unavailable', devices: [], setup: { phase: 'interrupted', error: 'The system installer may still be working. Check it before retrying.' } };
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    return route.fulfill({ json: command.operation === 'tailscale_setup_plan' ? { available: false, platform: 'linux', reason: 'Install Tailscale on the container host. Container setup does not change the host network.' } : status });
  });
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await expect(ts.getByRole('alert')).toContainText('Check it before retrying');
  await ts.getByRole('button', { name: 'Review Tailscale setup', exact: true }).click();
  await expect(ts.getByRole('heading', { name: 'Manual setup required', exact: true })).toBeFocused();
  await expect(ts).toContainText('container host');
  await expect(ts.getByRole('link', { name: 'Open official downloads', exact: true })).toHaveAttribute('href', 'https://tailscale.com/download');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
  await ts.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(ts.getByRole('button', { name: 'Review Tailscale setup', exact: true })).toBeFocused();
  expect(actions.some(action => action.operation === 'tailscale_setup')).toBe(false);
});

test('Tailscale setup expires without execution and preserves command details on a small screen', async ({ page }) => {
  await page.clock.install();
  const script = readFileSync(new URL('../../../../crates/workspace-node/src/network/install-tailscale-macos.sh', import.meta.url), 'utf8');
  const status: any = initial(), actions: any[] = [];
  status.tailscale = { installed: false, state: 'NotInstalled', devices: [], setup: { phase: 'idle' } };
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    return route.fulfill({ json: command.operation === 'tailscale_setup_plan' ? { available: true, id: 'expiring-plan', platform: 'macos', action: 'install', requires_admin: true, expires_in_seconds: 600, commands: [{ program: '/bin/sh', args: ['-c', '<bundled installation script>', 'tailscale-setup', '/opt/homebrew/bin/brew'] }], script, script_sha256: 'a'.repeat(64) } : status });
  });
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('button', { name: 'Review Tailscale setup', exact: true }).click();
  await ts.getByText('View commands', { exact: true }).click();
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await ts.screenshot({ path: '/tmp/cranibox-tailscale-setup-' + theme + '.png', animations: 'disabled' });
  }
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.evaluate(() => document.documentElement.style.fontSize = '200%');
  for (const viewport of [{ width: 375, height: 1000 }, { width: 667, height: 375 }]) {
    await page.setViewportSize(viewport);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(ts.getByRole('button', { name: 'Confirm and run', exact: true })).toBeVisible();
  }
  await ts.getByLabel('I reviewed and allow these actions on this node', { exact: true }).check();
  await page.clock.fastForward(601000);
  await expect(ts.getByRole('alert')).toContainText('plan expired');
  await expect(ts.getByRole('button', { name: 'Confirm and run', exact: true })).toBeDisabled();
  expect(actions.some(action => action.operation === 'tailscale_setup')).toBe(false);
});

test('continuous Tailscale discovery retains errors, survives reload and connects only on request', async ({ page }) => {
  const status: any = initial(), actions: any[] = [];
  const identity = { node_id: 'discovered-laptop', name: 'Personal laptop', address: '100.64.0.2', port: 1234, url: 'http://100.64.0.2:1234', paths: [{ address: '100.64.0.2', port: 1234, url: 'http://100.64.0.2:1234' }, { address: '100.64.0.2', port: 4321, url: 'http://100.64.0.2:4321' }] };
  status.tailscale.discovery = { enabled: false, port: null, phase: 'disabled', devices: [], observed_at: null, checked: 0, skipped: 0, ambiguous: 0, error: null };
  let saves = 0, connected = false;
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'tailscale_discovery') {
      if (++saves === 1) return route.fulfill({ status: 400, json: { error: 'Restore networks.json before changing network settings.' } });
      const enabled = command.ports.length > 0;
      Object.assign(status.tailscale.discovery, { enabled, port: command.ports[0] ?? null, ports: command.ports, phase: enabled ? 'ready' : 'disabled', devices: enabled ? [identity] : [], observed_at: enabled ? Math.floor(Date.now() / 1000) : null, checked: enabled ? 3 : 0, skipped: enabled ? 2 : 0, ambiguous: enabled ? 1 : 0 });
    }
    if (command.operation === 'tailscale_connect') { connected = true; return route.fulfill({ json: { network: status, catalog: {} } }); }
    return route.fulfill({ json: status });
  });
  await page.route('**/api/peers', route => route.fulfill({ json: { node_id: 'this-node', routes: connected ? [{ node: identity.node_id, url: identity.url }] : [], grants: [] } }));
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true }), discovery = ts.getByRole('region', { name: 'Continuous Tailscale discovery', exact: true });
  const port = discovery.getByLabel('Node listener ports', { exact: true });
  for (const invalid of ['1234, 1234', '1234, 0', '65536', '1-8', '1,2,3,4,5,6,7,8,9']) {
    await port.fill(invalid); await discovery.getByRole('button', { name: 'Enable continuous discovery', exact: true }).click();
    await expect(port).toHaveAttribute('aria-invalid', 'true'); await expect(port).toBeFocused();
  }
  expect(actions.filter(action => action.operation === 'tailscale_discovery')).toHaveLength(0);
  await port.fill('1234, 4321'); await discovery.getByRole('button', { name: 'Enable continuous discovery', exact: true }).click();
  await expect(discovery.getByRole('alert')).toContainText('Restore networks.json'); await expect(port).toHaveValue('1234, 4321');
  await discovery.getByRole('button', { name: 'Enable continuous discovery', exact: true }).click();
  await expect(discovery).toContainText('Continuous discovery enabled'); await expect(discovery).toContainText('Checked 3');
  await expect(discovery).toContainText('devices not checked'); await expect(discovery.getByRole('alert')).toContainText('Conflicting identities are hidden');
  expect(actions.some(action => action.operation === 'tailscale_connect')).toBe(false);
  await page.reload(); await expect(port).toHaveValue('1234, 4321');
  await expect(discovery.getByRole('list', { name: 'Automatically discovered nodes' })).toContainText(identity.node_id);
  await discovery.getByLabel('Connection address', { exact: true }).selectOption(identity.paths[1].url);
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await ts.screenshot({ path: '/tmp/cranibox-tailscale-discovery-' + theme + '.png', animations: 'disabled' });
  }
  await discovery.getByRole('button', { name: 'Connect node', exact: true }).click();
  const form = ts.getByRole('form', { name: 'Connect Tailscale node', exact: true });
  await expect(form).toContainText(identity.node_id); await expect(form.getByLabel('Target access token', { exact: true })).toBeFocused();
  await form.getByLabel('Target access token', { exact: true }).fill('a'.repeat(64));
  await form.getByRole('button', { name: 'Save device connection', exact: true }).click();
  await expect(form).toHaveCount(0); await expect(page.locator('.direct-list')).toContainText(identity.node_id);
  expect(actions.find(action => action.operation === 'tailscale_connect')).toEqual({ operation: 'tailscale_connect', address: identity.address, port: 4321, node: identity.node_id, token: 'a'.repeat(64) });
  await discovery.getByRole('button', { name: 'Stop discovery', exact: true }).click();
  await expect(discovery).toContainText('Continuous discovery is off');
  await expect(discovery.getByRole('list')).toHaveCount(0);
  expect(actions.filter(action => action.operation === 'tailscale_discovery')).toEqual([{ operation: 'tailscale_discovery', ports: [1234, 4321] }, { operation: 'tailscale_discovery', ports: [1234, 4321] }, { operation: 'tailscale_discovery', ports: [] }]);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('a'.repeat(64));
});

test('network join preserves errors and credentials, then discovers and authorizes a node', async ({ page }) => {
  const status = initial(), actions: any[] = [], errors: string[] = [];
  let joins = 0, connected = false;
  page.on('pageerror', e => errors.push(e.message));
  await page.route('**/api/network', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'easy_tier_join') {
      joins++;
      if (joins === 1) return route.fulfill({ status: 400, json: { error: 'Peer address is not reachable. Check the entry.' } });
      Object.assign(status.easytier, { enabled: true, phase: 'running', network: command.network, peer: command.peers[0], peers: command.peers, devices: [{ node: 'remote-mac', address: '10.0.0.2:43210', path: 'direct', latency_ms: 12.4 }] });
    }
    if (command.operation === 'easy_tier_connect') { connected = true; status.easytier.connections = [command.node]; return route.fulfill({ json: { network: status, catalog: {} } }); }
    if (command.operation === 'easy_tier_stop') { status.easytier.enabled = false; status.easytier.phase = 'stopped'; status.easytier.devices = []; }
    return route.fulfill({ json: status });
  });
  await page.route('**/api/peers', route => route.fulfill({ json: { node_id: 'this-node', routes: connected ? [{ node: 'remote-mac', url: 'http://127.0.0.1:43211' }] : [], grants: [] } }));
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Device networks', exact: true });
  const et = panel.getByRole('article', { name: 'EasyTier', exact: true });
  await et.getByRole('button', { name: 'Enter addresses manually', exact: true }).click();
  await expect(panel).toContainText('Your browser’s device does not join automatically');
  await expect(panel.getByRole('article', { name: 'Tailscale', exact: true })).toContainText('Personal laptop');
  const entry = et.getByLabel('Network entry', { exact: false });
  const token = et.getByLabel('Network token', { exact: false }).first();
  const addresses = ['tcp://entry.example.com:11010', 'wss://backup.example.com/mesh'];
  await entry.fill(addresses.join('\n')); await token.fill('a'.repeat(64));
  await et.getByRole('button', { name: 'Connect EasyTier', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Peer address is not reachable');
  await page.getByRole('button', { name: 'Refresh device networks', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Peer address is not reachable');
  await expect(token).toHaveValue('a'.repeat(64)); await expect(entry).toHaveValue(addresses.join('\n'));
  await et.getByRole('button', { name: 'Connect EasyTier', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('Running in background');
  await expect(et).toContainText('remote-mac');
  expect(actions.filter(a => a.operation === 'easy_tier_join')).toHaveLength(2);
  expect(actions.filter(a => a.operation === 'easy_tier_join')[1].peers).toEqual(addresses);
  await expect(et.locator('.network-saved')).toContainText(addresses[1]);
  await et.getByRole('button', { name: 'Connect', exact: true }).click();
  const access = et.getByLabel('Target access token', { exact: true }); await expect(access).toBeFocused();
  await access.fill('b'.repeat(64)); await et.getByRole('button', { name: 'Save device connection', exact: true }).click();
  await expect(page.locator('.direct-list')).toContainText('remote-mac');
  expect(await page.evaluate(() => JSON.stringify([Object.entries(sessionStorage), Object.entries(localStorage)]))).not.toContain('b'.repeat(64));
  await page.reload(); await expect(et.getByRole('status')).toHaveText('Running in background');
  expect(actions.filter(a => a.operation === 'easy_tier_join')).toHaveLength(2);
  page.once('dialog', d => d.dismiss()); await et.getByRole('button', { name: 'Disconnect EasyTier', exact: true }).click();
  expect(actions.filter(a => a.operation === 'easy_tier_stop')).toHaveLength(0);
  page.once('dialog', d => d.accept()); await et.getByRole('button', { name: 'Disconnect EasyTier', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('Disconnected');
  expect(actions.filter(a => a.operation === 'easy_tier_stop')).toHaveLength(1);
  expect(errors).toEqual([]);
});

test('multiple entries validate before submitting, retain credentials and fit a mobile viewport', async ({ page }) => {
  const commands: any[] = [];
  await page.route('**/api/network', route => { commands.push(route.request().postDataJSON()); return route.fulfill({ json: initial() }); });
  await page.goto('/#/nodes');
  const et = page.getByRole('article', { name: 'EasyTier', exact: true });
  await et.getByRole('button', { name: 'Enter addresses manually', exact: true }).click();
  const entry = et.getByLabel('Network entry addresses', { exact: true });
  const token = et.getByLabel('Network token', { exact: false }).first();
  await token.fill('a'.repeat(64));
  for (const [value, error] of [
    ['tcp://entry.example.com:11010\ntcp://entry.example.com:11010/', 'Remove duplicate'],
    ['tcp://entry.example.com:11010\ntcp://user:secret@backup:11010', 'Entry 2 is invalid'],
    [Array.from({ length: 9 }, (_, n) => 'tcp://entry.example.com:' + (11010 + n)).join('\n'), 'at most 8'],
  ]) {
    await entry.fill(value);
    await et.getByRole('button', { name: 'Connect EasyTier', exact: true }).click();
    await expect(entry).toBeFocused(); await expect(entry).toHaveAttribute('aria-invalid', 'true');
    await expect(et.getByRole('alert')).toContainText(error);
    await expect(token).toHaveValue('a'.repeat(64)); await expect(entry).toHaveValue(value);
  }
  expect(commands.filter(c => c.operation === 'easy_tier_join')).toEqual([]);
  await entry.fill('tcp://entry.example.com:11010\nwss://backup.example.com/mesh');
  await expect(et.getByRole('alert')).toHaveCount(0);
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await et.screenshot({ path: '/tmp/cranibox-network-multiple-' + theme + '.png', animations: 'disabled' });
  }
});

test('installation is reviewable and survives leaving the view; mobile forms stay within the viewport', async ({ page }) => {
  const status = initial(); status.easytier.installed = false; status.tailscale = { installed: false, state: 'NotInstalled', devices: [] };
  let installs = 0;
  await page.route('**/api/network', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'plan') return route.fulfill({ json: { version: '2.6.4', platform: 'linux-x86_64', directory: '/config/.config/cranibox-node/tools/easytier-2.6.4', sha256: 'f'.repeat(64) } });
    if (command.operation === 'install') { installs++; status.easytier.phase = 'installing'; }
    return route.fulfill({ json: status });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Device networks', exact: true });
  await panel.getByRole('button', { name: 'Review installation', exact: true }).click();
  await expect(panel).toContainText('SHA-256'); expect(installs).toBe(0);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await panel.screenshot({ path: '/tmp/cranibox-network-install-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await panel.getByRole('button', { name: 'Download and install', exact: true }).click();
  await expect(panel.getByRole('article', { name: 'EasyTier', exact: true }).getByRole('status')).toHaveText('Installing…');
  await page.reload(); await expect(panel).toContainText('Installing…'); expect(installs).toBe(1);
});

test('Tailscale uses the existing service and only exposes an official login URL', async ({ page }) => {
  const status: any = initial(); status.tailscale = { installed: true, state: 'NeedsLogin', login_pending: false, devices: [] };
  const actions: string[] = [];
  await page.route('**/api/network', async route => {
    const command = route.request().postDataJSON(); actions.push(command.operation);
    if (command.operation === 'tailscale_login') { status.tailscale.login_pending = true; status.tailscale.login_url = 'https://evil.example/forged-login'; }
    if (command.operation === 'tailscale_cancel_login') status.tailscale.login_pending = false;
    return route.fulfill({ json: status });
  });
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('button', { name: 'Sign in to Tailscale', exact: true }).click();
  await expect(ts.getByRole('link', { name: 'Open Tailscale sign in', exact: true })).toHaveCount(0);
  status.tailscale.login_url = 'https://login.tailscale.com/a/fixture';
  await page.getByRole('button', { name: 'Refresh device networks', exact: true }).click();
  await expect(ts.getByRole('link', { name: 'Open Tailscale sign in', exact: true })).toHaveAttribute('href', status.tailscale.login_url);
  await page.reload(); await expect(ts.getByRole('link', { name: 'Open Tailscale sign in', exact: true })).toBeVisible();
  expect(actions.filter(a => a === 'tailscale_login')).toHaveLength(1);
  await ts.getByRole('button', { name: 'Cancel sign in', exact: true }).click();
  await expect(ts.getByRole('button', { name: 'Sign in to Tailscale', exact: true })).toBeVisible();
});

test('Tailscale checks one selected identity before saving access and keeps failed input', async ({ page }) => {
  const status = initial(), actions: any[] = [];
  status.tailscale.devices.push({ name: 'Offline device', addresses: ['100.64.0.3'], online: false, path: 'relay' });
  let lookups = 0, saves = 0, connected = false;
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'tailscale_discover') {
      if (++lookups === 1) return route.fulfill({ status: 400, json: { error: 'Node not reachable. Check the listener port.' } });
      return route.fulfill({ json: { node_id: 'tailscale-laptop', address: command.address, port: command.port, url: 'http://100.64.0.2:' + command.port } });
    }
    if (command.operation === 'tailscale_connect') {
      if (++saves === 1) return route.fulfill({ status: 400, json: { error: 'The target node identity changed. Find the node again before saving.' } });
      connected = true; return route.fulfill({ json: { network: status, catalog: {} } });
    }
    return route.fulfill({ json: status });
  });
  await page.route('**/api/peers', route => route.fulfill({ json: { node_id: 'this-node', grants: [], routes: connected ? [{ node: 'tailscale-laptop', url: 'http://100.64.0.2:1234' }] : [] } }));
  await page.goto('/#/nodes');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await expect(ts.getByRole('button', { name: 'Find node', exact: true }).nth(1)).toBeDisabled();
  expect(actions.filter(a => a.operation === 'tailscale_discover')).toHaveLength(0);
  await ts.getByRole('button', { name: 'Find node', exact: true }).first().click();
  const form = ts.getByRole('form', { name: 'Connect Tailscale node', exact: true });
  const port = form.getByLabel('Target node listener port', { exact: false });
  await expect(port).toBeFocused(); await port.fill('1234');
  await form.getByRole('button', { name: 'Check node identity', exact: true }).click();
  await expect(form.getByRole('alert')).toContainText('not reachable'); await expect(port).toHaveValue('1234');
  await form.getByRole('button', { name: 'Check node identity', exact: true }).click();
  const token = form.getByLabel('Target access token', { exact: true }); await expect(token).toBeFocused();
  await expect(form).toContainText('tailscale-laptop'); await expect(form).toContainText('this-node');
  await token.fill('b'.repeat(64));
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await ts.screenshot({ path: '/tmp/cranibox-tailscale-connect-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  await form.getByRole('button', { name: 'Save device connection', exact: true }).click();
  await expect(form.getByRole('alert')).toContainText('identity changed'); await expect(token).toHaveValue('b'.repeat(64));
  await form.getByRole('button', { name: 'Find again', exact: true }).click();
  await expect(port).toBeFocused(); await expect(token).toHaveCount(0);
  await form.getByRole('button', { name: 'Check node identity', exact: true }).click();
  await expect(token).toHaveValue(''); await token.fill('b'.repeat(64));
  await form.getByRole('button', { name: 'Save device connection', exact: true }).click();
  await expect(form).toHaveCount(0); await expect(page.locator('.direct-list')).toContainText('tailscale-laptop');
  expect(actions.filter(a => a.operation === 'tailscale_discover')[0]).toEqual({ operation: 'tailscale_discover', address: '100.64.0.2', port: 1234 });
  expect(actions.filter(a => a.operation === 'tailscale_connect')[1]).toEqual({ operation: 'tailscale_connect', address: '100.64.0.2', port: 1234, node: 'tailscale-laptop', token: 'b'.repeat(64) });
  expect(await page.evaluate(() => JSON.stringify([Object.entries(sessionStorage), Object.entries(localStorage)]))).not.toContain('b'.repeat(64));
  await page.reload(); await expect(ts).toContainText('Personal laptop');
  expect(lookups).toBe(3); expect(saves).toBe(2);
});

test('invitation enrollment keeps failed input and joins without exposing or persisting secrets', async ({ page }) => {
  const status = initial(), commands: any[] = [];
  let attempts = 0;
  let release: () => void = () => {};
  const response = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/network', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'easy_tier_enroll') {
      if (++attempts === 1) { await response; return route.fulfill({ status: 400, json: { error: 'Invitation is invalid, expired or already used.' } }); }
      Object.assign(status.easytier, { enabled: true, phase: 'running', network: 'home', peer: 'tcp://entry:11010', peers: ['tcp://entry:11010'] });
    }
    return route.fulfill({ json: status });
  });
  await page.goto('/#/nodes');
  const et = page.getByRole('article', { name: 'EasyTier', exact: true });
  await expect(et.getByRole('button', { name: 'Use invitation', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await et.getByLabel('Workspace address', { exact: true }).fill('https://workspace.example.com');
  const token = et.getByLabel('Invitation token', { exact: true });
  await token.fill('cbi_' + 'd'.repeat(64));
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await et.screenshot({ path: '/tmp/cranibox-network-enrollment-' + theme + '.png', animations: 'disabled' });
  }
  await et.getByRole('button', { name: 'Join with invitation', exact: true }).click();
  await expect(et.getByLabel('Workspace address', { exact: true })).toBeDisabled();
  await expect(token).toBeDisabled();
  await expect(et.getByRole('button', { name: 'Joining…', exact: true })).toBeDisabled();
  release();
  await expect(page.getByRole('region', { name: 'Device networks', exact: true }).getByRole('alert')).toContainText('already used');
  await expect(token).toHaveValue('cbi_' + 'd'.repeat(64));
  await et.getByRole('button', { name: 'Join with invitation', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('Running in background');
  await expect(et.getByLabel('Invitation token', { exact: true })).toHaveCount(0);
  expect(commands.filter(command => command.operation === 'easy_tier_enroll')[0]).toEqual({ operation: 'easy_tier_enroll', url: 'https://workspace.example.com', token: 'cbi_' + 'd'.repeat(64), listen_port: null });
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('d'.repeat(64));
  await page.reload(); await expect(et.getByRole('status')).toHaveText('Running in background'); expect(attempts).toBe(2);
});

test('invitation creation is explicit, shows its token once and remains revocable after reload', async ({ page }) => {
  const status: any = initial();
  Object.assign(status.easytier, { enabled: true, phase: 'running', network: 'home', peer: 'tcp://entry:11010', peers: ['tcp://entry:11010'], invitations: [] });
  const issued = { id: 'e'.repeat(32), token: 'cbi_' + 'f'.repeat(64), expires_at: Math.floor(Date.now() / 1000) + 900 };
  let creates = 0, revoked = '';
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'easy_tier_invite') {
      creates++; expect(command.peers).toEqual(['tcp://entry:11010', 'wss://backup.example.com/mesh']);
      status.easytier.invitations = [{ id: issued.id, expires_at: issued.expires_at }];
      return route.fulfill({ json: { network: status, invitation: issued } });
    }
    if (command.operation === 'easy_tier_revoke_invitation') { revoked = command.id; status.easytier.invitations = []; }
    return route.fulfill({ json: status });
  });
  await page.goto('/#/nodes');
  const et = page.getByRole('article', { name: 'EasyTier', exact: true });
  await et.getByText('Invite another device', { exact: true }).click();
  await et.getByLabel('Invitation entry addresses', { exact: true }).fill('tcp://entry:11010\nwss://backup.example.com/mesh');
  expect(creates).toBe(0);
  await et.getByRole('button', { name: 'Create 15 minute invitation', exact: true }).click();
  const result = et.getByRole('region', { name: 'New device invitation', exact: true });
  await expect(result.getByLabel('Share invitation token', { exact: true })).toHaveValue(issued.token);
  await expect(result.getByLabel('Share Workspace address', { exact: true })).toHaveValue(new URL(page.url()).origin);
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await et.screenshot({ path: '/tmp/cranibox-network-invitation-' + theme + '.png', animations: 'disabled' });
  }
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain(issued.token);
  status.easytier.invitations[0].redeemed_at = Math.floor(Date.now() / 1000);
  await page.getByRole('button', { name: 'Refresh device networks', exact: true }).click();
  await expect(result).toHaveCount(0);
  await expect(et.getByRole('list', { name: 'Active invitations', exact: true })).toContainText('Claimed · Only the original device can retry');
  await page.reload(); await et.getByText('Invite another device', { exact: true }).click();
  await expect(et.getByLabel('Share invitation token', { exact: true })).toHaveCount(0);
  await expect(et.getByRole('list', { name: 'Active invitations', exact: true })).toContainText('Expires at');
  await expect(et.getByRole('list', { name: 'Active invitations', exact: true })).toContainText('Claimed');
  await et.getByRole('button', { name: 'Revoke invitation eeeeeeee', exact: true }).click();
  await expect(et.getByRole('list', { name: 'Active invitations', exact: true })).toHaveCount(0);
  expect(revoked).toBe(issued.id); expect(creates).toBe(1);
});
