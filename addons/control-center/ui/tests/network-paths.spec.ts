import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  // Browser requests stay on this test's loopback origin.
  await page.route('**/*', route => new URL(route.request().url()).origin === 'http://127.0.0.1:19084' ? route.continue() : route.abort());
  await page.addInitScript(() => { if (!localStorage.getItem('cranibox.language')) localStorage.setItem('cranibox.language', 'en'); });
});

test('matrix keeps directional values, aliases, scopes, latency bands and stale ages', async ({ page }) => {
  await page.route('**/api/v1/app/devices', async route => {
    const response = await route.fetch();
    const devices = await response.json();
    devices[0].name = 'My Mac';
    await route.fulfill({ json: devices });
  });
  await page.goto('/#/nodes');
  const section = page.getByRole('region', { name: 'Connections between devices' });
  const table = section.getByRole('table');
  await expect(table).toBeVisible();
  await expect(table.getByRole('row')).toHaveCount(5);
  await expect(table.locator('th[scope=col]')).toHaveCount(5);
  await expect(table.locator('th[scope=row]')).toHaveCount(4);
  await expect(table.getByRole('rowheader', { name: 'My Mac' })).toBeVisible();
  const mac = table.getByRole('row').filter({ has: page.getByRole('rowheader', { name: 'My Mac', exact: true }) });
  await expect(mac.locator('td').nth(0)).toBeEmpty();
  const relay = mac.locator('td').nth(1).locator('.cx-path-value');
  await expect(relay).toHaveText('⇄48 ms');
  await expect(relay).toHaveClass(/cx-latency-good/);
  await expect(relay).toHaveAttribute('title', /My Mac → Home server · Relay relay.n0.iroh.network · 48 ms · 2 min ago/);
  await relay.focus();
  await expect(relay).toBeFocused();
  const lan = mac.locator('td').nth(2).locator('.cx-path-value');
  await expect(lan).toHaveText('⌂8 ms');
  await expect(lan).toHaveClass(/cx-latency-fast.*cx-path-stale/);
  await expect(lan).toHaveAttribute('title', /11 min ago · Stale/);
  await expect(lan).toHaveCSS('opacity', '0.55');
  const server = table.getByRole('row').filter({ has: page.getByRole('rowheader', { name: 'Home server', exact: true }) });
  await expect(server.locator('td').nth(0)).toHaveText('↗105 ms');
  await expect(server.locator('td').nth(0).locator('span').first()).toHaveClass(/cx-latency-slow/);
  await expect(server.locator('td').nth(1)).toBeEmpty();
  await expect(table.locator('.cx-latency-high')).toHaveText('◇280 ms');
  await expect(table.locator('[title*="easytier"]').last()).toHaveAttribute('aria-label', /Unknown latency/);
  await expect(table.locator('[aria-label*="Not measured"]').first()).toHaveText('?—');
  await expect(section.getByLabel('Connection legend')).toContainText('Internet direct (hole-punched)');
  await expect(section.locator('.cx-path-list')).toBeHidden();
});

test('narrow screens show groups and route details in Chinese', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  // Choose the language before the first load: opening '/' first lets its
  // redirect to the home route land after the navigation to the nodes page.
  await page.addInitScript(() => localStorage.setItem('cranibox.language', 'zh-cn'));
  await page.goto('/#/nodes');
  const section = page.getByRole('region', { name: '设备间连接' });
  await expect(section.locator('.cx-path-matrix')).toBeHidden();
  const groups = section.locator('.cx-path-list > section');
  await expect(groups).toHaveCount(4);
  await expect(groups.first().getByRole('heading')).toHaveText('从 MacBook Pro');
  await expect(groups.first().getByRole('listitem')).toHaveCount(3);
  await expect(groups.first().getByRole('listitem').first()).toContainText('中转 relay.n0.iroh.network · 48 ms · 2 分钟前');
  await expect(groups.first().locator('.cx-path-stale')).toHaveAttribute('title', /11 分钟前 · 已过期/);
  await expect(groups.first().getByRole('listitem').last()).toContainText('easytier · 延迟未知');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('missing measurements and API errors stay explicit', async ({ page }) => {
  await page.route('**/api/v1/app/network/paths', route => route.fulfill({ status: 503, json: { error: 'Paths temporarily unavailable' } }));
  await page.goto('/#/nodes');
  const section = page.getByRole('region', { name: 'Connections between devices' });
  await expect(section.getByRole('alert')).toContainText('Paths temporarily unavailable');
  await expect(section.getByRole('table').locator('[aria-label*="Not measured"]')).toHaveCount(12);
});
