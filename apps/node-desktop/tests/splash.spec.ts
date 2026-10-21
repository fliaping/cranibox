import { expect, test, type Page } from '@playwright/test';

async function mockSnapshot(page: Page, snapshot: object) {
  await page.addInitScript((value) => {
    (window as any).calls = [];
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        (window as any).calls.push(command);
        if (command === 'snapshot') return value;
        return null;
      },
      transformCallback: () => 0,
    };
  }, snapshot);
}

test('shows progress while the device starts', async ({ page }) => {
  await mockSnapshot(page, { ready: false, error: null });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Cranibox' })).toBeVisible();
  await expect(page.locator('#status')).toContainText('Starting this device');
  await expect(page.getByRole('alert')).toBeHidden();
});

test('a startup failure is explained and can be retried or quit', async ({ page }) => {
  await mockSnapshot(page, { ready: false, error: 'a node service is already running with this credential' });
  await page.goto('/');
  await expect(page.getByRole('alert')).toContainText('already running');
  await page.getByRole('button', { name: /Quit/ }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.includes('quit'))).toBe(true);
});
