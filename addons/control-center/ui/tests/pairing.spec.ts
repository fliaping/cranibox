import { expect, test } from '@playwright/test';

test('grants are strict by default, opting out is explicit and pairing codes must be typed to verify', async ({ page }) => {
  const actions: any[] = [];
  const catalog: any = { node_id: 'target-id', routes: [], grants: [
    { node: 'legacy-caller', permissions: { exec: true, terminals: false }, require_verified: false, peer: null },
    { node: 'pending-caller', permissions: { exec: false, terminals: true }, require_verified: true, peer: null },
    { node: 'pinned-caller', permissions: { exec: false, terminals: false, messages: true }, require_verified: true, peer: { fingerprint: 'ab12', verified: false, pairing_code: '1234-5678-9012' } },
  ], listener: null, listener_runtime: { active: false, address: null, applied: true } };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'verify') {
      const grant = catalog.grants.find((g: any) => g.node === command.node);
      if (command.code.replace(/\D/g, '') !== grant.peer.pairing_code.replace(/\D/g, '')) { await route.fulfill({ status: 400, json: { error: 'pairing code does not match; compare both devices again' } }); return; }
      grant.peer.verified = true; await route.fulfill({ json: { node: command.node, verified: ['grant'], catalog } }); return;
    }
    if (command.operation === 'grant') {
      catalog.grants.push({ node: command.node, permissions: command.permissions, require_verified: command.require_verified, peer: null });
      await route.fulfill({ json: { catalog, node: command.node, token: 'a'.repeat(64) } }); return;
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const panel = page.getByRole('region', { name: 'Incoming access', exact: true });
  const list = panel.getByLabel('Granted devices');
  await expect(list).toContainText('Pairing verification not required (compatibility mode)');
  await expect(list).toContainText('Waiting for the first connection, then verify the pairing code');
  const form = panel.getByRole('form', { name: 'Verify pairing code pinned-caller' });
  await expect(form).toContainText('Not verified: all capabilities are paused');
  await expect(form).toContainText('1234-5678-9012');
  // Nothing is verified without typing the other device's code.
  expect(actions.some(a => a.operation === 'verify')).toBe(false);
  const input = form.getByLabel('Enter the code shown on the other device');
  await input.fill('1111-2222-3333');
  await expect(input).toBeFocused();
  await form.getByRole('button', { name: 'Verify and trust' }).click();
  await expect(form.getByRole('alert')).toContainText('does not match');
  await input.fill('1234 5678 9012');
  await form.getByRole('button', { name: 'Verify and trust' }).click();
  await expect(list).toContainText('Pairing code verified');
  expect(actions.filter(a => a.operation === 'verify').map(a => a.code)).toEqual(['1111-2222-3333', '1234 5678 9012']);
  // New grants are strict unless the owner explicitly opts out.
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  const opt = panel.getByLabel('Allow before the pairing code is verified (not recommended)');
  await expect(opt).not.toBeChecked();
  await panel.getByLabel(/^Caller node ID/).fill('strict-caller');
  await panel.getByLabel('Execute commands', { exact: true }).check();
  await panel.getByRole('button', { name: 'Create access token', exact: true }).click();
  await page.getByRole('button', { name: 'Close token window' }).click();
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await panel.getByLabel(/^Caller node ID/).fill('older-node');
  await panel.getByLabel('Execute commands', { exact: true }).check();
  await opt.check();
  await panel.getByRole('button', { name: 'Create access token', exact: true }).click();
  const grants = actions.filter(a => a.operation === 'grant');
  expect(grants.map(a => [a.node, a.require_verified])).toEqual([['strict-caller', true], ['older-node', false]]);
});

test('the caller shows the pairing code the target asks for and can verify the target', async ({ page }) => {
  const actions: any[] = [];
  const catalog: any = { node_id: 'caller-id', grants: [], listener: null, listener_runtime: { active: false, address: null, applied: true }, routes: [
    { node: 'target-node', url: 'http://10.0.0.2:7700', allow_private_http: true, identity: { fingerprint: 'cd34', verified: false, pairing_code: '2713-7394-7426' }, paths: [{ url: 'http://10.0.0.2:7700', allow_private_http: true }] },
  ] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'verify') {
      const target = catalog.routes.find((r: any) => r.node === command.node);
      if (command.code.replace(/\D/g, '') !== target.identity.pairing_code.replace(/\D/g, '')) { await route.fulfill({ status: 400, json: { error: 'pairing code does not match; compare both devices again' } }); return; }
      target.identity.verified = true; await route.fulfill({ json: { node: command.node, verified: ['route'], catalog } }); return;
    }
    await route.fulfill({ json: catalog });
  });
  await page.goto('/#/nodes');
  const form = page.getByRole('form', { name: 'Verify pairing code target-node' });
  await expect(form).toContainText('Not verified: enter this code on the target device');
  await expect(form).toContainText('2713-7394-7426');
  expect(actions.some(a => a.operation === 'verify')).toBe(false);
  const input = form.getByLabel('Enter the code shown on the other device');
  await input.fill('0000-0000-0000');
  await form.getByRole('button', { name: 'Verify and trust' }).click();
  await expect(form.getByRole('alert')).toContainText('does not match');
  await input.fill('2713 7394 7426');
  await form.getByRole('button', { name: 'Verify and trust' }).click();
  await expect(page.getByText('Pairing code verified')).toBeVisible();
  expect(actions.filter(a => a.operation === 'verify').map(a => [a.node, a.code])).toEqual([['target-node', '0000-0000-0000'], ['target-node', '2713 7394 7426']]);
});
