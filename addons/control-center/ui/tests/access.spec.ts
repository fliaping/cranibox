import { expect, test, type Page } from '@playwright/test';

const remote = 'node-1a43abcdef0123456789abcdef012345';
const caps = ['messages', 'terminals', 'exec', 'projects', 'desktop', 'computer', 'browser', 'update', 'spaces', 'activity', 'llm', 'vault'];
const modes = () => Object.fromEntries(caps.map(cap => [cap, 'ask']));
const request = (id: string, capability = 'terminals') => ({ id, from: remote, name: 'Design Mac', capability, state: 'pending', created_at: Math.floor(Date.now() / 1000), expires_at: Math.floor(Date.now() / 1000) + 120, path: { text: 'UNTRUSTED REQUESTER TEXT' } });
async function mockAccess(page: Page) {
  const state = {
    access: { peers: [{ node: remote, name: 'Design Mac', capabilities: modes() }], pending: [] as ReturnType<typeof request>[], approvals: [], sessions: [] as any[] },
    saved: [] as any[], decisions: [] as any[], revoked: [] as any[], auditQueries: [] as URLSearchParams[],
    tampered: false, approval: 'pending', blocked: '', actions: [] as any[], polls: [] as string[], tickets: [] as any[], terminalBlocked: false,
    updateBlocked: false, updates: [] as string[], shells: [] as any[],
  };
  await page.addInitScript(() => {
    class Events extends EventTarget {
      static instances: Events[] = [];
      constructor() { super(); Events.instances.push(this); }
      close() { Events.instances = Events.instances.filter(item => item !== this); }
    }
    Object.assign(window, { EventSource: Events, accessEvent: () => Events.instances.forEach(item => item.dispatchEvent(new Event('access'))) });
  });
  function denial() {
    return state.blocked === 'off' ? { error: 'capability_off', capability: 'browser', hint: 'Open Design Mac → Access to enable Browser.' } : { error: 'approval_required', capability: 'browser', request_id: 'remote-request', expires_at: Math.floor(Date.now() / 1000) + 120 };
  }
  await page.route('**/api/**', async route => {
    const req = route.request(), url = new URL(req.url()), path = url.pathname;
    const body = ['PUT', 'POST'].includes(req.method()) ? req.postDataJSON() : {};
    if (path === '/api/v1/app/access') return route.fulfill({ json: state.access });
    if (path === '/api/v1/app/access/' + remote) {
      state.saved.push(body); state.access.peers[0].capabilities = body;
      return route.fulfill({ json: {} });
    }
    if (path.startsWith('/api/v1/app/access/requests/')) {
      if (req.method() === 'GET') { state.polls.push(url.searchParams.get('node') || ''); return route.fulfill({ json: { ...request('remote-request'), state: state.approval } }); }
      const id = path.split('/').at(-1); state.decisions.push({ id, ...body });
      state.access.pending = state.access.pending.filter(item => item.id !== id);
      return route.fulfill({ json: {} });
    }
    if (path === '/api/v1/app/access/revoke') { state.revoked.push(body); state.access.sessions = []; return route.fulfill({ json: {} }); }
    if (path === '/api/v1/app/access/audit') {
      state.auditQueries.push(url.searchParams);
      const before = Number(url.searchParams.get('before') || 52);
      return route.fulfill({ json: { verified: !state.tampered, broken_index: state.tampered ? 7 : null, entries: Array.from({ length: before === 52 ? 50 : 1 }, (_, index) => ({ index: before - index - 1, at: 1800000000, event: 'approved', from: remote, name: 'Design Mac', capability: 'exec', result: 'ok', basis: 'approval' })) } });
    }
    if (path.endsWith('/devices')) return route.fulfill({ json: [
      { node_id: 'host', name: 'Desk PC', badges: ['this'], online: true, kind: 'linux', caps: {} },
      { node_id: remote, name: 'Design Mac', badges: [], online: true, kind: 'mac', path: 'direct', guest: false, caps: Object.fromEntries(caps.map(cap => [cap, true])) },
    ] });
    if (path === '/api/v1/app/update') {
      state.updates.push(url.search);
      if (state.updateBlocked && state.approval !== 'approved') return route.fulfill({ status: 403, json: denial() });
      return route.fulfill({ json: { version: '1.1.0', status: null, runtime: { state: 'confirmed', managed: true, can_update: true, reasons: [], os: 'macos', arch: 'arm64' } } });
    }
    if (path.endsWith('/status')) return route.fulfill({ json: { state: 'connected', approvals_pending: 0 } });
    if (path === '/api/peers') {
      if (body.operation !== 'browser') return route.fulfill({ json: { routes: [{ node: remote }], grants: [] } });
      state.actions.push(body);
      if (state.blocked && state.approval !== 'approved') return route.fulfill({ status: 403, json: denial() });
      return route.fulfill({ json: { node: remote, result: { paused: false, backend_available: true } } });
    }
    if (path === '/api/terminal/sessions') {
      if (body.operation === 'create') { const session = { id: 'shell-' + (state.shells.length + 1), name: body.name, cwd: '/', status: 'running' }; state.shells.push(session); return route.fulfill({ json: { session } }); }
      return route.fulfill({ json: { installed: true, sessions: state.shells } });
    }
    if (path === '/api/terminal/ticket') {
      state.tickets.push(body);
      if (state.terminalBlocked && state.approval !== 'approved') return route.fulfill({ status: 403, json: { ...denial(), capability: 'terminals' } });
      return route.fulfill({ json: { ticket: 'mock' } });
    }
    if (path.endsWith('/events')) return route.fulfill({ contentType: 'text/event-stream', body: '' });
    return route.fulfill({ json: [] });
  });
  await page.routeWebSocket('**/api/terminal/ws?ticket=mock', socket => socket.send(JSON.stringify({ type: 'ready' })));
  return state;
}
async function openDevice(page: Page, tab = 'access', lang = 'en') { await page.goto('/?lang=' + lang + '#/app/device/' + remote + '/' + tab); }
async function refresh(page: Page) { await page.evaluate(() => (window as any).accessEvent()); }

for (const lang of ['en', 'zh-cn']) for (const width of [375, 1280]) {
  test('grouped modes, warnings and advanced save in ' + lang + ' at ' + width, async ({ page }) => {
    const state = await mockAccess(page), zh = lang === 'zh-cn';
    await page.setViewportSize({ width, height: 800 });
    await openDevice(page, 'access', lang);
    await expect(page.getByRole('heading', { name: zh ? 'Design Mac 可以在本设备上…' : 'Design Mac can use this device to…' })).toBeVisible();
    const group = page.getByRole('region', { name: zh ? '终端与命令' : 'Terminal & commands', exact: true });
    await group.getByRole('button', { name: zh ? '允许' : 'Allow', exact: true }).first().click();
    await expect(group.getByRole('alert')).toContainText(zh ? '无需询问' : 'without asking');
    await group.getByText(zh ? '高级' : 'Advanced', { exact: true }).click();
    await group.getByRole('group', { name: zh ? '执行命令' : 'Run commands', exact: true }).getByRole('button', { name: zh ? '关闭' : 'Off', exact: true }).click();
    for (const name of [zh ? '远程更新' : 'Remote updates', zh ? '模型与账号' : 'Models & accounts']) {
      const risky = page.getByRole('region', { name, exact: true });
      await risky.getByRole('button', { name: zh ? '允许' : 'Allow', exact: true }).first().click();
      await expect(risky.getByRole('alert')).toContainText(zh ? '仍需逐次批准' : 'each time');
    }
    await page.getByRole('button', { name: zh ? '保存' : 'Save', exact: true }).click();
    await expect(page.getByRole('status').filter({ hasText: zh ? '已保存' : 'Saved' })).toBeVisible();
    expect(state.saved[0]).toMatchObject({ terminals: 'allow', exec: 'off', projects: 'allow', update: 'allow', llm: 'allow', vault: 'allow' });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}
for (const [label, decision, scope] of [['Deny', 'deny', undefined], ['Allow once', 'approve', 'session'], ['Allow 30 minutes', 'approve', '30m'], ['Allow 8 hours', 'approve', '8h']] as const) {
  test('approval queue supports ' + label + ' and cannot be accidentally dismissed', async ({ page }) => {
    const state = await mockAccess(page); state.access.pending = [request('first'), request('second', 'desktop')];
    await openDevice(page);
    const dialog = page.getByRole('dialog');
    await expect(dialog).toHaveCount(1); await expect(dialog).toContainText('Design Mac requests');
    await expect(dialog).toContainText(remote); await expect(dialog.getByRole('timer')).toContainText('seconds left');
    await expect(dialog).not.toContainText('UNTRUSTED');
    await expect(dialog.getByRole('button', { name: 'Deny', exact: true })).toBeFocused();
    await page.keyboard.press('Escape'); await expect(dialog).toBeVisible();
    await page.keyboard.press('Shift+Tab'); await expect(dialog.getByRole('button', { name: 'Allow 8 hours', exact: true })).toBeFocused();
    await dialog.getByRole('button', { name: label, exact: true }).click();
    await expect(dialog).toContainText('Remote desktop');
    expect(state.decisions[0]).toEqual({ id: 'first', decision, ...(scope ? { scope } : {}) });
    await dialog.getByRole('button', { name: 'Deny', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    expect(state.decisions).toHaveLength(2);
  });
}

test('live snapshot opens a Chinese approval dialog on any page', async ({ page }) => {
  const state = await mockAccess(page); await page.goto('/?lang=zh-cn#/app/devices');
  await expect(page.locator('main h1')).toBeVisible();
  state.access.pending = [request('new')]; await refresh(page);
  await expect(page.getByRole('dialog')).toContainText('Design Mac 请求使用');
  await page.getByRole('dialog').getByRole('button', { name: '允许本次', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('approval and session names survive a missing device catalogue entry', async ({ page }) => {
  const state = await mockAccess(page);
  state.access.peers = [];
  state.access.pending = [request('uncatalogued')];
  state.access.sessions = [{ id: 'session', from: remote, name: 'Design Mac', capability: 'desktop' }];
  await page.route('**/api/v1/app/devices', route => route.fulfill({ json: [] }));
  await page.goto('/#/app/access-log');
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('heading')).toHaveText('Design Mac requests Terminal');
  await dialog.getByRole('button', { name: 'Deny', exact: true }).click();
  await expect(page.locator('.cx-access-surfaces')).toContainText('Design Mac is using Remote desktop');
});

test('active sessions stay visible while scrolling and disconnect all', async ({ page }) => {
  const state = await mockAccess(page); state.access.sessions = [{ id: 's1', from: remote, name: 'Design Mac', capability: 'terminals' }, { id: 's2', from: remote, name: 'Design Mac', capability: 'desktop' }];
  await openDevice(page);
  const banner = page.locator('.cx-access-surfaces').getByRole('status').filter({ hasText: 'is using' });
  await expect(banner).toContainText('Design Mac is using Terminal · Remote desktop');
  await page.evaluate(() => scrollTo(0, document.body.scrollHeight)); await expect(banner).toBeInViewport();
  await banner.getByRole('link', { name: 'Access log', exact: true }).click(); await expect(page).toHaveURL(/access-log/);
  await banner.getByRole('button', { name: 'Disconnect all', exact: true }).click();
  await expect(banner).toHaveCount(0); expect(state.revoked).toEqual([{}]);
});

test('audit filters, stable paging and integrity failure', async ({ page }) => {
  const state = await mockAccess(page); await page.goto('/#/app/access-log');
  await expect(page.getByText('Log verified', { exact: true })).toBeVisible();
  await expect(page.locator('.cx-access-log li')).toHaveCount(50);
  await page.getByRole('button', { name: 'Older entries', exact: true }).click();
  await expect(page.locator('.cx-access-log li')).toHaveCount(51);
  expect(state.auditQueries.at(-1)?.get('before')).toBe('2');
  await page.getByRole('combobox', { name: 'Device', exact: true }).selectOption(remote);
  await page.getByRole('combobox', { name: 'Capability', exact: true }).selectOption('exec');
  await expect.poll(() => state.auditQueries.at(-1)?.get('capability')).toBe('exec');
  expect(state.auditQueries.at(-1)?.get('node')).toBe(remote);
  state.tampered = true; await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByText('Log may have been tampered with (entry 7)', { exact: true })).toBeVisible();
});

for (const outcome of ['approved', 'denied', 'expired', 'off']) {
  test('shared requester flow handles ' + outcome, async ({ page }) => {
    const state = await mockAccess(page); state.blocked = outcome === 'off' ? 'off' : 'ask';
    await openDevice(page, 'browser');
    await page.getByRole('button', { name: 'Check target browser', exact: true }).click();
    const status = page.locator('.direct-grant-form .cx-access-banner');
    if (outcome === 'off') {
      await expect(status).toContainText('This capability is off'); await expect(status).toContainText('Open Design Mac → Access');
    } else {
      await expect(status).toContainText('Waiting for approval on Design Mac');
      state.approval = outcome;
      if (outcome === 'approved') {
        await expect(page.getByText('Target browser ready.', { exact: true })).toBeVisible();
        expect(state.actions).toHaveLength(2); expect(state.actions[1]).toEqual(state.actions[0]); expect(state.polls).toEqual([remote]);
        return;
      }
      await expect(status).toContainText(outcome === 'denied' ? 'Denied' : 'Expired');
    }
    state.approval = 'approved';
    await status.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByText('Target browser ready.', { exact: true })).toBeVisible();
    expect(state.actions).toHaveLength(2); expect(state.actions[1]).toEqual(state.actions[0]);
  });
}

test('app API adapter waits and automatically retries the original GET', async ({ page }) => {
  const state = await mockAccess(page); state.updateBlocked = true;
  await openDevice(page, 'overview');
  await expect(page.locator('.cx-access-surfaces')).toContainText('Waiting for approval on Design Mac');
  state.approval = 'approved';
  await expect(page.getByText('1.1.0', { exact: true })).toBeVisible();
  expect(state.updates).toEqual(['?node=' + remote, '?node=' + remote]);
});

test('terminal waits beyond the attachment timeout, retries the same ticket and cancels cleanly', async ({ page }) => {
  const state = await mockAccess(page); state.terminalBlocked = true;
  await page.clock.install(); await page.goto('/?window=terminal');
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('direct:' + remote);
  await page.locator('.terminal-toolbar').getByRole('button', { name: 'New session', exact: true }).click();
  const pane = page.locator('.terminal-panel.active');
  await expect(pane).toContainText('Waiting for approval on Design Mac');
  await page.clock.fastForward(25000);
  await expect(pane).toContainText('Waiting for approval on Design Mac'); expect(state.tickets).toHaveLength(1);
  state.approval = 'approved'; await page.clock.fastForward(2000);
  await expect(pane.locator('.status')).toHaveText('Connected'); expect(state.tickets[1]).toEqual(state.tickets[0]);
  state.approval = 'pending'; await page.locator('.terminal-toolbar').getByRole('button', { name: 'New session', exact: true }).click();
  await expect(pane).toContainText('Waiting for approval on Design Mac');
  await pane.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(pane.locator('.status')).toHaveText('Running in background');
  const count = state.tickets.length; await page.clock.fastForward(20000); expect(state.tickets).toHaveLength(count);
});

for (const width of [375, 1280]) test('Chinese approval, sessions and audit fit at ' + width, async ({ page }) => {
  const state = await mockAccess(page);
  state.access.pending = [request('mobile')];
  state.access.sessions = [{ id: 's1', from: remote, name: 'Design Mac', capability: 'desktop' }];
  await page.setViewportSize({ width, height: 800 });
  await openDevice(page, 'access', 'zh-cn');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Design Mac 请求使用 终端');
  const bounds = await dialog.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
  await dialog.getByRole('button', { name: '拒绝', exact: true }).click();
  const banner = page.locator('.cx-access-surfaces').getByRole('status').filter({ hasText: '正在使用' });
  await expect(banner).toContainText('Design Mac 正在使用 远程桌面');
  await banner.getByRole('link', { name: '访问记录', exact: true }).click();
  await expect(page.getByText('记录完整', { exact: true })).toBeVisible();
  state.tampered = true;
  await page.getByRole('button', { name: '刷新', exact: true }).click();
  await expect(page.getByText('记录可能被篡改（第 7 条）', { exact: true })).toBeVisible();
  await banner.getByRole('button', { name: '全部断开', exact: true }).click();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('Chinese requester waits and retries after approval', async ({ page }) => {
  const state = await mockAccess(page); state.blocked = 'ask';
  await openDevice(page, 'browser', 'zh-cn');
  await page.getByRole('button', { name: '检查目标浏览器', exact: true }).click();
  await expect(page.locator('.direct-grant-form')).toContainText('等待 Design Mac 上的批准…');
  state.approval = 'approved';
  await expect.poll(() => state.actions.length).toBe(2);
  await expect(page.locator('.direct-grant-form .cx-access-banner')).toHaveCount(0);
});

test('standalone terminal keeps the active session bar above its content', async ({ page }) => {
  const state = await mockAccess(page);
  state.access.sessions = [{ id: 's1', from: remote, name: 'Design Mac', capability: 'terminals' }];
  await page.goto('/?window=terminal');
  const banner = page.locator('.terminal-window > .cx-access-surfaces');
  const button = banner.getByRole('button', { name: 'Disconnect all', exact: true });
  await expect(button).toBeInViewport();
  await button.click(); expect(state.revoked).toEqual([{}]);
  await expect(banner).toBeHidden();
});
