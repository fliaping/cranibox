import { expect, test } from '@playwright/test';
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const settings = { server: 'wss://workspace.example.com', id: 'laptop', name: '我的笔记本', token_file: '/private/node.token', allow_insecure: false };
    (window as any).calls = [];
    (window as any).__TAURI_INTERNALS__ = { invoke: async (command: string, payload?: any) => {
      (window as any).calls.push({ command, payload });
      if (command === 'snapshot') return { settings, status: { state: 'connected', message: null }, autostart: true, error: null, quit_requested: false };
      if (command === 'peers') return { nodes: [{ id: 'laptop', name: '我的笔记本', online: true, os: 'macos', mesh_group: 'personal' }, { id: 'desktop', name: '开发主机', online: true, os: 'linux', mesh_group: 'personal' }] };
      if (command === 'mcp_config') return { mcpServers: { 'cranibox-devices': { command: '/apps/cranibox-node-desktop', args: ['mcp', '--config', '/private/settings.json'] } } };
      return null;
    } };
  });
});
test('devices, MCP and reconnect preserve the service', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '我的笔记本' })).toBeVisible();
  await expect(page.getByText('开发主机')).toBeVisible();
  await page.getByRole('button', { name: '重新连接', exact: true }).click();
  await page.getByRole('button', { name: 'Agent 接入', exact: true }).click();
  expect(await page.getByRole('textbox', { name: 'MCP 配置' }).inputValue()).toContain('cranibox-devices');
  expect(await page.getByRole('textbox', { name: 'MCP 配置' }).inputValue()).not.toContain('token');
  await page.getByRole('button', { name: '收起到系统托盘' }).click();
  const calls = await page.evaluate(() => (window as any).calls.map((call: any) => call.command));
  expect(calls).toContain('reconnect'); expect(calls).toContain('hide_window'); expect(calls).not.toContain('quit');
});
test('quitting requires confirmation and cancel keeps the service', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '退出客户端', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).not.toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'quit'))).toBe(false);
  await page.getByRole('button', { name: '退出客户端', exact: true }).click();
  await page.getByRole('button', { name: '退出并结束会话', exact: true }).click();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'quit'))).toBe(true);
});
test('narrow layout and credential saving', async ({ page }) => {
  await page.setViewportSize({ width: 440, height: 780 });
  await page.goto('/');
  await page.getByRole('button', { name: '连接设置', exact: true }).click();
  await page.getByLabel('节点 Token', { exact: true }).fill('a'.repeat(64));
  await page.getByRole('button', { name: '保存并连接', exact: true }).click();
  await expect(page.getByText('设置已保存，后台节点正在连接。')).toBeVisible();
  await page.getByRole('button', { name: '连接设置', exact: true }).click();
  await expect(page.getByLabel('节点 Token', { exact: true })).toHaveValue('');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.setViewportSize({ width: 880, height: 720 });
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await page.screenshot({ path: '/tmp/cranibox-node-desktop-preview.png', fullPage: true });
});
