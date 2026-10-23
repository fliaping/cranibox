import { expect, test } from '@playwright/test';

test('space applications open an isolated tab and recover from gateway errors without revealing credentials', async ({ page, context }) => {
  const commands: any[] = [];
  let fail = true;
  await page.route('**/api/spaces', route => route.fulfill({ json: { platform: 'linux', default_image: 'local:test', setup: null, spaces: [{ id: 'space-application', name: 'My space', provider: 'docker', image: 'local:test', node: 'space-node', phase: 'ready', error: null, volume: 'persistent', ports: { desktop: 43001, code: 48443 } }] } }));
  await page.route('**/api/spaces/application', async route => {
    commands.push(route.request().postDataJSON());
    await route.fulfill(fail ? { status: 400, json: { error: 'Application gateway unavailable' } } : { json: { url: 'http://applications.test/cranibox-app-test-desktop/' } });
  });
  await context.route('http://applications.test/**', route => route.fulfill({ contentType: 'text/html', body: '<title>Space desktop</title><p>Remote space</p>' }));
  await page.goto('/#/spaces');
  const space = page.getByRole('article', { name: 'My space', exact: true });
  const failedTab = context.waitForEvent('page');
  await space.getByRole('button', { name: 'Desktop', exact: true }).click();
  const closed = await failedTab;
  await expect(page.getByRole('alert')).toContainText('Application gateway unavailable');
  await expect.poll(() => closed.isClosed()).toBe(true);
  await expect(space.getByRole('button', { name: 'Desktop', exact: true })).toBeEnabled();
  fail = false;
  const popup = context.waitForEvent('page');
  await space.getByRole('button', { name: 'Editor', exact: true }).click();
  const tab = await popup;
  try {
    await expect(tab).toHaveURL('http://applications.test/cranibox-app-test-desktop/');
    await expect(tab).toHaveTitle('Space desktop');
    expect(await tab.evaluate(() => window.opener)).toBeNull();
  } finally { await tab.close(); }
  expect(commands).toEqual([{ id: 'space-application', application: 'desktop' }, { id: 'space-application', application: 'code' }]);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('space-application');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      for (const name of ['Desktop', 'Editor']) {
        const bounds = await space.getByRole('button', { name, exact: true }).boundingBox();
        expect(bounds?.height).toBeGreaterThanOrEqual(44);
      }
    }
  }
});

test('shared space directories retain ownership, require permission and recover without container operations', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  let granted = true, unavailable = false;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { node_id: 'local-manager', routes: [{ node: 'linux-manager', url: 'http://linux-manager.test:9091', allow_private_http: true }, { node: 'mac-manager', url: 'http://mac-manager.test:9091', allow_private_http: true }], grants: [] } });
    if (command.operation === 'probe') return route.fulfill({ json: { nodes: [{ id: command.node, spaces: command.node === 'linux-manager' && granted }] } });
    if (command.operation === 'space_catalog' && unavailable) return route.fulfill({ status: 400, json: { error: 'Connection unavailable' } });
    if (command.operation === 'space_catalog') return route.fulfill({ json: { owner: command.node, spaces: [{ id: '11111111-1111-4111-8111-111111111111', name: '<Remote Agent lab>', node: 'remote-space-node', provider: 'docker', phase: 'pulling' }] } });
    throw Error('Unexpected directory operation: ' + command.operation);
  });
  await page.goto('/#/spaces');
  const panel = page.getByRole('region', { name: 'Spaces on other devices', exact: true });
  const source = panel.getByRole('article', { name: 'linux-manager', exact: true });
  await expect(source.getByText('<Remote Agent lab>', { exact: true })).toBeVisible();
  await expect(source).toContainText('Downloading image');
  await expect(source).toContainText('remote-space-node');
  await expect(source).toContainText('containers were not inspected');
  const details = panel.locator('.remote-space-details');
  await expect(details.locator('summary')).toContainText('1 not shared');
  await expect(details).not.toHaveAttribute('open');
  await expect(panel.getByRole('article', { name: 'mac-manager', exact: true })).toHaveCount(0);
  await expect(panel.getByRole('status')).toContainText('1 space · 1/2');
  expect(commands.some(command => command.operation === 'space_catalog' && command.node === 'mac-manager')).toBe(false);
  expect(await source.getByRole('button').count()).toBe(0);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375) await panel.screenshot({ path: '/tmp/cranibox-shared-spaces-' + theme + '.png', animations: 'disabled' });
    }
  }
  await details.locator('summary').focus();
  await page.keyboard.press('Enter');
  await expect(panel.getByRole('article', { name: 'mac-manager', exact: true })).toContainText('Not shared');
  await expect(details).toContainText('Space catalog access is missing');
  unavailable = true;
  await panel.getByRole('button', { name: 'Refresh remote spaces' }).click();
  await expect(source).toContainText('Connection unavailable');
  await expect(details).toHaveAttribute('open', '');
  await expect(details).toContainText('does not mean its spaces were deleted');
  await expect(panel.getByText('<Remote Agent lab>', { exact: true })).toHaveCount(0);
  unavailable = false;
  await page.reload();
  await expect(source.getByText('<Remote Agent lab>', { exact: true })).toBeVisible();
  await details.locator('summary').click();
  granted = false;
  await panel.getByRole('button', { name: 'Refresh remote spaces' }).click();
  await expect(source).toContainText('Not shared');
  await expect(panel.getByText('<Remote Agent lab>', { exact: true })).toHaveCount(0);
  expect(commands.every(command => ['list', 'probe', 'space_catalog'].includes(command.operation))).toBe(true);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Remote Agent lab');
  await panel.getByRole('button', { name: 'Device connections', exact: true }).click();
  await expect(page).toHaveURL(/#\/nodes$/);
  expect(errors).toEqual([]);
});

test('many unshared devices stay compact while shared space nodes and recoverable errors remain discoverable', async ({ page }) => {
  const privateNodes = Array.from({ length: 24 }, (_, index) => 'private-device-' + index + '-with-a-long-name');
  const nodes = ['node-space-owner', 'offline-device', ...privateNodes, 'zz-checking-device'];
  let recovered = false, release: () => void = () => {};
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'list') return route.fulfill({ json: { routes: nodes.map(node => ({ node })) } });
    if (command.operation === 'probe') {
      if (command.node === 'zz-checking-device') await held;
      if (command.node === 'offline-device' && !recovered) return route.fulfill({ status: 400, json: { error: 'Device connection timed out' } });
      return route.fulfill({ json: { nodes: [{ id: command.node, spaces: ['node-space-owner', 'offline-device'].includes(command.node) }] } });
    }
    if (command.operation === 'space_catalog') return route.fulfill({ json: { owner: command.node, spaces: command.node === 'node-space-owner' ? [{ id: 'nested-space', name: 'Nested research space', node: 'nested-node', provider: 'docker', phase: 'ready' }] : [] } });
    throw Error('Unexpected directory operation: ' + command.operation);
  });
  try {
    await page.goto('/#/spaces');
    const panel = page.getByRole('region', { name: 'Spaces on other devices', exact: true });
    const details = panel.locator('.remote-space-details'), summary = details.locator('summary');
    await expect(panel.getByText('Nested research space', { exact: true })).toBeVisible();
    await expect(summary).toContainText('24 not shared · 1 unavailable · 1 checking');
    await expect(panel.getByRole('article')).toHaveCount(1);
    await expect(panel.getByRole('status')).toContainText('1 space · 1/27');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        await panel.screenshot({ path: '/tmp/cranibox-space-directory-' + width + '-' + theme + '.png', animations: 'disabled' });
      }
    }
    await summary.focus();
    await page.keyboard.press('Enter');
    const offline = panel.getByRole('article', { name: 'offline-device', exact: true });
    await expect(offline).toContainText('Device connection timed out');
    await expect(panel.getByRole('article', { name: privateNodes[0], exact: true })).toContainText('Not shared');
    await expect(panel.getByRole('article')).toHaveCount(26);
    await expect(details).toContainText('Checking directory access on the remaining devices');
    await expect(panel.getByRole('alert')).toHaveCount(0);
    await page.setViewportSize({ width: 375, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await details.screenshot({ path: '/tmp/cranibox-space-directory-expanded.png', animations: 'disabled' });
    release();
    await expect(summary).toContainText('25 not shared · 1 unavailable');
    recovered = true;
    await panel.getByRole('button', { name: 'Refresh remote spaces' }).click();
    await expect(offline).toContainText('This device’s space directory is empty.');
    await expect(summary).toHaveText('Device directory status25 not shared');
    await expect(details).toHaveAttribute('open', '');
    await expect(panel.getByRole('status')).toContainText('1 space · 2/27');
    await expect(panel.getByText('Device connection timed out')).toHaveCount(0);
  } finally { release(); }
});

test('removed connections cannot restore a space directory from an older in-flight response', async ({ page }) => {
  let routes = [{ node: 'former-manager' }], release: () => void = () => {}, started = false;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'list') return route.fulfill({ json: { routes } });
    if (command.operation === 'probe') return route.fulfill({ json: { nodes: [{ id: 'former-manager', spaces: true }] } });
    started = true; await held;
    await route.fulfill({ json: { owner: 'former-manager', spaces: [{ id: '11111111-1111-4111-8111-111111111111', name: 'Former private space', node: 'space-node', provider: 'docker', phase: 'ready' }] } });
  });
  await page.goto('/#/spaces');
  await expect.poll(() => started).toBe(true);
  routes = [];
  await page.evaluate(() => window.dispatchEvent(new Event('cranibox:peers-changed')));
  release();
  const panel = page.getByRole('region', { name: 'Spaces on other devices', exact: true });
  await expect(panel.getByRole('status')).toContainText('Connect another device');
  await expect(panel.getByText('Former private space', { exact: true })).toHaveCount(0);
  await expect(panel.getByRole('article')).toHaveCount(0);
});

test('guided setup reviews installation, enrolls a space and retains it across reload and archive', async ({ page }) => {
  const commands: any[] = [], errors: string[] = [];
  const images = { registries: [{ id: 'dockerhub', repository: 'docker.io/xuping/agent-workspace' }, { id: 'ghcr', repository: 'ghcr.io/fliaping/agent-workspace' }, { id: 'aliyun', repository: 'registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace' }], desktops: [] };
  const catalog: any = { spaces: [], platform: 'linux', default_image: 'cranibox-test:latest', setup: null, images };
  let checks = 0;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/spaces', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    let value = catalog;
    if (command.operation === 'preflight') value = { ready: checks++ > 0, detail: 'Start the local Docker engine.', notice: 'Runs on this management device.', engine: { name: 'Development laptop', version: 'test', os: 'Linux' }, actions: ['linux_docker'] };
    if (command.operation === 'registries') value = [{ id: 'dockerhub', milliseconds: null }, { id: 'ghcr', milliseconds: 310 }, { id: 'aliyun', milliseconds: 42 }];
    if (command.operation === 'plan') value = { action: command.action, plan_id: 'review-only', expires_in_seconds: 600, summary: 'Install Docker on this device.', commands: 'apt-get install docker-ce', administrator: true };
    if (command.operation === 'create') catalog.spaces = [{ id: 'space-1', name: command.name, provider: command.provider, image: 'registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace:ubuntu-' + command.desktop, registry: command.registry, desktop: command.desktop, display: command.display, pull_policy: command.pull_policy, node: 'space-node', phase: 'enrolling', error: null, volume: 'cranibox-data-stable', ports: { desktop: 43001, code: 48443 } }];
    if (command.operation === 'stop') catalog.spaces[0].phase = 'stopped';
    if (command.operation === 'remove') catalog.spaces[0].phase = 'archived';
    if (command.operation === 'resume') catalog.spaces[0].phase = 'ready';
    if (command.operation === 'credentials') value = { username: 'agent', password: 'only-revealed-on-request' };
    await route.fulfill({ json: value });
  });
  await page.goto('/#/spaces');
  const section = page.getByRole('region', { name: 'Virtual spaces', exact: true });
  await expect(section.getByRole('heading', { name: 'No spaces managed on this device' })).toBeVisible();
  await section.getByRole('button', { name: 'New space', exact: true }).click();
  await section.getByRole('button', { name: 'Check environment', exact: true }).click();
  await section.getByRole('button', { name: 'Install Docker Engine', exact: true }).click();
  await expect(section.getByLabel('Setup commands to execute')).toContainText('apt-get install docker-ce');
  await section.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(commands.some(command => command.operation === 'install')).toBe(false);
  await section.getByRole('button', { name: 'Check environment', exact: true }).click();
  await section.getByLabel('Space name', { exact: true }).fill('Agent playground');
  await section.getByLabel('Desktop', { exact: true }).selectOption('kde');
  await section.getByLabel('Display server', { exact: true }).selectOption('x11');
  await section.getByText('Advanced: image source', { exact: true }).click();
  // Automatic selection names the fastest registry measured from this device.
  await expect(section.getByLabel('Registry', { exact: true }).locator('option[value=auto]')).toHaveText('Automatic (fastest reachable) · Alibaba Cloud mirror');
  await expect(section.getByLabel('Registry', { exact: true }).locator('option[value=dockerhub]')).toHaveText('Docker Hub · unreachable');
  await expect(section).toContainText('registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace:ubuntu-kde');
  await section.getByLabel('Registry', { exact: true }).selectOption('ghcr');
  await expect(section).toContainText('ghcr.io/fliaping/agent-workspace:ubuntu-kde');
  await section.getByLabel('Registry', { exact: true }).selectOption('aliyun');
  await expect(section.getByLabel('Image pull policy', { exact: true })).toHaveValue('always');
  await section.getByLabel('Image pull policy', { exact: true }).selectOption('never');
  await expect(section).toContainText('without contacting a registry');
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await section.getByRole('button', { name: 'Next', exact: true }).click();
  await expect(section.locator('.spaces-review')).toContainText('2 CPU');
  await expect(section.locator('.spaces-review')).toContainText('Use local image only');
  await expect(section.locator('.spaces-review')).toContainText('KDE Plasma · X11 (compatibility)');
  await expect(section.locator('.spaces-review')).toContainText('registry.cn-hangzhou.aliyuncs.com/fliaping/agent-workspace:ubuntu-kde');
  await section.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(section.getByLabel('Space name', { exact: true })).toHaveValue('Agent playground');
  await section.getByRole('button', { name: 'Next', exact: true }).click();
  await section.getByRole('button', { name: 'Create and start', exact: true }).click();
  const card = section.getByRole('article', { name: 'Agent playground', exact: true });
  await expect(card.getByRole('status')).toHaveText('Waiting for node');
  catalog.spaces[0].phase = 'ready';
  await expect(card.getByRole('status')).toHaveText('Ready');
  await page.reload();
  await expect(card.getByRole('status')).toHaveText('Ready');
  await card.locator('summary').click();
  expect(commands.some(command => command.operation === 'credentials')).toBe(false);
  await card.getByRole('button', { name: 'View login credentials' }).click();
  await expect(card.getByLabel('Space password')).toHaveValue('only-revealed-on-request');
  await card.getByRole('button', { name: 'Hide credentials' }).click();
  await expect(card.getByLabel('Space password')).toHaveCount(0);
  page.once('dialog', dialog => dialog.accept());
  await card.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(card.getByRole('status')).toHaveText('Stopped');
  page.once('dialog', dialog => dialog.dismiss());
  await card.getByRole('button', { name: 'Archive', exact: true }).click();
  expect(commands.some(command => command.operation === 'remove')).toBe(false);
  page.once('dialog', dialog => dialog.accept());
  await card.getByRole('button', { name: 'Archive', exact: true }).click();
  await expect(card.getByRole('status')).toContainText('Archived');
  await card.getByRole('button', { name: 'Restore space', exact: true }).click();
  await expect(card.getByRole('status')).toHaveText('Ready');
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375 || width === 1440) await page.screenshot({ path: '/tmp/cranibox-spaces-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
    }
  }
  expect(commands.find(command => command.operation === 'create')).toEqual({ operation: 'create', name: 'Agent playground', provider: 'docker', registry: 'aliyun', desktop: 'kde', display: 'x11', pull_policy: 'never' });
  expect(errors).toEqual([]);
});

test('failed spaces can review a new image, recover errors and retry once with the same identity', async ({ page }, testInfo) => {
  const commands: any[] = [];
  const space = { id: 'space-recovery', name: 'Image recovery', provider: 'docker', image: 'old:image', pull_policy: 'always', node: 'persistent-node', phase: 'failed', error: 'This image does not support virtual-space enrollment.', volume: 'persistent-volume', ports: {} };
  const catalog = { spaces: [space], platform: 'linux', default_image: 'old:image', setup: null };
  let attempts = 0, release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/spaces', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'resume') {
      if (++attempts === 1) { await route.fulfill({ status: 400, json: { error: 'Archive this space before changing its image. Its container was not changed.' } }); return; }
      await pending; Object.assign(space, { image: command.image, pull_policy: command.pull_policy, phase: 'preparing', error: null });
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/spaces');
  const card = page.getByRole('article', { name: 'Image recovery', exact: true });
  const change = card.getByRole('button', { name: 'Change image', exact: true });
  await change.click();
  const form = card.getByRole('form', { name: 'Change space image', exact: true });
  await expect(form.getByLabel('Image reference', { exact: true })).toBeFocused();
  await form.getByLabel('Image reference', { exact: true }).fill('unsubmitted:image');
  await form.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(change).toBeFocused();
  expect(commands.filter(command => command.operation === 'resume')).toEqual([]);
  await change.click();
  await expect(form.getByLabel('Image reference', { exact: true })).toHaveValue('old:image');
  const image = 'registry.example.com/cranibox/current-workspace-image-for-development:compatible';
  await form.getByLabel('Image reference', { exact: true }).fill(image);
  await form.getByLabel('Image pull policy', { exact: true }).selectOption('never');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      for (const control of await form.locator('button, input, select').all()) expect((await control.boundingBox())!.height).toBeGreaterThanOrEqual(44);
      await page.screenshot({ path: testInfo.outputPath('image-recovery-' + width + '-' + theme + '.png'), fullPage: true, animations: 'disabled' });
    }
  }
  await form.getByRole('button', { name: 'Save and retry', exact: true }).click();
  await expect(form.getByRole('alert')).toContainText('Archive this space');
  await expect(form.getByRole('alert')).toBeFocused();
  await expect(form.getByLabel('Image reference', { exact: true })).toHaveValue(image);
  await expect(form.getByLabel('Image pull policy', { exact: true })).toHaveValue('never');
  await form.getByRole('button', { name: 'Save and retry', exact: true }).evaluate((button: HTMLButtonElement) => { button.click(); button.click(); });
  await expect(form.getByRole('button', { name: 'Submitting…', exact: true })).toBeDisabled();
  await expect.poll(() => attempts).toBe(2);
  release();
  await expect(form).toHaveCount(0);
  await expect(card).toBeFocused();
  await expect(page.getByRole('status').filter({ hasText: 'Image settings saved' })).toBeVisible();
  expect(commands.filter(command => command.operation === 'resume')).toEqual(Array(2).fill({ operation: 'resume', id: space.id, image, display: 'wayland', pull_policy: 'never' }));
  expect(space.node).toBe('persistent-node'); expect(space.volume).toBe('persistent-volume');
  space.phase = 'ready';
  await page.reload();
  await expect(card.getByRole('status')).toHaveText('Ready');
  await card.getByText('Data and access', { exact: true }).click();
  await expect(card).toContainText(image);
  await expect(card).toContainText('Use local image only');
});

test('space setup requires a fresh review after expiry or a lost response and never repeats an accepted install', async ({ page }) => {
  const commands: any[] = [];
  const catalog: any = { spaces: [], platform: 'linux', default_image: 'local:test', setup: null };
  let plans = 0;
  await page.route('**/api/spaces', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    if (command.operation === 'preflight') return route.fulfill({ json: { ready: false, detail: 'Docker is not ready.', notice: 'Runs on this device.', actions: ['linux_docker'], engine: null } });
    if (command.operation === 'plan') return route.fulfill({ json: { action: command.action, plan_id: 'review-' + ++plans, expires_in_seconds: 600, summary: 'Install Docker.', commands: 'apt-get install docker-ce', administrator: true } });
    if (command.operation === 'install') {
      if (command.plan_id === 'review-1') return route.fulfill({ status: 400, json: { error: 'Setup plan expired. Review the setup action again.' } });
      catalog.setup = { action: 'linux_docker', phase: 'installing', error: null };
      // The server accepted the operation but the browser did not receive its response.
      return route.fulfill({ status: 502, json: { error: 'Setup response unavailable. Refresh to check its status.' } });
    }
    return route.fulfill({ json: catalog });
  });
  await page.goto('/#/spaces');
  const section = page.getByRole('region', { name: 'Virtual spaces', exact: true });
  await section.getByRole('button', { name: 'New space', exact: true }).click();
  await section.getByRole('button', { name: 'Check environment', exact: true }).click();
  const prepare = section.getByRole('button', { name: 'Install Docker Engine', exact: true });
  await prepare.click();
  await expect(section.getByRole('region', { name: 'Review setup action', exact: true })).toBeFocused();
  await section.getByRole('button', { name: 'Confirm and run setup', exact: true }).click();
  await expect(section.getByRole('alert')).toContainText('expired');
  await expect(section.getByRole('alert')).toBeFocused();
  await expect(section.getByRole('button', { name: 'Confirm and run setup', exact: true })).toHaveCount(0);
  await prepare.click();
  // Dispatch twice in one browser task to cover the React render gap as well as disabled buttons.
  await section.getByRole('button', { name: 'Confirm and run setup', exact: true }).evaluate(button => { (button as HTMLButtonElement).click(); (button as HTMLButtonElement).click(); });
  await expect(section.locator('.spaces-setup-status')).toContainText('Closing this page does not cancel setup');
  await expect(prepare).toBeDisabled();
  await expect(section.getByRole('button', { name: 'Confirm and run setup', exact: true })).toHaveCount(0);
  await page.reload();
  await expect(section.locator('.spaces-setup-status')).toContainText('running in the background');
  await expect(section.getByRole('button', { name: 'New space', exact: true })).toBeDisabled();
  catalog.setup = { action: 'linux_docker', phase: 'interrupted', error: 'The system installer may still be running. Recheck the environment.' };
  await section.getByRole('button', { name: 'Refresh spaces', exact: true }).click();
  await expect(section.locator('.spaces-setup-status')).toContainText('may still be running');
  await expect(section.getByRole('button', { name: 'New space', exact: true })).toBeEnabled();
  expect(commands.filter(command => command.operation === 'install')).toEqual([
    { operation: 'install', action: 'linux_docker', plan_id: 'review-1' },
    { operation: 'install', action: 'linux_docker', plan_id: 'review-2' },
  ]);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('review-');
});

test('setup review stays readable on small screens and cancellation restores keyboard focus without installing', async ({ page }) => {
  const commands: any[] = [];
  await page.route('**/api/spaces', async route => {
    const command = route.request().postDataJSON(); commands.push(command);
    const value = command.operation === 'preflight' ? { ready: false, detail: 'Docker needs setup.', notice: '', actions: ['linux_docker'], engine: null }
      : command.operation === 'plan' ? { action: command.action, plan_id: 'cancelled-review', expires_in_seconds: 600, summary: 'Install Docker Engine on this management device. Administrator approval is required.', commands: 'apt-get install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin', administrator: true }
      : { spaces: [], platform: 'linux', default_image: 'local:test', setup: null };
    return route.fulfill({ json: value });
  });
  await page.goto('/#/spaces');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const section = page.getByRole('region', { name: 'Virtual spaces', exact: true });
  await section.getByRole('button', { name: 'New space', exact: true }).click();
  await section.getByRole('button', { name: 'Check environment', exact: true }).click();
  const prepare = section.getByRole('button', { name: 'Install Docker Engine', exact: true });
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      await prepare.click();
      const review = section.getByRole('region', { name: 'Review setup action', exact: true });
      await expect(review).toBeFocused();
      await expect(review).toContainText('ten minutes');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      for (const name of ['Confirm and run setup', 'Cancel']) {
        const bounds = await review.getByRole('button', { name, exact: true }).boundingBox();
        expect(bounds?.height).toBeGreaterThanOrEqual(44);
      }
      await page.evaluate(() => scrollTo(0, 0));
      await page.screenshot({ path: '/tmp/cranibox-space-setup-review-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
      await review.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(prepare).toBeFocused();
      await expect(review).toHaveCount(0);
    }
  }
  expect(commands.some(command => command.operation === 'install')).toBe(false);
});

test('the real Rust catalog is accessible from both language routes without accessing Docker', async ({ page }) => {
  for (const [origin, title] of [['http://127.0.0.1:19080', 'A separate space for every task'], ['http://127.0.0.1:19082', '给每个任务，一个独立空间']]) {
    await page.goto(origin + '/#/spaces');
    await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(page.locator('.spaces-empty')).toBeVisible();
    await expect(page.locator('.virtual-spaces [role="alert"]')).toHaveCount(0);
  }
});
