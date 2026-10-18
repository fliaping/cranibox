import { expect, test } from '@playwright/test';

const projectId = '22222222-2222-4222-8222-222222222222';
const replicaId = '33333333-3333-4333-8333-333333333333';
const shell = { id: '11111111-1111-4111-8111-111111111111', name: 'Original direct Shell', cwd: '/home/me/shared', status: 'running' };

test('federated copies keep their owner, shared ProjectID and Shell through source failures', async ({ page }) => {
  const actions: any[] = [], errors: string[] = [];
  let revoked = false;
  const remote = { owner: 'direct-laptop', revision: 1, permissions: { projects: true, terminals: true }, projects: [{ id: projectId, name: 'Shared project', replicas: [{ id: replicaId, node: 'direct-laptop', path: shell.cwd, sessions: [] as any[] }] }] };
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [{ id: projectId, name: 'Shared project', replicas: [{ id: 'gateway-copy', node: null, path: '/workspace/shared', sessions: [] }] }] } }));
  await page.route('**/api/peers', async route => {
    const action = route.request().postDataJSON(); actions.push(action);
    if (action.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'direct-laptop' }, { node: 'unavailable-desktop' }] } });
    if (action.node === 'unavailable-desktop' || revoked) return route.fulfill({ status: 400, json: { error: 'Project access revoked. Request a project catalog grant.' } });
    if (action.operation === 'sessions') return route.fulfill({ json: { installed: true, sessions: [shell] } });
    if (action.command.operation === 'register') {
      remote.projects[0].replicas.push({ id: '44444444-4444-4444-8444-444444444444', node: action.command.node, path: action.command.path, sessions: [] }); remote.revision++;
    }
    if (action.command.operation === 'bind') { remote.projects[0].replicas[0].sessions = [{ id: 'binding', terminal_id: action.command.terminal_id }]; remote.revision++; }
    await route.fulfill({ json: remote });
  });
  await page.route('**/api/terminal/sessions', route => route.fulfill({ json: { installed: true, sessions: [shell] } }));
  await page.route('**/api/terminal/ticket', async route => { actions.push(route.request().postDataJSON()); await route.fulfill({ json: { ticket: 'catalog-direct-ticket' } }); });
  await page.routeWebSocket('**/api/terminal/ws?ticket=catalog-direct-ticket', socket => socket.send(JSON.stringify({ type: 'ready' })));
  await page.goto('/#/projects');
  await expect(page.locator('.projects-list button')).toHaveCount(1);
  await expect(page.locator('.projects-list')).toContainText('2 working copies');
  await page.locator('.projects-sources summary').click();
  await expect(page.locator('.projects-sources')).toContainText('2/3 available');
  await expect(page.locator('.projects-sources')).toContainText('Project access revoked');
  await page.getByLabel('Working copy', { exact: true }).selectOption('direct:direct-laptop/' + replicaId);
  await expect(page.locator('.projects-location')).toContainText('direct-laptop · Direct');
  await page.getByLabel('Link existing session', { exact: true }).selectOption(shell.id);
  await page.getByRole('button', { name: 'Link', exact: true }).click();
  expect(actions.find(a => a.operation === 'projects' && a.command.operation === 'bind')).toEqual({ operation: 'projects', node: 'direct-laptop', command: { operation: 'bind', replica_id: replicaId, terminal_id: shell.id } });
  await page.locator('.projects-sessions').getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.locator('#floating-terminal .terminal-pane-head')).toContainText('Connected');
  expect(actions.find(a => a.session_id === shell.id)).toMatchObject({ node: 'direct-laptop', direct: true });
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await page.getByRole('button', { name: 'Add working copy', exact: true }).click();
  await page.getByLabel('Device', { exact: true }).selectOption('direct:direct-laptop');
  await page.getByLabel('Absolute directory on device', { exact: true }).fill('/home/me/another-copy');
  await page.getByRole('button', { name: 'Save directory', exact: true }).click();
  await expect(page.locator('.projects-list')).toContainText('3 working copies');
  expect(actions.find(a => a.operation === 'projects' && a.command.operation === 'register')).toEqual({ operation: 'projects', node: 'direct-laptop', command: { operation: 'register', node: 'direct-laptop', project_id: projectId, name: 'Shared project', path: '/home/me/another-copy' } });
  revoked = true;
  await page.getByRole('button', { name: 'Refresh projects', exact: true }).click();
  await expect(page.locator('.projects-list')).toContainText('1 working copies');
  await expect(page.locator('.projects-location')).toContainText('/workspace/shared');
  expect(await page.getByLabel('Working copy', { exact: true }).textContent()).not.toContain('/home/me/');
  revoked = false;
  await page.getByRole('button', { name: 'Refresh projects', exact: true }).click();
  await expect(page.locator('.projects-list')).toContainText('3 working copies');
  await page.getByLabel('Working copy', { exact: true }).selectOption('direct:direct-laptop/' + replicaId);
  await expect(page.locator('.projects-sessions')).toContainText(shell.name);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: `/tmp/cranibox-federated-${width}-${theme}.png`, fullPage: true, animations: 'disabled' });
    }
  }
  expect(actions.some(a => a.command && ['create', 'end'].includes(a.command.operation))).toBe(false);
  expect(errors).toEqual([]);
});

test('catalog-only direct access works with the gateway down and does not grant Shell access', async ({ page }) => {
  const actions: any[] = [];
  const remote = { owner: 'direct-laptop', revision: 1, permissions: { projects: true, terminals: false }, projects: [{ id: projectId, name: 'Independent project', replicas: [{ id: replicaId, node: 'direct-laptop', path: '/private/project', sessions: [] }] }] };
  await page.route('**/api/projects', route => route.fulfill({ status: 503, json: { error: 'Workspace gateway unavailable' } }));
  await page.route('**/api/peers', async route => {
    const action = route.request().postDataJSON(); actions.push(action);
    if (action.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'direct-laptop' }] } });
    if (action.command.operation === 'forget') { remote.projects = []; remote.revision++; }
    await route.fulfill({ json: remote });
  });
  await page.goto('/#/projects');
  await expect(page.getByRole('heading', { name: 'Independent project', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Create shell here', exact: true })).toBeDisabled();
  await expect(page.getByRole('combobox', { name: 'Link existing session', exact: true })).toBeDisabled();
  await expect(page.locator('.projects-sources')).toContainText('1/2 available');
  await page.getByRole('button', { name: 'Register project', exact: true }).click();
  await expect(page.getByLabel('Device', { exact: true })).toHaveValue('direct:direct-laptop');
  expect(await page.getByLabel('Device', { exact: true }).locator('option:enabled').allTextContents()).toEqual(['direct-laptop · Direct']);
  await page.getByRole('button', { name: 'Cancel registration', exact: true }).click();
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Remove directory record', exact: true }).click();
  await expect(page.locator('.projects-list button')).toHaveCount(0);
  expect(actions.find(a => a.command?.operation === 'forget')).toEqual({ operation: 'projects', node: 'direct-laptop', command: { operation: 'forget', replica_id: replicaId } });
  expect(actions.some(a => a.operation === 'sessions')).toBe(false);
});

test('offline project snapshots stay visible and read-only until the owner reconnects', async ({ page }) => {
  const actions: any[] = [], errors: string[] = [];
  let offline = true, revoked = false;
  const synced = 1792180800000;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/projects', route => route.fulfill({ status: 503, json: { error: 'Workspace gateway unavailable' } }));
  await page.route('**/api/peers', async route => {
    const action = route.request().postDataJSON(); actions.push(action);
    if (action.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'direct-laptop' }] } });
    if (revoked) return route.fulfill({ status: 400, json: { error: 'API 401: peer credential rejected' } });
    if (action.operation === 'sessions') return route.fulfill({ json: { installed: true, sessions: [shell] } });
    await route.fulfill({ json: { owner: 'direct-laptop', revision: offline ? 1 : 2,
      permissions: { projects: true, terminals: !offline },
      ...(offline ? { sync: { state: 'offline', synced_at: synced } } : {}),
      projects: [{ id: projectId, name: 'Saved offline project', replicas: [{ id: replicaId,
        node: 'direct-laptop', path: offline ? '/private/saved/cranibox' : shell.cwd,
        sessions: [{ id: 'binding', terminal_id: shell.id }] }] }],
    } });
  });
  await page.goto('/#/projects');
  await expect(page.getByRole('heading', { name: 'Saved offline project', exact: true })).toBeVisible();
  await expect(page.locator('.projects-sources')).toContainText('1 offline snapshots');
  await expect(page.locator('.projects-snapshot')).toContainText('Last synced:');
  await expect(page.locator('.projects-snapshot time')).toHaveAttribute('datetime', new Date(synced).toISOString());
  await expect(page.locator('.projects-sessions')).toContainText('Waiting for device status');
  for (const name of ['Open', 'Create shell here', 'Remove directory record', 'Register project', 'Add working copy']) {
    await expect(page.getByRole('button', { name, exact: true })).toBeDisabled();
  }
  await expect(page.getByRole('button', { name: 'Unlink ' + shell.id, exact: true })).toBeDisabled();
  expect(actions.some(a => a.operation === 'sessions')).toBe(false);
  await page.reload();
  await expect(page.locator('.projects-location')).toContainText('/private/saved/cranibox');
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('/private/saved/cranibox');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: '/tmp/cranibox-projects-offline-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
    }
  }
  offline = false;
  await page.getByRole('button', { name: 'Refresh projects', exact: true }).click();
  await expect(page.locator('.projects-snapshot')).toHaveCount(0);
  await expect(page.locator('.projects-location')).toContainText(shell.cwd);
  await expect(page.locator('.projects-sessions')).toContainText(shell.name);
  await expect(page.getByRole('button', { name: 'Open', exact: true })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Remove directory record', exact: true })).toBeEnabled();
  revoked = true;
  await page.getByRole('button', { name: 'Refresh projects', exact: true }).click();
  await expect(page.locator('.projects-list button')).toHaveCount(0);
  expect(await page.locator('.projects-view').textContent()).not.toContain(shell.cwd);
  expect(actions.filter(a => a.operation === 'projects').every(a => a.command.operation === 'list')).toBe(true);
  expect(errors).toEqual([]);
});
