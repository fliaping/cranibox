import { test, expect, type Page } from '@playwright/test';

const pages = ['overview', 'nodes', 'terminal', 'history', 'performance', 'agents', 'browser', 'resources', 'services', 'desktop', 'network', 'diagnostics'];
for (const width of [375, 768, 1024, 1440]) {
  test('all pages render without overflow at ' + width + 'px', async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    for (const id of pages) {
      await page.goto('/#/' + id);
      await expect(page.locator('main h1')).toBeVisible();
      await expect(page.locator('.sidebar-revision')).toContainText('ui-preview');
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
    expect(errors).toEqual([]);
  });
}

const activePane = (page: Page) => page.locator('.terminal-panel.active');
async function createTerminal(page: Page, name: string) {
  await page.locator('.terminal-tabs-bar').getByRole('button', { name: 'New session', exact: true }).click();
  await page.locator('dialog').getByRole('textbox', { name: 'Session name', exact: true }).fill(name);
  await page.locator('dialog').getByRole('button', { name: 'Create and open', exact: true }).click();
  await expect(page.locator('dialog')).toHaveCount(0);
  await expect(page.getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(activePane(page).locator('.status')).toContainText('Connected');
}
async function sendTerminal(page: Page, command: string) {
  await activePane(page).locator('.terminal-screen').click();
  await page.keyboard.type(command);
  await page.keyboard.press('Enter');
}
async function endTerminal(page: Page, name: string) {
  await page.getByRole('tab', { name, exact: true }).click();
  await activePane(page).getByRole('button', { name: 'Terminal actions ' + name, exact: true }).click();
  await page.getByRole('menuitem', { name: 'End session…', exact: true }).click();
  await page.locator('dialog').getByRole('button', { name: 'End session', exact: true }).click();
  await expect(page.getByRole('tab', { name, exact: true })).toHaveCount(0);
}

test('persistent terminal opens automatically, detaches, reattaches, renames and ends on both nodes', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const sockets: import('@playwright/test').WebSocket[] = [];
  page.on('websocket', socket => sockets.push(socket));
  await page.goto('/#/terminal');
  await expect(page.getByRole('combobox', { name: 'Target', exact: true }).locator('option')).toHaveCount(2);
  for (const node of ['', 'preview-node']) {
    await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption(node);
    await createTerminal(page, 'Browser task');
    await expect(page.getByRole('combobox', { name: 'Target', exact: true })).toBeEnabled();
    await sendTerminal(page, "printf '%s%s\\n' BROWSER _PTY_OK");
    await expect(activePane(page).locator('.xterm-accessibility')).toContainText('BROWSER_PTY_OK');
    await activePane(page).getByRole('button', { name: 'Detach', exact: true }).click();
    await expect(activePane(page).locator('.status')).toContainText('Detached');
    await activePane(page).getByRole('button', { name: 'Reattach', exact: true }).first().click();
    await expect(activePane(page).locator('.status')).toContainText('Connected');
    await page.reload();
    await expect(activePane(page).locator('.status')).toContainText('Connected');
    await sendTerminal(page, "printf '%s%s\\n' REATTACH _OK");
    await expect(activePane(page).locator('.xterm-accessibility')).toContainText('REATTACH_OK');
    await page.getByRole('button', { name: 'Session actions Browser task', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Rename Browser task', exact: true }).click();
    await page.locator('dialog').getByRole('textbox').fill('Renamed task');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('tab', { name: 'Renamed task', exact: true })).toBeVisible();
    await endTerminal(page, 'Renamed task');
  }
  await createTerminal(page, 'Navigation task');
  const connection = sockets.at(-1)!;
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await page.getByRole('link', { name: 'Overview', exact: true }).click();
  await expect(page.locator('.xterm')).toHaveCount(1);
  expect(connection.isClosed()).toBe(false);
  await page.getByRole('button', { name: 'Terminal', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await endTerminal(page, 'Navigation task');
  expect(errors).toEqual([]);
});

test('remote terminal reconnects automatically after transport loss and respects manual detach', async ({ page }) => {
  await page.addInitScript(() => {
    const sockets: WebSocket[] = [];
    (window as unknown as { terminalTestSockets: WebSocket[] }).terminalTestSockets = sockets;
    const NativeSocket = window.WebSocket;
    window.WebSocket = class extends NativeSocket {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        if (String(url).includes('/api/terminal/ws')) sockets.push(this);
      }
    };
  });
  await page.goto('/#/terminal');
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('preview-node');
  await createTerminal(page, 'Network recovery');
  await sendTerminal(page, "export CRANIBOX_BROWSER=NATIVE; printf '%s%s\\n' BEFORE _DISCONNECT");
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('BEFORE_DISCONNECT');
  await page.evaluate(() => (window as unknown as { terminalTestSockets: WebSocket[] }).terminalTestSockets.at(-1)!.close());
  await expect.poll(() => page.evaluate(() => (window as unknown as { terminalTestSockets: WebSocket[] }).terminalTestSockets.length)).toBe(2);
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await sendTerminal(page, "printf '%s%s\\n' \"$CRANIBOX_BROWSER\" _AFTER_RECONNECT");
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('NATIVE_AFTER_RECONNECT');
  await activePane(page).getByRole('button', { name: 'Detach', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Detached');
  // Allow the initial retry interval to pass: intentional detach must stay detached.
  await page.waitForTimeout(1500);
  expect(await page.evaluate(() => (window as unknown as { terminalTestSockets: WebSocket[] }).terminalTestSockets.length)).toBe(2);
  await endTerminal(page, 'Network recovery');
});

test('floating terminal moves, resizes and keeps a running command connected while minimized', async ({ page }) => {
  const sockets: import('@playwright/test').WebSocket[] = [], operations: string[] = [];
  page.on('websocket', socket => sockets.push(socket));
  page.on('request', request => {
    if (request.url().endsWith('/api/terminal/sessions') && request.method() === 'POST') operations.push(request.postDataJSON()?.operation);
  });
  await page.goto('/#/services');
  await page.getByRole('button', { name: 'Terminal', exact: true }).click();
  await expect(page).toHaveURL(/#\/services$/);
  await createTerminal(page, 'Floating task');
  const floating = page.locator('#floating-terminal');
  await expect(floating).toHaveAttribute('aria-modal', 'false');
  const before = (await floating.boundingBox())!;
  const drag = page.getByRole('button', { name: 'Move terminal window, use arrow keys to adjust position', exact: true });
  await drag.focus();
  await page.keyboard.press('ArrowRight');
  await expect.poll(async () => (await floating.boundingBox())!.x).toBeGreaterThan(before.x);
  const handle = (await drag.boundingBox())!;
  await page.mouse.move(handle.x + 120, handle.y + 22);
  await page.mouse.down();
  await page.mouse.move(handle.x + 150, handle.y + 46, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await floating.boundingBox())!.y).toBeGreaterThan(before.y);
  const resize = page.getByRole('button', { name: 'Resize terminal window, use arrow keys to adjust', exact: true });
  await resize.focus();
  await page.keyboard.press('Shift+ArrowLeft');
  await expect.poll(async () => (await floating.boundingBox())!.width).toBeLessThan(before.width);
  const reduced = (await floating.boundingBox())!, corner = (await resize.boundingBox())!;
  await page.mouse.move(corner.x + 16, corner.y + 15);
  await page.mouse.down();
  await page.mouse.move(corner.x - 70, corner.y - 37, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await floating.boundingBox())!.width).toBeLessThan(reduced.width);
  await sendTerminal(page, "sleep 1; printf '%s%s\\n' MINIMIZED _TASK_DONE");
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await expect(floating).toBeHidden();
  await expect(page.getByRole('button', { name: 'Restore terminal window', exact: true })).toBeFocused();
  await page.getByRole('link', { name: 'Agents', exact: true }).click();
  await expect(page.locator('main h1')).toHaveText('Agents');
  await page.getByRole('button', { name: 'Restore terminal window', exact: true }).click();
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('MINIMIZED_TASK_DONE');
  expect(sockets).toHaveLength(1);
  expect(sockets[0].isClosed()).toBe(false);
  await page.getByRole('button', { name: 'Maximize terminal window', exact: true }).click();
  await expect(floating).toHaveClass(/is-maximized/);
  await page.getByRole('button', { name: 'Restore terminal size', exact: true }).click();
  await page.getByRole('button', { name: 'Terminal window layout', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Compact window', exact: true }).click();
  await expect.poll(async () => (await floating.boundingBox())!.width).toBe(700);
  await expect(page.getByRole('button', { name: 'Open session list', exact: true })).toBeVisible();
  const closing = sockets[0].waitForEvent('close');
  await page.getByRole('button', { name: 'Close terminal window', exact: true }).click();
  await closing;
  await expect(floating).toHaveCount(0);
  expect(operations).toContain('create');
  expect(operations).not.toContain('end');
  await page.getByRole('button', { name: 'Open terminal window', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('MINIMIZED_TASK_DONE');
  await endTerminal(page, 'Floating task');
});

test('overview, Agent and remote node actions open terminals in place and restore minimized state after reload', async ({ page }) => {
  const inputs: string[] = [];
  page.on('websocket', socket => socket.on('framesent', event => {
    try { const data = JSON.parse(String(event.payload)); if (data.type === 'input') inputs.push(data.data); } catch { /* Other frames. */ }
  }));
  await page.goto('/#/overview');
  await page.getByRole('button', { name: 'Open terminal', exact: true }).click();
  await expect(page).toHaveURL(/#\/overview$/);
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await endTerminal(page, (await page.getByRole('tab', { selected: true }).textContent())!);
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await page.getByRole('link', { name: 'Agents', exact: true }).click();
  await page.locator('.agent-card').filter({ hasText: 'Codex' }).getByRole('button', { name: 'Launch Agent', exact: true }).click();
  await expect(page).toHaveURL(/#\/agents$/);
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await expect(activePane(page).getByRole('textbox', { name: 'Command input', exact: true })).toHaveValue('codex');
  expect(inputs).toHaveLength(0);
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await page.reload();
  await expect(page.locator('#floating-terminal')).toBeHidden();
  await page.getByRole('button', { name: 'Restore terminal window', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await endTerminal(page, 'Codex');
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await page.getByRole('link', { name: 'Remote nodes', exact: true }).click();
  await page.locator('.list-row').filter({ hasText: 'Preview Linux node' }).getByRole('button', { name: 'Terminal', exact: true }).click();
  await expect(page).toHaveURL(/#\/nodes$/);
  await expect(page.getByRole('combobox', { name: 'Target', exact: true })).toHaveValue('preview-node');
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await endTerminal(page, (await page.getByRole('tab', { selected: true }).textContent())!);
});

test('mobile command composer protects IME input, exposes shortcuts and manages sessions in a drawer', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 720 });
  const inputs: string[] = [];
  page.on('websocket', socket => socket.on('framesent', event => {
    try { const data = JSON.parse(String(event.payload)); if (data.type === 'input') inputs.push(data.data); } catch { /* Other frames. */ }
  }));
  await page.goto('/#/terminal');
  await expect(page.locator('.sidebar')).toBeHidden();
  await createTerminal(page, 'Mobile task');
  const command = activePane(page).getByRole('textbox', { name: 'Command input', exact: true });
  await command.fill("printf '%s%s\\n' 手机 _INPUT_OK");
  await command.dispatchEvent('compositionstart');
  await command.dispatchEvent('keydown', { key: 'Enter', code: 'Enter', isComposing: true, keyCode: 229 });
  expect(inputs).toHaveLength(0);
  await expect(command).not.toHaveValue('');
  await command.dispatchEvent('compositionend');
  await command.press('Enter');
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('手机_INPUT_OK');
  await activePane(page).getByRole('button', { name: 'Arrow up', exact: true }).click();
  await activePane(page).getByRole('button', { name: 'Enter', exact: true }).click();
  expect(inputs).toContain('\x1b[A');
  await page.setViewportSize({ width: 375, height: 420 });
  await expect(command).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole('button', { name: 'Open session list', exact: true }).click();
  const drawer = page.getByRole('dialog', { name: 'Sessions', exact: true });
  await expect(drawer.getByRole('button', { name: 'Open Mobile task', exact: true })).toBeVisible();
  await drawer.getByRole('button', { name: 'Session actions Mobile task', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Rename Mobile task', exact: true }).click();
  await page.getByRole('dialog', { name: 'Rename session', exact: true }).getByRole('textbox').fill('Mobile renamed');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await page.reload();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await endTerminal(page, 'Mobile renamed');
});

test('tabs preserve independent attachments, support side by side and close without ending sessions', async ({ page }) => {
  const sockets: import('@playwright/test').WebSocket[] = [];
  page.on('websocket', socket => sockets.push(socket));
  await page.goto('/#/terminal');
  await createTerminal(page, 'Build');
  await sendTerminal(page, "printf '%s%s\\n' FIRST _PANE");
  await createTerminal(page, 'Logs');
  await sendTerminal(page, "printf '%s%s\\n' SECOND _PANE");
  expect(sockets).toHaveLength(2);
  await page.getByRole('tab', { name: 'Build', exact: true }).click();
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('FIRST_PANE');
  await page.getByRole('button', { name: 'Session actions Logs', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Open beside', exact: true }).click();
  await expect(page.locator('.terminal-panel.beside')).toBeVisible();
  await expect(page.locator('.terminal-panel.beside .xterm-accessibility')).toContainText('SECOND_PANE');
  expect(sockets).toHaveLength(2);
  await page.getByRole('tab', { name: 'Build', exact: true }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Logs', exact: true })).toBeFocused();
  await expect(page.getByRole('tab', { name: 'Logs', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.getByRole('button', { name: 'Exit side by side', exact: true }).click();
  const closing = sockets[1].waitForEvent('close');
  await page.getByRole('button', { name: 'Close tab Logs', exact: true }).click();
  await closing;
  await expect(page.getByRole('button', { name: 'Open Logs', exact: true })).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Logs', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Open Logs', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('SECOND_PANE');
  await endTerminal(page, 'Logs');
  await endTerminal(page, 'Build');
});

test('local and remote tabs keep their connections while switching nodes and navigating pages', async ({ page }) => {
  const sockets: import('@playwright/test').WebSocket[] = [];
  page.on('websocket', socket => sockets.push(socket));
  await page.goto('/#/terminal');
  await createTerminal(page, 'Local build');
  await sendTerminal(page, "printf '%s%s\\n' LOCAL _CONNECTED");
  await page.getByRole('combobox', { name: 'Target', exact: true }).selectOption('preview-node');
  await createTerminal(page, 'Remote logs');
  await sendTerminal(page, "printf '%s%s\\n' REMOTE _CONNECTED");
  await page.getByRole('tab', { name: 'Local build', exact: true }).click();
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('LOCAL_CONNECTED');
  await page.getByRole('tab', { name: 'Remote logs', exact: true }).click();
  await page.getByRole('button', { name: 'Minimize terminal window', exact: true }).click();
  await page.getByRole('link', { name: 'Remote nodes', exact: true }).click();
  await expect(page.locator('main h1')).toHaveText('Remote nodes');
  await page.getByRole('button', { name: 'Restore terminal window', exact: true }).click();
  await expect(page.getByRole('tab', { name: 'Remote logs', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(activePane(page).locator('.xterm-accessibility')).toContainText('REMOTE_CONNECTED');
  expect(sockets).toHaveLength(2);
  await endTerminal(page, 'Remote logs');
  await endTerminal(page, 'Local build');
});

test('terminal redraw clears old cells when resizing between desktop and mobile', async ({ page }) => {
  const styleErrors: string[] = [];
  page.on('console', message => { if (message.text().includes('Content Security Policy')) styleErrors.push(message.text()); });
  await page.goto('/#/terminal');
  await createTerminal(page, 'Resize task');
  // The alternate screen retains clipped cells for later expansion. Read only
  // glyphs inside each visible row, excluding retained cells and live announcements.
  const visibleText = () => activePane(page).locator('.xterm-rows').evaluate(element => {
    return Array.from(element.children, row => {
      const clip = row.getBoundingClientRect(), walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
      let text = '', node: Node | null;
      while ((node = walker.nextNode())) {
        const value = node.textContent || '';
        for (let offset = 0; offset < value.length; offset++) {
          const range = document.createRange(); range.setStart(node, offset); range.setEnd(node, offset + 1);
          const box = range.getBoundingClientRect();
          if (box.left >= clip.left - 0.5 && box.right <= clip.right + 0.5) text += value[offset];
        }
      }
      return text;
    }).join('\n');
  });
  await sendTerminal(page, "clear; printf '%s%s\\n' BEFORE_ 'RESIZE_0123456789012345678901234567890123456789'");
  await expect.poll(visibleText).toContain('BEFORE_RESIZE_');
  await page.setViewportSize({ width: 390, height: 844 });
  await sendTerminal(page, "clear; printf '%s%s\\n' AFTER _RESIZE");
  await expect.poll(visibleText).toContain('AFTER_RESIZE');
  await expect.poll(visibleText).not.toContain('BEFORE_RESIZE_');
  await expect.poll(visibleText).not.toContain('0123456789');
  await page.setViewportSize({ width: 1440, height: 900 });
  await sendTerminal(page, "clear; printf '%s%s\\n' RESTORED _SIZE");
  await expect.poll(visibleText).toContain('RESTORED_SIZE');
  await expect.poll(visibleText).not.toContain('AFTER_RESIZE');
  expect(styleErrors).toEqual([]);
  const policy = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute('content');
  const scriptPolicy = policy!.split(';').find(value => value.trim().startsWith('script-src'))!;
  expect(scriptPolicy).toContain('nonce-');
  expect(scriptPolicy).not.toContain('unsafe-inline');
  await endTerminal(page, 'Resize task');
});

test('terminal menus stay usable in a short viewport and tab limits preserve background sessions', async ({ page }) => {
  await page.goto('/#/terminal');
  for (const name of ['Limit 1', 'Limit 2', 'Limit 3', 'Limit 4']) await createTerminal(page, name);
  await expect(page.locator('.terminal-tabs-bar').getByRole('button', { name: 'New session', exact: true })).toBeDisabled();
  await page.setViewportSize({ width: 812, height: 375 });
  await page.getByRole('button', { name: 'Terminal actions Limit 4', exact: true }).click();
  await expect(page.getByRole('menu', { name: 'Terminal actions Limit 4', exact: true })).toBeInViewport({ ratio: 1 });
  await page.keyboard.press('End');
  await expect(page.getByRole('menuitem', { name: 'End session…', exact: true })).toBeFocused();
  await expect(page.getByRole('menuitem', { name: 'End session…', exact: true })).toBeInViewport();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Terminal actions Limit 4', exact: true })).toBeFocused();
  await page.getByRole('button', { name: 'Close tab Limit 4', exact: true }).click();
  await expect(page.locator('.terminal-tabs-bar').getByRole('button', { name: 'New session', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Open session list', exact: true }).click();
  await page.locator('dialog').getByRole('button', { name: 'Open Limit 4', exact: true }).click();
  await expect(activePane(page).locator('.status')).toContainText('Connected');
  for (const name of ['Limit 4', 'Limit 3', 'Limit 2', 'Limit 1']) await endTerminal(page, name);
});

test('service search and theme persist through status updates and reload', async ({ page }) => {
  await page.goto('/#/services');
  const search = page.getByRole('searchbox');
  await search.fill('bridge');
  await expect(page.locator('.service-item')).toHaveCount(1);
  await search.focus();
  const refresh = page.waitForResponse(response => response.url().endsWith('/api/poll') && response.request().postDataJSON()?.refresh === true);
  await page.evaluate(() => document.querySelector<HTMLButtonElement>('[aria-label="Refresh status"]')?.click());
  await refresh;
  await expect(page.getByRole('button', { name: 'Refresh status' })).toBeEnabled();
  await expect(search).toBeFocused();
  await expect(search).toHaveValue('bridge');
  await expect(page.locator('.service-item')).toHaveCount(1);
  await page.getByRole('button', { name: 'Switch to light theme' }).click();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(page.locator('main h1')).toHaveText('Services');
  await page.getByRole('button', { name: /^Not running/ }).click();
  await expect(page.locator('.service-item')).toHaveCount(2);
  await page.getByRole('searchbox').fill('no-match');
  await expect(page.getByText('No matching services')).toBeVisible();
  await page.getByRole('button', { name: 'Clear filters' }).click();
  await expect(page.locator('.service-item')).toHaveCount(6);
});

test('mobile navigation traps keyboard focus and restores its trigger', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/overview');
  await expect(page.locator('main h1')).toBeVisible();
  const trigger = page.getByRole('button', { name: 'Open navigation' });
  await trigger.click();
  const navigation = page.getByRole('dialog', { name: 'Workspace navigation' });
  await expect(navigation).toBeVisible();
  await expect(page.locator('.workspace-shell')).toHaveAttribute('inert');
  await navigation.getByRole('link', { name: 'Diagnostics' }).focus();
  await page.keyboard.press('Tab');
  await expect(navigation.getByRole('link', { name: /Cranibox/ })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(trigger).toBeFocused();
  await trigger.click();
  await navigation.getByRole('link', { name: 'Network', exact: true }).click();
  await expect(page.locator('main h1')).toHaveText('Network & access');
  await expect(page.locator('main')).toBeFocused();
});

test('server validation retains input and pending configuration survives reload', async ({ page }) => {
  await page.goto('/#/network');
  await page.getByRole('button', { name: 'Add route', exact: true }).click();
  let dialog = page.locator('dialog');
  await dialog.getByRole('textbox').fill('--help');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(dialog.getByRole('alert')).toBeVisible();
  await expect(dialog.getByRole('textbox')).toHaveValue('--help');
  await expect(dialog.getByRole('textbox')).toBeFocused();
  await dialog.getByRole('textbox').fill('preview');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(dialog).toContainText('Route target');
  await page.reload();
  dialog = page.locator('dialog');
  await expect(dialog).toContainText('Route target');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).not.toBeVisible();
});

test('emergency stop remains usable while another configuration waits for input', async ({ page }) => {
  await page.goto('/#/network');
  await page.getByRole('button', { name: 'Add route', exact: true }).click();
  const dialog = page.locator('dialog');
  await expect(dialog.getByRole('textbox')).toBeVisible();
  const emergency = dialog.getByRole('button', { name: 'Emergency stop desktop control' });
  await expect(emergency).toBeEnabled();
  await emergency.click();
  await expect(dialog.getByRole('textbox')).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.locator('dialog')).toContainText('desktop emergency-stop');
  await page.locator('dialog').getByRole('button', { name: 'Close', exact: true }).click();
});

test('long operations restore logs after reload and can be cancelled', async ({ page }) => {
  await page.goto('/#/services');
  await page.getByRole('button', { name: 'View logs for long-job', exact: true }).click();
  await expect(page.locator('dialog').locator('pre')).toContainText('long job started');
  await page.reload();
  await page.getByRole('button', { name: 'Activity log', exact: true }).click();
  const dialog = page.locator('dialog');
  await expect(dialog.locator('pre')).toContainText('long job started');
  await dialog.getByRole('button', { name: 'Cancel operation' }).click();
  await expect(dialog).toContainText('No operation running');
});

test('poll failure shows stale status without losing the page', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.goto('/#/services');
  await expect(page.locator('.service-item')).toHaveCount(6);
  await page.route('**/api/poll', route => route.abort());
  await page.getByRole('button', { name: 'Refresh status' }).click();
  await expect(page.getByRole('alert')).toContainText('last successful snapshot');
  await expect(page.locator('.service-item')).toHaveCount(6);
});

test('MCP transport and multiple Agent selections reach the server correctly', async ({ page }) => {
  await page.goto('/#/resources');
  await page.getByRole('button', { name: 'Add MCP', exact: true }).click();
  const dialog = page.locator('dialog');
  await dialog.getByRole('textbox').fill('preview');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(dialog.getByRole('radio', { name: /Streamable HTTP/ })).toBeChecked();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await dialog.getByRole('textbox').fill('https://mcp.example.com');
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(dialog.getByRole('checkbox', { name: /Codex/ })).toBeChecked();
  await expect(dialog.getByRole('checkbox', { name: /Claude Code/ })).toBeChecked();
  await dialog.getByRole('checkbox', { name: /Claude Code/ }).uncheck();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.locator('dialog').locator('pre')).toContainText('mcp add preview --transport http --agents codex --url https://mcp.example.com');
});

for (const width of [375, 1440]) {
  test('Chinese pages render without overflow at ' + width + 'px', async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    for (const id of pages) {
      await page.goto('http://127.0.0.1:19082/#/' + id);
      await expect(page.locator('html')).toHaveAttribute('lang', 'zh-cn');
      await expect(page.locator('main h1')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  });
}


test('node credentials appear once and remote tasks reach durable history', async ({ page }) => {
  await page.goto('/#/nodes');
  await expect(page.locator('.list-row strong').filter({ hasText: 'Preview Linux node' })).toBeVisible();
  const preview = page.locator('.list-row').filter({ hasText: 'Preview Linux node' });
  await preview.getByRole('textbox', { name: 'Device group preview-node', exact: true }).fill('personal');
  page.once('dialog', dialog => dialog.accept());
  await preview.getByRole('button', { name: 'Save group', exact: true }).click();
  await expect(preview).toContainText('Group: personal');
  await page.getByLabel('New node ID', { exact: true }).fill('browser-test-node');
  await page.getByRole('button', { name: 'Register node', exact: true }).click();
  const secret = page.getByLabel('Node credential', { exact: true });
  await expect(secret).toHaveAttribute('type', 'password');
  await expect(secret).not.toHaveValue('');
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(secret).toHaveCount(0);
  await page.reload();
  await expect(secret).toHaveCount(0);
  await page.getByRole('combobox', { name: 'Target node', exact: true }).selectOption('preview-node');
  await page.getByLabel('Program', { exact: true }).fill('/bin/echo');
  await page.getByRole('textbox', { name: 'Arguments (JSON array)', exact: true }).fill('["browser remote output"]');
  await page.getByRole('button', { name: 'Run task', exact: true }).click();
  await page.getByRole('link', { name: 'Task submitted. View output' }).click();
  await expect(page.locator('.history-output')).toContainText('browser remote output');
  await expect(page.getByText('Exit code 0', { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.locator('.history-output')).toContainText('browser remote output');
  await page.goto('/#/nodes');
  page.once('dialog', dialog => dialog.accept());
  await page.locator('.list-row').filter({ hasText: 'browser-test-node' }).getByRole('button', { name: 'Revoke', exact: true }).click();
  await expect(page.locator('.list-row').filter({ hasText: 'browser-test-node' })).toHaveCount(0);
});

test('cancelled local operation appears in history and metrics use actual service data', async ({ page }) => {
  await page.goto('/#/services');
  await page.getByRole('button', { name: 'View logs for long-job', exact: true }).click();
  const dialog = page.locator('dialog');
  await expect(dialog.locator('pre')).toContainText('long job started');
  await dialog.getByRole('button', { name: 'Cancel operation' }).click();
  await expect(dialog).toContainText('No operation running');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await page.goto('/#/history');
  const record = page.locator('.list-row').filter({ hasText: 'Cancelled' }).first();
  await expect(record).toBeVisible();
  await record.getByRole('button', { name: 'Details', exact: true }).click();
  await expect(page.locator('.history-output')).toContainText('long job started');
  await page.goto('/#/performance');
  await expect(page.locator('.metric-value').first()).toContainText('MiB');
  await expect(page.getByText('Memory and CPU measure the unified server', { exact: false })).toBeVisible();
});
