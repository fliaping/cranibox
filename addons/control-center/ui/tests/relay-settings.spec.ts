import { expect, test } from '@playwright/test';
import { openNetworkSettings } from './network-page';

const defaults = () => ({ enabled: false, public_url: '', listen: '127.0.0.1:7378' });

test('relay settings persist separately from running status, survive reload and can be disabled', async ({ page }) => {
  let settings = defaults();
  const commands: any[] = [];
  let fail = false;
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.route('**/api/network', route => {
    const command = route.request().postDataJSON();
    commands.push(command);
    if (command.operation === 'relay_settings') {
      if (fail) return route.fulfill({ status: 400, json: { error: 'Use a public HTTPS origin.' } });
      const { operation, ...saved } = command;
      settings = saved;
    }
    return route.fulfill({ json: { iroh: { lan: false, relays: [], n0_fallback: true, learned_relays: [], this_relay: { enabled: false, public_url: null, listen: null, settings, restart_required: settings.enabled } } } });
  });
  await openNetworkSettings(page);
  const panel = page.getByRole('region', { name: 'This device as a relay', exact: true });
  await expect(panel).toContainText('public internet');
  await expect(panel).toContainText('WebSocket');
  await expect(panel).toContainText('Not running');
  const toggle = panel.getByRole('switch');
  await toggle.click();
  await panel.getByLabel('Public HTTPS address', { exact: true }).fill('https://relay.example.com');
  await panel.getByLabel('Relay listener address and port', { exact: true }).fill('0.0.0.0:7378');
  await panel.getByRole('button', { name: 'Save relay configuration', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Restart Cranibox');
  await expect(panel).toContainText('Not running');
  expect(commands.filter(c => c.operation === 'relay_settings')).toEqual([{ operation: 'relay_settings', enabled: true, public_url: 'https://relay.example.com', listen: '0.0.0.0:7378' }]);
  await page.reload();
  await expect(toggle).toBeChecked();
  await expect(panel.getByLabel('Public HTTPS address', { exact: true })).toHaveValue('https://relay.example.com');
  await expect(panel.getByRole('status')).toContainText('Restart Cranibox');
  fail = true;
  await panel.getByLabel('Public HTTPS address', { exact: true }).fill('http://relay.example.com');
  await panel.getByRole('button', { name: 'Save relay configuration', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Use a public HTTPS origin');
  await expect(panel.getByLabel('Public HTTPS address', { exact: true })).toHaveValue('http://relay.example.com');
  fail = false;
  await panel.getByLabel('Public HTTPS address', { exact: true }).fill('https://relay.example.com');
  await toggle.click();
  await panel.getByRole('button', { name: 'Save relay configuration', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Relay configuration saved');
  await page.reload();
  await expect(toggle).not.toBeChecked();
  await expect(panel.getByRole('status')).toHaveCount(0);
  await page.setViewportSize({ width: 375, height: 1000 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('Chinese relay settings explain public exposure and restart', async ({ page }, testInfo) => {
  await page.addInitScript(() => localStorage.setItem('cranibox.onboarded', '1'));
  await page.route('**/api/network', route => route.fulfill({ json: { iroh: { lan: false, relays: [], n0_fallback: true, learned_relays: [], this_relay: { enabled: false, public_url: null, settings: defaults() } } } }));
  await page.goto('http://127.0.0.1:19082/#/nodes');
  const panel = page.getByRole('region', { name: '这台设备作为中继', exact: true });
  await expect(panel).toContainText('暴露到公网');
  await expect(panel).toContainText('保存后重启');
  await expect(panel.getByLabel('中继监听地址与端口', { exact: true })).toHaveValue('127.0.0.1:7378');
  await panel.screenshot({ path: testInfo.outputPath('relay-settings-zh.png'), animations: 'disabled' });
});
