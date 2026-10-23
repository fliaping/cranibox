import { expect, test, type Page } from '@playwright/test';

async function api(page: Page, path: string, body: Record<string, unknown>) {
  return page.evaluate(async ({ path, body }) => {
    const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify(body) });
    const value = await response.json();
    if (!response.ok) throw Error(JSON.stringify(value));
    return value;
  }, { path, body });
}

test('Web host uses one stable identity for device, project and restored terminal views', async ({ page }) => {
  const errors: string[] = [], creates: unknown[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    if (request.url().endsWith('/api/terminal/sessions') && request.postDataJSON().operation === 'create') creates.push(request.postDataJSON());
  });
  // The host is "this device" on the devices page.
  await page.goto('/#/app/devices');
  const host = (await api(page, '/api/peers', { operation: 'identity' })).node_id;
  const card = page.locator('.cx-device').filter({ hasText: 'This device' });
  await expect(card).toHaveCount(1);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375 || width === 1440) await page.screenshot({ path: '/tmp/cranibox-host-node-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
    }
  }
  await card.getByRole('button', { name: 'Terminal', exact: true }).click();
  const pane = page.locator('.terminal-panel.active');
  await expect(pane.locator('.status')).toContainText('Connected');
  await expect(page.getByRole('combobox', { name: 'Target', exact: true })).toHaveValue(host);
  await pane.locator('.terminal-screen').click();
  await page.keyboard.type('export HOST_BROWSER=kept; printf "%s%s" HOST_READY _OK');
  await page.keyboard.press('Enter');
  await expect(pane.locator('.xterm-accessibility')).toContainText('HOST_READY_OK');
  const catalog = await api(page, '/api/terminal/sessions', { node: host, operation: 'list' });
  const shell = catalog.sessions[0];
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  // Simulate a pre-upgrade tab: empty node meant the Web host.
  await page.evaluate(shell => {
    sessionStorage.setItem('cranibox.terminal.tabs', JSON.stringify([{ node: '', id: shell.id, name: shell.name, cwd: shell.cwd }]));
    sessionStorage.setItem('cranibox.terminal.window', JSON.stringify({ mode: 'open', selection: { node: '', id: shell.id } }));
  }, shell);
  await page.reload();
  await expect(pane.locator('.status')).toContainText('Connected');
  await expect(page.getByRole('combobox', { name: 'Target', exact: true })).toHaveValue(host);
  await expect(page.getByRole('tablist', { name: 'Open terminals' }).getByRole('tab')).toHaveCount(1);
  await pane.locator('.terminal-screen').click();
  await page.keyboard.type('printf "%s%s" HOST_REOPEN_ "$HOST_BROWSER"');
  await page.keyboard.press('Enter');
  await expect(pane.locator('.xterm-accessibility')).toContainText('HOST_REOPEN_kept');
  await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem('cranibox.terminal.tabs') || '[]')[0]?.node)).toBe(host);
  expect(creates).toHaveLength(1);
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await page.goto('/#/projects');
  await page.getByRole('button', { name: 'Register project', exact: true }).click();
  await expect(page.getByLabel('Device', { exact: true })).toHaveValue(host);
  await page.getByLabel('Project name', { exact: true }).fill('Workspace host project');
  await page.getByLabel('Absolute directory on device', { exact: true }).fill('/tmp');
  await page.getByRole('button', { name: 'Save directory', exact: true }).click();
  await page.getByLabel('Link existing session', { exact: true }).selectOption(shell.id);
  await page.getByRole('button', { name: 'Link', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Unlink ' + shell.name, exact: true })).toBeVisible();
  const projects = await api(page, '/api/projects', { operation: 'list' });
  const project = projects.projects.find((p: any) => p.name === 'Workspace host project');
  expect(project.replicas[0].node).toBe(host);
  expect(project.replicas[0].sessions[0].terminal_id).toBe(shell.id);
  await page.locator('.projects-sessions').getByRole('button', { name: 'Open', exact: true }).click();
  await expect(pane.locator('.status')).toContainText('Connected');
  expect(creates).toHaveLength(1);
  expect(errors).toEqual([]);
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await api(page, '/api/terminal/sessions', { node: host, operation: 'end', id: shell.id });
  await api(page, '/api/projects', { operation: 'forget', replica_id: project.replicas[0].id });
});
