import { expect, test, type Page } from '@playwright/test';

async function api(page: Page, path: string, body: Record<string, unknown>) {
  return page.evaluate(async ({ path, body }) => {
    const response = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify(body) });
    const value = await response.json();
    if (!response.ok) throw Error(JSON.stringify(value));
    return value;
  }, { path, body });
}

// The Workspace host's own working copies; another device registers its own.
const host = async (page: Page) => (await api(page, '/api/peers', { operation: 'identity' })).node_id as string;

async function register(page: Page, name: string, path: string) {
  await page.getByRole('button', { name: 'Register project', exact: true }).click();
  await page.getByLabel('Project name', { exact: true }).fill(name);
  await page.getByLabel('Device', { exact: true }).selectOption(await host(page));
  await page.getByLabel('Absolute directory on device', { exact: true }).fill(path);
  await page.getByRole('button', { name: 'Save directory', exact: true }).click();
  await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
}

test('project shells reopen in place, share existing sessions and survive page reload', async ({ page }) => {
  const errors: string[] = [], operations: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    if (request.url().endsWith('/api/terminal/sessions')) operations.push(request.postDataJSON().operation);
  });
  await page.goto('/#/projects');
  await register(page, 'Cross-device project', '/tmp');
  await page.getByLabel('New session name', { exact: true }).fill('Project shell');
  await page.getByRole('button', { name: 'Create shell here', exact: true }).click();
  const pane = page.locator('.terminal-panel.active');
  await expect(pane.locator('.status')).toContainText('Connected');
  await pane.locator('.terminal-screen').click();
  await page.keyboard.type('export PROJECT_BROWSER=kept; printf "%s%s" PROJECT_READY _OK');
  await page.keyboard.press('Enter');
  await expect(pane.locator('.xterm-accessibility')).toContainText('PROJECT_READY_OK');
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await page.reload();
  await page.locator('.projects-list').getByRole('button', { name: /Cross-device project/ }).click();
  await expect(page.locator('.projects-sessions')).toContainText('Project shell');
  await page.locator('.projects-sessions').getByRole('button', { name: 'Open', exact: true }).click();
  await expect(pane.locator('.status')).toContainText('Connected');
  await pane.locator('.terminal-screen').click();
  await page.keyboard.type('printf "%s%s" REOPEN_ "$PROJECT_BROWSER"');
  await page.keyboard.press('Enter');
  await expect(pane.locator('.xterm-accessibility')).toContainText('REOPEN_kept');
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  expect(operations.filter(op => op === 'create')).toHaveLength(1);
  expect(operations).not.toContain('end');
  const original = await api(page, '/api/terminal/sessions', { node: await host(page), operation: 'list' });
  const session = original.sessions.find((s: any) => s.name === 'Project shell');
  await page.getByRole('button', { name: 'Unlink Project shell', exact: true }).click();
  await expect(page.locator('.projects-sessions li')).toHaveCount(0);
  await page.getByLabel('Link existing session', { exact: true }).selectOption(session.id);
  await page.getByRole('button', { name: 'Link', exact: true }).click();
  await expect(page.locator('.projects-sessions')).toContainText('Project shell');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: '/tmp/cranibox-projects-web-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
    }
  }
  expect(errors).toEqual([]);
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Remove directory record', exact: true }).click();
  await expect(page.locator('.projects-list button')).toHaveCount(0);
  const kept = await api(page, '/api/terminal/sessions', { node: await host(page), operation: 'list' });
  expect(kept.sessions.find((s: any) => s.id === session.id).status).toBe('running');
  await api(page, '/api/terminal/sessions', { node: await host(page), operation: 'end', id: session.id });
});

test('invalid target directories report failure without inventing a session', async ({ page }) => {
  await page.goto('/#/projects');
  await register(page, 'Missing working tree', '/cranibox-test-nonexistent-directory');
  const before = await api(page, '/api/terminal/sessions', { node: await host(page), operation: 'list' });
  await page.getByRole('button', { name: 'Create shell here', exact: true }).click();
  await expect(page.locator('.projects-view').getByRole('alert')).toBeVisible();
  await expect(page.locator('.projects-sessions li')).toHaveCount(0);
  const after = await api(page, '/api/terminal/sessions', { node: await host(page), operation: 'list' });
  expect(after.sessions.map((s: any) => s.id)).toEqual(before.sessions.map((s: any) => s.id));
  await expect(page.getByRole('button', { name: 'Create shell here', exact: true })).toBeEnabled();
});
