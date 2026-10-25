import { expect, test, type Page, type WebSocketRoute } from '@playwright/test';

const remote = 'node-1a43abcdef0123456789abcdef012345';
const sleeping = 'node-87654321abcdef0123456789abcdef01';
type Shell = { id: string; name: string; cwd: string; status: 'running' | 'ended' | 'unknown' };
const pane = (page: Page) => page.locator('.terminal-panel.active');
const toolbar = (page: Page) => page.locator('.terminal-toolbar');

async function mockTerminal(page: Page) {
  const state = {
    devices: [
      { node_id: 'host', name: 'Desk PC', badges: ['this'], online: true },
      { node_id: remote, name: 'Design Mac', badges: [], online: true },
      { node_id: sleeping, name: 'Travel laptop', badges: [], online: false },
    ],
    sessions: new Map<string, Shell[]>(), operations: [] as Record<string, any>[],
    sockets: [] as WebSocketRoute[], inputs: [] as string[], ticketError: '', socketError: '',
  };
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url());
    const command = route.request().method() === 'POST' ? route.request().postDataJSON() : {};
    if (url.pathname === '/api/peers') return route.fulfill({ json: { routes: [{ node: remote }], grants: [] } });
    if (url.pathname === '/api/terminal/sessions') {
      state.operations.push(command);
      const node = command.node || '';
      const sessions = state.sessions.get(node) || [];
      if (command.operation === 'create') {
        const session: Shell = { id: 'shell-' + state.operations.length, name: command.name, cwd: command.cwd || '/home/me', status: 'running' };
        state.sessions.set(node, [...sessions, session]);
        return route.fulfill({ json: { session } });
      }
      if (command.operation === 'rename') sessions.find(session => session.id === command.id)!.name = command.name;
      if (command.operation === 'end') state.sessions.set(node, sessions.filter(session => session.id !== command.id));
      return route.fulfill({ json: { installed: true, sessions } });
    }
    if (url.pathname === '/api/terminal/ticket') {
      if (state.ticketError) return route.fulfill({ status: 404, json: { error: state.ticketError } });
      return route.fulfill({ json: { ticket: 'mock' } });
    }
    if (url.pathname.endsWith('/events')) return route.fulfill({ contentType: 'text/event-stream', body: '' });
    if (url.pathname.endsWith('/devices')) return route.fulfill({ json: state.devices.map(device => ({ ...device, kind: 'linux', path: 'direct', guest: false, caps: { terminal: true } })) });
    if (url.pathname.endsWith('/status')) return route.fulfill({ json: { state: 'connected', approvals_pending: 0 } });
    return route.fulfill({ json: [] });
  });
  await page.routeWebSocket('**/api/terminal/ws?ticket=mock', socket => {
    state.sockets.push(socket);
    if (state.socketError) { socket.send(JSON.stringify({ type: 'error', message: state.socketError })); return; }
    socket.send(JSON.stringify({ type: 'ready' }));
    socket.onMessage(message => {
      const value = JSON.parse(String(message));
      if (value.type === 'input') state.inputs.push(value.data);
    });
  });
  return state;
}

async function create(page: Page, label = 'New session') {
  await toolbar(page).getByRole('button', { name: label, exact: true }).click();
}
async function options(page: Page) {
  const trigger = toolbar(page).getByRole('button', { name: 'New session options', exact: true });
  await trigger.focus();
  await trigger.press('ArrowDown');
  await expect(page.getByRole('menuitem', { name: 'New with options…', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
}

for (const language of ['en', 'zh-cn']) for (const width of [375, 1280]) {
  test('owner aliases and one-click creation at ' + width + 'px in ' + language, async ({ page }) => {
    const state = await mockTerminal(page);
    await page.setViewportSize({ width, height: 740 });
    await page.goto('/?window=terminal&lang=' + language);
    const zh = language === 'zh-cn';
    const picker = page.getByRole('combobox', { name: zh ? '目标' : 'Target', exact: true });
    await expect(picker.locator('option:checked')).toContainText('Desk PC');
    await expect(picker.locator('option[value="direct:' + sleeping + '"]')).toHaveAttribute('disabled', '');
    await picker.selectOption('direct:' + remote);
    await expect(picker.locator('option:checked')).toHaveText('Design Mac · ' + (zh ? '在线' : 'Online'));
    await expect(picker).not.toContainText(remote);
    await create(page, zh ? '新建会话' : 'New session');
    await expect(page.getByRole('tab', { name: 'Design Mac 1', exact: true })).toBeVisible();
    await expect(page.locator('dialog[open]')).toHaveCount(0);
    await expect(pane(page).locator('.status')).toContainText(zh ? '已连接' : 'Connected');
    await expect(pane(page).locator('.terminal-pane-title')).toContainText('Design Mac');
    await expect(page.getByRole('tab')).toHaveAttribute('title', 'Design Mac · Design Mac 1');
    expect(state.operations.filter(command => command.operation === 'create')).toEqual([{ operation: 'create', name: 'Design Mac 1', cwd: null, node: remote, direct: true }]);
    await create(page, zh ? '新建会话' : 'New session');
    await expect(page.getByRole('tab', { name: 'Design Mac 2', exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(toolbar(page)).toBeInViewport({ ratio: 1 });
  });
}

test('advanced options support cwd, a start command and double-click rename', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.goto('/?window=terminal');
  await options(page);
  const dialog = page.getByRole('dialog', { name: 'New session', exact: true });
  await expect(dialog).toContainText('Desk PC');
  await dialog.getByText('Choose a working directory', { exact: true }).click();
  await dialog.getByRole('textbox', { name: 'Working directory (optional)', exact: true }).fill('/work/project/');
  await dialog.getByRole('textbox', { name: 'Start command (optional)', exact: true }).fill('npm run dev');
  await dialog.getByRole('button', { name: 'Create and open', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'project 1', exact: true })).toBeVisible();
  await expect.poll(() => state.inputs).toEqual(['npm run dev\r']);
  await pane(page).getByRole('button', { name: 'Detach', exact: true }).click();
  await expect(pane(page).getByRole('status')).toHaveText('Running in background. Reattach to continue.Reattach');
  await pane(page).getByRole('button', { name: 'Reattach', exact: true }).first().click();
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  expect(state.inputs).toEqual(['npm run dev\r']);
  await page.getByRole('tab', { name: 'project 1', exact: true }).dblclick();
  const rename = page.getByRole('dialog', { name: 'Rename session', exact: true });
  await expect(rename).toContainText('Desk PC');
  await rename.getByRole('textbox', { name: 'Session name', exact: true }).fill('Build');
  await rename.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'Build', exact: true })).toBeVisible();
});

test('refresh retains missing tabs and recreates on the same device and cwd at the tab limit', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.goto('/?window=terminal');
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('direct:' + remote);
  for (let index = 0; index < 4; index++) await create(page);
  await expect(page.getByRole('tab')).toHaveCount(4);
  state.sessions.set(remote, []);
  await page.getByRole('button', { name: 'Refresh sessions', exact: true }).click();
  await expect(pane(page).locator('.status')).toHaveText('Session ended');
  await expect(page.getByRole('tab')).toHaveCount(4);
  await expect(pane(page).getByRole('button', { name: 'Reattach', exact: true })).toHaveCount(0);
  await pane(page).getByRole('button', { name: 'New in same directory', exact: true }).click();
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  await expect(page.getByRole('tab')).toHaveCount(4);
  expect(state.operations.filter(command => command.operation === 'create').at(-1)).toMatchObject({ node: remote, direct: true, cwd: '/home/me', name: 'me 1' });
  await page.getByRole('tab', { name: 'Design Mac 1', exact: true }).click();
  await pane(page).getByRole('button', { name: 'Close tab', exact: true }).click();
  await expect(page.getByRole('tab')).toHaveCount(3);
});

for (const failure of ['ticket', 'socket']) {
  test('attach ' + failure + ' failure shows server text and stops reattaching an ended session', async ({ page }) => {
    const state = await mockTerminal(page);
    await page.goto('/?window=terminal');
    if (failure === 'ticket') state.ticketError = 'unknown session: removed by owner';
    else state.socketError = 'session ended; create a new session';
    await create(page);
    await expect(pane(page).getByRole('alert')).toContainText(failure === 'ticket' ? state.ticketError : state.socketError);
    await expect(pane(page).locator('.status')).toHaveText('Session ended');
    await expect(pane(page).getByRole('button', { name: 'New in same directory', exact: true })).toBeEnabled();
    const sockets = state.sockets.length;
    await page.waitForTimeout(1200);
    expect(state.sockets).toHaveLength(sockets);
  });
}

test('refresh and reload preserve ended tabs, cwd and aliases of saved node IDs', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.addInitScript(({ remote }) => {
    if (sessionStorage.getItem('terminal-test-seeded')) return;
    sessionStorage.setItem('terminal-test-seeded', 'true');
    sessionStorage.setItem('cranibox.terminal.tabs', JSON.stringify([{ node: remote, id: 'gone', name: 'Old build', cwd: '/work/old' }]));
  }, { remote });
  await page.goto('/?window=terminal');
  await expect(page.getByRole('tab', { name: 'Old build', exact: true })).toHaveAttribute('title', 'Design Mac · Old build');
  await expect(pane(page).locator('.status')).toHaveText('Session ended');
  await page.reload();
  await expect(page.getByRole('tab', { name: 'Old build', exact: true })).toBeVisible();
  await expect(pane(page).locator('.status')).toHaveText('Session ended');
  await pane(page).getByRole('button', { name: 'New in same directory', exact: true }).click();
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  expect(state.operations.filter(command => command.operation === 'create').at(-1)).toMatchObject({ node: remote, cwd: '/work/old' });
});

test('an attachment exit checks the catalog and unknown session lists keep running tabs', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.goto('/?window=terminal');
  await create(page);
  state.sessions.get('')![0].status = 'unknown';
  await page.getByRole('button', { name: 'Refresh sessions', exact: true }).click();
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  state.sockets.at(-1)!.send(JSON.stringify({ type: 'attachment_closed' }));
  state.sockets.at(-1)!.close();
  await expect(pane(page).locator('.status')).toHaveText('Running in background');
  await expect(pane(page).getByRole('button', { name: 'New in same directory', exact: true })).toHaveCount(0);
  await pane(page).getByRole('button', { name: 'Reattach', exact: true }).first().click();
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  state.sessions.get('')![0].status = 'ended';
  state.sockets.at(-1)!.send(JSON.stringify({ type: 'attachment_closed' }));
  await expect(pane(page).locator('.status')).toHaveText('Session ended');
  await expect(pane(page).getByRole('button', { name: 'New in same directory', exact: true })).toBeEnabled();
});

test('refresh checks open tabs on other devices', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.goto('/?window=terminal');
  await create(page);
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('direct:' + remote);
  await create(page);
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  state.sessions.set('', []);
  await page.getByRole('button', { name: 'Refresh sessions', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Desk PC 1', exact: true, includeHidden: true }).locator('.status')).toHaveText('Session ended');
  await expect(pane(page).locator('.status')).toHaveText('Connected');
});

test('offline devices explain recovery and reconnect automatically when online', async ({ page }) => {
  const state = await mockTerminal(page);
  await page.clock.install();
  await page.goto('/?window=terminal');
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('direct:' + remote);
  await create(page);
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  state.devices[1].online = false;
  await page.clock.fastForward(10050);
  await expect(pane(page).getByRole('status')).toHaveText('Device offline. Reattaching automatically when it returns.');
  await expect(toolbar(page).getByRole('button', { name: 'New session', exact: true })).toBeDisabled();
  state.devices[1].online = true;
  await page.clock.fastForward(10050);
  await expect(pane(page).locator('.status')).toHaveText('Connected');
  expect(state.sockets).toHaveLength(2);
});

test('unnamed devices use a short node ID', async ({ page }) => {
  const state = await mockTerminal(page);
  state.devices[1].name = remote;
  await page.goto('/?window=terminal');
  await expect(page.getByRole('combobox', { name: 'Target', exact: true }).locator('option[value="direct:' + remote + '"]')).toHaveText('node-1a43abcd… · Online');
});
