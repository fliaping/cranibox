import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';

const fixtures = Object.fromEntries(['status', 'devices', 'agents', 'approvals'].map(name => [name,
  JSON.parse(readFileSync(new URL('../../../crates/cranibox-contracts/fixtures/app_' + name + '.json', import.meta.url), 'utf8')),
]));

test.beforeEach(async ({ page }) => {
  await page.addInitScript(fixtures => {
    const data = structuredClone(fixtures);
    const callbacks = new Map<number, Function>(); let next = 1;
    (window as any).calls = [];
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback(callback: Function) { const id = next++; callbacks.set(id, callback); return id; },
      unregisterCallback(id: number) { callbacks.delete(id); },
      async invoke(command: string, payload?: any) {
        (window as any).calls.push({ command, payload });
        if (command === 'plugin:event|listen') return next++;
        if (command === 'snapshot') return { quit_requested: false };
        if (command === 'app_approval_decide') { data.approvals = data.approvals.filter((a: any) => a.id !== payload.id); return null; }
        if (command === 'app_models') return [];
        if (command === 'app_call' && payload?.path === '/tasks') return [];
        const key = command.replace(/^app_/, '');
        if (key in data) return structuredClone(data[key]);
        return null;
      },
    };
  }, fixtures);
});

test('first native launch opens personal onboarding without connection fields', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('dialog')).toContainText('很高兴，在这里遇见你');
  await expect(page.getByLabel(/服务端地址|节点 Token/)).toHaveCount(0);
  await page.getByRole('button', { name: '先在这台设备使用', exact: true }).click();
  await page.getByRole('button', { name: '稍后添加，进入首页', exact: true }).click();
  await expect(page.locator('.pa-main h1')).toHaveText('今天，一起做些什么？');
  await page.reload();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('native approval cards use the named C6 IPC request shapes', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.goto('/#/app/inbox');
  const card = page.locator('.pa-approval').filter({ hasText: 'rm -rf build/' });
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: '拒绝', exact: true }).click();
  await expect(card).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls).toContainEqual({ command: 'app_approval_decide', payload: { id: 'apr-2', req: { decision: 'deny' } } });
  expect(calls.some((call: any) => call.command === 'app_status')).toBe(true);
  expect(calls.some((call: any) => call.command === 'app_events')).toBe(true);
  expect(calls.some((call: any) => ['configure', 'app_call'].includes(call.command))).toBe(false);
});

test('native task drawer opens the collaboration tasks page', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.goto('/#/app/home');
  await page.getByRole('button', { name: '任务', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: '查看所有任务', exact: true }).click();
  await expect(page).toHaveURL(/#\/tasks$/);
  await expect(page.getByRole('region', { name: '协作任务', exact: true })).toBeVisible();
});
