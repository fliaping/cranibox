import { test, expect, type Page } from '@playwright/test';

async function ready(page: Page, path: string) {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.goto('/#' + path);
  await expect(page.locator('main h1').first()).toBeVisible();
}

test('the device network draws who may use whom and explains a selected device', async ({ page }) => {
  await ready(page, '/app/devices');
  const graph = page.getByRole('region', { name: 'Device network' });
  await expect(graph).toContainText('An arrow A → B means A can use B');
  // Your devices sit in the identity zone; the others around it, each with its relation.
  await expect(graph.getByRole('button', { name: /^MacBook Pro · This device/ })).toBeVisible();
  await expect(graph.getByRole('button', { name: /^Build worker · Controlled device · Controlled by MacBook Pro/ })).toBeVisible();
  await expect(graph.getByRole('button', { name: /^CI runner · Controlled device · Controlled by Home server/ })).toBeVisible();
  await expect(graph.getByRole('button', { name: /^Alex's laptop · Guest · Uses MacBook Pro/ })).toBeVisible();
  await expect(graph.getByRole('button', { name: /^Lin's NAS · Shared with me/ })).toBeVisible();
  await expect(graph.locator('.cxg-edge.control')).toHaveCount(2);
  await expect(graph.locator('.cxg-edge.guest')).toHaveCount(1);
  await expect(graph.locator('.cxg-edge.shared')).toHaveCount(1);
  // Hovering lights the device and its edges and dims the rest.
  await graph.getByRole('button', { name: /^Build worker/ }).hover();
  await expect(graph.locator('svg.has-focus')).toHaveCount(1);
  await expect(graph.locator('.cxg-edge.lit')).toHaveCount(1);
  await expect(graph.locator('.cxg-node.lit')).toHaveCount(2);
  await graph.getByRole('button', { name: /^Build worker/ }).click();
  const panel = graph.locator('.cx-graph-panel');
  await expect(panel).toContainText('MacBook Pro can run tasks on Build worker. Build worker cannot reach any of your devices');
  await expect(panel).toContainText('It can useNothing');
  await panel.getByRole('link', { name: 'Open device' }).click();
  await expect(page).toHaveURL(/#\/app\/device\/node-4e5f/);
  await expect(page.getByRole('main')).toContainText('Controlled device');
  await expect(page.getByRole('main')).toContainText('This device can run tasks on Build worker');
});

test('devices are grouped by relation and the graph can hide its layers', async ({ page }) => {
  await ready(page, '/app/devices');
  for (const name of ['My devices', 'Controlled devices', 'Shared with me', 'Guest devices']) await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Devices that control this one' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Build worker', exact: true })).toContainText('Controlled device');
  const graph = page.getByRole('region', { name: 'Device network' });
  await graph.getByRole('button', { name: 'Who uses whom' }).click();
  await expect(graph.locator('.cxg-edge')).toHaveCount(0);
  await graph.getByRole('button', { name: 'Connections' }).click();
  await expect(graph.locator('.cxg-link')).toHaveCount(0);
  await graph.getByRole('button', { name: 'Who uses whom' }).click();
  await graph.getByRole('button', { name: 'Connections' }).click();
  await expect(graph.locator('.cxg-edge')).toHaveCount(4);
});

test('narrow screens list the relations instead of drawing them', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await ready(page, '/app/devices');
  const graph = page.getByRole('region', { name: 'Device network' });
  await expect(graph.locator('svg.cx-graph')).toBeHidden();
  const list = graph.getByRole('list', { name: 'Who uses whom' });
  await expect(list.getByRole('listitem')).toHaveCount(4);
  await expect(list.getByRole('listitem').first()).toContainText('MacBook Pro');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('pairing asks how the devices relate and sends that direction', async ({ page }) => {
  let sent: unknown = null;
  await page.route('**/api/v1/app/pair/invite', async route => { sent = route.request().postDataJSON(); await route.fallback(); });
  await ready(page, '/app/devices');
  await page.getByRole('button', { name: 'Add device' }).first().click();
  const choices = page.getByRole('radiogroup', { name: 'Relationship' });
  await expect(choices.getByRole('radio')).toHaveCount(3);
  await expect(choices.getByRole('radio', { name: /^Peer device/ })).toHaveAttribute('aria-checked', 'true');
  const worker = choices.getByRole('radio', { name: /^Controlled device/ });
  await expect(worker).toContainText('This→New');
  await expect(worker).toContainText('The new device cannot reach any of your devices');
  await expect(choices.getByRole('radio', { name: /^Guest/ })).toContainText('New→This');
  await worker.click();
  await expect(worker).toHaveAttribute('aria-checked', 'true');
  await page.getByRole('button', { name: 'Create QR code and link' }).click();
  await expect.poll(() => sent).toEqual({ purpose: 'worker' });
});

test('the joining device sees what it agrees to before confirming the code', async ({ page }) => {
  await page.route('**/api/v1/app/pair/accept', route => route.fulfill({ json: { flow_id: 'pair-flow-1', sas: '1234-5678-9012', purpose: 'worker' } }));
  await ready(page, '/app/devices');
  await page.getByRole('button', { name: 'Add device' }).first().click();
  await page.getByLabel('Pairing link or server address').fill('cranibox://pair/test');
  await page.getByRole('button', { name: 'Continue to verification' }).click();
  const meaning = page.getByRole('region', { name: 'After pairing' });
  await expect(meaning).toContainText('Let the other device control this device');
  await expect(meaning).toContainText('the other device can run commands, open terminals and manage projects on this device');
  await expect(meaning).toHaveClass(/warn/);
  await expect(page.getByRole('button', { name: 'Codes match, allow control' })).toBeVisible();
});

test('a pairing approval says which way access goes instead of the raw purpose', async ({ page }) => {
  await ready(page, '/app/inbox');
  const card = page.locator('.cx-approval').filter({ hasText: 'Verify a new device' });
  const meaning = card.getByRole('region', { name: 'After pairing' });
  await expect(meaning).toContainText('As Guest');
  await expect(meaning).toContainText('New→This');
  await expect(card.locator('.cx-untrusted').filter({ hasText: /^guest$/ })).toHaveCount(0);
});

test('routing through your own devices can be switched off on either side', async ({ page }) => {
  const state = { use_devices: true, forward_for_devices: true };
  const puts: unknown[] = [];
  await page.route('**/api/v1/app/network/routing', async route => {
    if (route.request().method() === 'PUT') { const change = route.request().postDataJSON(); puts.push(change); Object.assign(state, change); }
    await route.fulfill({ json: state });
  });
  await ready(page, '/nodes');
  const group = page.getByRole('region', { name: 'Routing through devices' });
  await expect(group).toContainText('they see only encrypted data');
  const use = group.getByRole('switch', { name: /Go through my other devices when faster/ });
  const forward = group.getByRole('switch', { name: /Let my other devices go through this one/ });
  await expect(use).toBeChecked();
  await forward.click({ force: true });
  await expect.poll(() => puts).toEqual([{ forward_for_devices: false }]);
  await expect(forward).not.toBeChecked();
  await expect(use).toBeChecked();
});
