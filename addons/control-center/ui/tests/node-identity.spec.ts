import { expect, test, type Page } from '@playwright/test';

async function api(page: Page, path: string, body: Record<string, unknown>) {
  return page.evaluate(async ({ path, body }) => {
    const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify(body) });
    const value = await response.json();
    if (!response.ok) throw Error(JSON.stringify(value));
    return value;
  }, { path, body });
}

test('identity migration retains failed input, restores keyboard focus and keeps old IDs searchable', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const source = 'identity-ui-node', target = 'node-11111111111111111111111111111111';
  await page.goto('/#/nodes');
  await api(page, '/api/nodes/enroll', { id: source });
  await page.reload();
  try {
    const trigger = page.getByRole('button', { name: 'Migrate identity ' + source, exact: true });
    await trigger.click();
    const dialog = page.getByRole('dialog', { name: 'Migrate node identity', exact: true });
    const field = dialog.getByLabel('Target NodeID', { exact: true });
    await expect(field).toBeFocused();
    await expect(dialog.getByLabel('Previous node ID', { exact: true })).toHaveValue(source);
    await expect(dialog.getByRole('button', { name: 'Migrate identity', exact: true })).toBeDisabled();
    await page.keyboard.press('Escape');
    await expect(dialog).not.toBeVisible();
    await expect(trigger).toBeFocused();
    await trigger.click();
    await field.fill('preview-node');
    await dialog.getByRole('button', { name: 'Migrate identity', exact: true }).click();
    await expect(dialog.getByRole('alert')).toContainText('already registered');
    await expect(field).toHaveValue('preview-node');
    await expect(field).toBeFocused();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    for (const width of [375, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ['dark', 'light']) {
        await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        const bounds = await dialog.boundingBox();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
        if (width === 375 || width === 1440) await page.screenshot({ path: '/tmp/cranibox-identity-' + width + '-' + theme + '.png', animations: 'disabled' });
      }
    }
    await field.fill(target);
    await dialog.getByRole('button', { name: 'Migrate identity', exact: true }).click();
    await expect(dialog).not.toBeVisible();
    await expect(page.locator('.cx-notice[role="status"]')).toContainText('Node identity migrated');
    await page.getByLabel('Search nodes', { exact: true }).fill(source);
    const row = page.locator('.node-list .list-row');
    await expect(row).toHaveCount(1);
    await expect(row).toContainText(target);
    await expect(row).toContainText('Previous ID: ' + source);
    await page.reload();
    await expect(page.getByRole('button', { name: 'Migrate identity ' + target, exact: true })).toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await api(page, '/api/nodes/revoke', { id: source });
  }
});

test('a floating terminal reconnects to its original Shell after the device identity is migrated', async ({ page }) => {
  const target = 'node-22222222222222222222222222222222', operations: string[] = [];
  page.on('request', request => {
    if (request.url().endsWith('/api/terminal/sessions')) operations.push(request.postDataJSON().operation);
  });
  await page.goto('/#/nodes');
  const created = await api(page, '/api/terminal/sessions', { node: 'preview-node', operation: 'create', name: 'Identity Shell', cwd: '/tmp' });
  const session = created.session.id;
  let companion = '';
  let replicaId = '';
  try {
    companion = (await api(page, '/api/terminal/sessions', { node: 'preview-node', operation: 'create', name: 'Identity companion', cwd: '/tmp' })).session.id;
    const catalog = await api(page, '/api/projects', { operation: 'register', node: 'preview-node', name: 'Identity project', path: '/tmp/identity-project' });
    const replica = catalog.projects.flatMap((project: any) => project.replicas).find((replica: any) => replica.node === 'preview-node' && replica.path === '/tmp/identity-project');
    replicaId = replica.id;
    await api(page, '/api/projects', { operation: 'bind', replica_id: replica.id, terminal_id: session });
    await page.goto('/#/projects');
    await page.locator('.projects-list').getByRole('button', { name: /Identity project/ }).click();
    await page.locator('.projects-sessions').getByRole('button', { name: 'Open', exact: true }).click();
    const floating = page.locator('#floating-terminal'), pane = floating.locator('.terminal-panel.active');
    await expect(pane.locator('.status')).toContainText('Connected');
    await pane.locator('.terminal-screen').click();
    await page.keyboard.type('export IDENTITY_BROWSER=kept; printf "%s%s" BEFORE_ "$IDENTITY_BROWSER"');
    await page.keyboard.press('Enter');
    await expect(pane.locator('.xterm-accessibility')).toContainText('BEFORE_kept');
    await floating.getByRole('button', { name: 'Session actions Identity companion', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Open beside', exact: true }).click();
    await expect(floating.locator('.terminal-panes')).toHaveClass(/is-split/);
    await floating.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
    await page.goto('/#/nodes');
    await page.getByRole('button', { name: 'Migrate identity preview-node', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Migrate node identity', exact: true });
    await dialog.getByLabel('Target NodeID', { exact: true }).fill(target);
    // The dialog closes before the old transport disconnects. Wait for this
    // Shell's new attachment so input cannot race with the identity reconnect.
    const reattached = page.waitForResponse(response => {
      if (!response.url().endsWith('/api/terminal/ticket') || response.request().method() !== 'POST') return false;
      const body = response.request().postDataJSON();
      return response.ok() && body.node === target && body.session_id === session;
    });
    await dialog.getByRole('button', { name: 'Migrate identity', exact: true }).click();
    await expect(dialog).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'Migrate identity ' + target, exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Restore terminal window', exact: true }).click();
    await reattached;
    await expect(pane.locator('.status')).toContainText('Connected');
    await pane.locator('.terminal-screen').click();
    await page.keyboard.type('printf "%s%s" AFTER_ "$IDENTITY_BROWSER"');
    await page.keyboard.press('Enter');
    await expect(pane.locator('.xterm-accessibility')).toContainText('AFTER_kept');
    await expect(floating.getByRole('combobox', { name: 'Target', exact: true })).toHaveValue(target);
    await expect(floating.getByRole('tab', { name: 'Identity Shell', exact: true })).toHaveCount(1);
    await expect(floating.locator('.terminal-panes')).toHaveClass(/is-split/);
    await expect(floating.locator('.terminal-panel.beside .status')).toContainText('Connected');
    await floating.getByRole('button', { name: 'Close terminal window', exact: true }).click();
    await page.goto('/#/projects');
    await page.locator('.projects-list').getByRole('button', { name: /Identity project/ }).click();
    await expect(page.locator('.projects-location')).toContainText('Preview Linux node');
    await expect(page.locator('.projects-sessions')).toContainText('Identity Shell');
    const migrated = await api(page, '/api/projects', { operation: 'list' });
    const retained = migrated.projects.flatMap((project: any) => project.replicas).find((replica: any) => replica.id === replicaId);
    expect(retained.node).toBe(target);
    expect(retained.sessions.map((binding: any) => binding.terminal_id)).toContain(session);
    await page.locator('.projects-sessions').getByRole('button', { name: 'Open', exact: true }).click();
    await expect(pane.locator('.status')).toContainText('Connected');
    await expect(floating.getByRole('tab', { name: 'Identity Shell', exact: true })).toHaveCount(1);
    const original = await api(page, '/api/terminal/sessions', { node: target, operation: 'list' });
    expect(original.sessions.find((shell: any) => shell.id === session).status).toBe('running');
    expect(operations.filter(operation => operation === 'create')).toHaveLength(2);
    expect(operations).not.toContain('end');
  } finally {
    await api(page, '/api/nodes/identity', { id: 'preview-node', target: 'preview-node' });
    await expect.poll(async () => {
      const response = await api(page, '/api/poll', { refresh: true });
      return response.snapshot.remote_nodes.nodes.some((node: any) => node.id === 'preview-node' && node.online);
    }, { timeout: 15000 }).toBe(true);
    if (replicaId) await api(page, '/api/projects', { operation: 'forget', replica_id: replicaId });
    await api(page, '/api/terminal/sessions', { node: 'preview-node', operation: 'end', id: session });
    if (companion) await api(page, '/api/terminal/sessions', { node: 'preview-node', operation: 'end', id: companion });
  }
});
