import { test, expect, type Page } from '@playwright/test';

async function ready(page: Page, path: string) {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.goto('/#/app/' + path);
  await expect(page.locator('.pa-main h1')).toBeVisible();
}

test('personal onboarding can finish locally and stays completed after reload', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('dialog')).toContainText('A little space for big ideas');
  await page.getByRole('button', { name: 'Use on this device first', exact: true }).click();
  await page.getByRole('button', { name: 'Add later, go home', exact: true }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.reload();
  await expect(page.locator('.pa-main h1')).toHaveText('What shall we do today?');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByTitle('Chat with DeepSeek Harness')).toBeVisible();
});

test('home task drawer shows current progress and unanswered questions', async ({ page }) => {
  await page.route('**/api/v1/app/tasks', route => route.fulfill({ json: [{
    id: 'task-test', title: 'Prepare the itinerary', direction: 'incoming', state: 'running',
    progress: 'Comparing train times', result: null, updated_at: 1,
    questions: [{ id: 'question-test', text: 'Which day shall we leave?', answer: null }],
  }] }));
  await ready(page, 'home');
  await page.getByRole('button', { name: 'Tasks', exact: true }).click();
  const drawer = page.getByRole('dialog');
  await expect(drawer).toContainText('Prepare the itinerary');
  await expect(drawer).toContainText('Running');
  await expect(drawer).toContainText('Comparing train times');
  await expect(drawer).toContainText('Which day shall we leave?');
  await drawer.getByRole('button', { name: 'View all tasks', exact: true }).click();
  await expect(page).toHaveURL(/#\/tasks$/);
  await expect(page.getByRole('region', { name: 'Collaboration tasks', exact: true })).toBeVisible();
});

test('device pairing uses the C6 invitation and SAS confirmation contract', async ({ page }) => {
  await ready(page, 'devices');
  await page.getByRole('button', { name: 'Add device', exact: true }).click();
  await page.getByRole('button', { name: 'Create QR code and link', exact: true }).click();
  const link = await page.getByLabel('Invitation link', { exact: true }).inputValue();
  expect(link.startsWith('cranibox://pair/')).toBe(true);
  await page.getByLabel('Pairing link', { exact: true }).fill(link);
  await page.getByRole('button', { name: 'Continue to verification', exact: true }).click();
  await expect(page.locator('.pa-sas code')).toHaveText(/[0-9]{4}-[0-9]{4}-[0-9]{4}/);
  const confirmed = page.waitForRequest(request => request.url().includes('/confirm') && request.method() === 'POST');
  await page.getByRole('button', { name: 'Codes match', exact: true }).click();
  expect((await confirmed).postDataJSON()).toEqual({ match: true });
  await expect(page.getByRole('status')).toContainText('Device paired');
});

test('subscription setup sends regional key and enables one chosen node', async ({ page }) => {
  await ready(page, 'models');
  await page.getByRole('button', { name: 'Add subscription', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'API key', exact: true }).click();
  await dialog.getByRole('combobox', { name: 'Service region', exact: true }).selectOption('cn');
  await dialog.getByLabel('Name (optional)', { exact: true }).fill('Personal test account');
  await dialog.getByLabel('API key', { exact: true }).fill('fake-personal-test-key');
  const saved = page.waitForRequest(request => request.url().endsWith('/models/accounts') && request.method() === 'POST');
  await dialog.getByRole('button', { name: 'Save subscription', exact: true }).click();
  expect((await saved).postDataJSON()).toMatchObject({ kind: 'api_key', preset: 'deepseek', region: 'cn', key: 'fake-personal-test-key' });
  await expect(page.getByRole('heading', { name: 'Personal test account', exact: true })).toBeVisible();
  const account = page.locator('.pa-card').filter({ has: page.getByRole('heading', { name: 'DeepSeek', exact: true }) });
  await account.getByText('Choose devices individually', { exact: true }).click();
  const enable = page.waitForRequest(request => request.url().includes('/enable') && request.method() === 'POST');
  await account.getByRole('checkbox', { name: 'Studio', exact: true }).click();
  expect((await enable).postDataJSON()).toMatchObject({ on: true, all_nodes: null });
  await expect(account.getByRole('checkbox', { name: 'Studio', exact: true })).toBeChecked();
  await expect(page.locator('body')).not.toContainText('fake-personal-test-key');
});

test('all six agents can select an available model with DSH recommended', async ({ page }) => {
  await ready(page, 'models');
  await expect(page.locator('.pa-agent-row')).toHaveCount(6);
  await expect(page.locator('.pa-agent-row').first()).toContainText('DeepSeek Harness');
  await expect(page.locator('.pa-agent-row').first()).toContainText('Recommended · Default');
  const saved = page.waitForRequest(request => request.url().endsWith('/agents/pi') && request.method() === 'PUT');
  await page.getByRole('combobox', { name: 'Pi model', exact: true }).selectOption('deepseek/deepseek-chat');
  expect((await saved).postDataJSON()).toEqual({ managed: true, model: 'deepseek/deepseek-chat' });
  await expect(page.getByRole('combobox', { name: 'Pi model', exact: true })).toHaveValue('deepseek/deepseek-chat');
});

test('approval cards expose the request and remove it only after a decision', async ({ page }) => {
  await ready(page, 'inbox');
  const card = page.locator('.pa-approval').filter({ hasText: 'Agent requests a tool' });
  await expect(card).toContainText('rm -rf build/');
  const decision = page.waitForRequest(request => request.url().endsWith('/approvals/apr-2') && request.method() === 'POST');
  await card.getByRole('button', { name: 'Deny', exact: true }).click();
  expect((await decision).postDataJSON()).toEqual({ decision: 'deny' });
  await expect(card).toHaveCount(0);
  await page.getByRole('button', { name: 'Connected', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
});

test('device view distinguishes hub Claude guest and forbids guest model grants', async ({ page }) => {
  await ready(page, 'devices');
  const hub = page.locator('.pa-card').filter({ has: page.getByRole('heading', { name: 'Home server', exact: true }) });
  await expect(hub).toContainText('Hub');
  await expect(hub).toContainText('Via another device');
  const guest = page.locator('.pa-card').filter({ has: page.getByRole('heading', { name: "Alex's laptop", exact: true }) });
  await expect(guest.getByRole('checkbox', { name: 'messages', exact: true })).toBeChecked();
  await expect(guest.getByRole('checkbox', { name: 'llm', exact: true })).toHaveCount(0);
  await expect(guest.getByRole('checkbox', { name: 'vault', exact: true })).toHaveCount(0);
  await expect(page.locator('.pa-card').filter({ hasText: 'Studio' })).toContainText('Offline');
});

test('settings keep connection setup out of the personal UI and retain advanced routes', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await ready(page, 'settings');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(page.getByRole('textbox', { name: /connection|token|server address/i })).toHaveCount(0);
  await page.getByRole('button', { name: 'Dark', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByLabel('Language', { exact: true }).selectOption('zh-cn');
  await expect(page.locator('.pa-main h1')).toHaveText('按你的习惯来');
  await page.getByRole('button', { name: '打开高级功能', exact: true }).click();
  await expect(page).toHaveURL(/overview$/);
  await expect(page.locator('.pa-advanced-bar')).toBeVisible();
  await expect(page.locator('main h1')).toBeVisible();
});

test('advanced bookmark opens the existing overview', async ({ page }) => {
  await page.goto('/#/app/advanced');
  await expect(page).toHaveURL(/#\/overview$/);
  await expect(page.locator('.pa-advanced-bar')).toBeVisible();
  await expect(page.locator('main h1')).toBeVisible();
});
