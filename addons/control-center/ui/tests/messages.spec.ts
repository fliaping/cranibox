import { openNetworkSettings } from './network-page';
import { expect, test, type Page } from '@playwright/test';

const incomingID = '11111111-1111-4111-8111-111111111111';
const agentID = '44444444-4444-4444-8444-444444444444';
async function mailbox(page: Page, failFirstSend = false) {
  const actions: any[] = [], now = Math.floor(Date.now() / 1000);
  const state = {
    owner: 'local-device', capacity: 256, used: 1, max_text_bytes: 8192,
    incoming: [{ message: { id: incomingID, thread: incomingID, sender: 'remote-device', recipient: 'local-device', text: '<img src=x onerror="window.unsafe=true">\n请检查项目，收到后回复。', created_at: now, expires_at: now + 604800, in_reply_to: null, sender_session: null as string | null, target_session: null as string | null }, status: 'received' }],
    outgoing: [] as any[],
  };
  let fail = failFirstSend;
  const agents = { error: '', entries: [{ id: agentID, name: 'Review Agent', harness: 'local-cli', status: 'ready', revision: 1, updated_at: now, expires_at: now + 300 }] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'agent_sessions') {
      if (agents.error) await route.fulfill({ status: 503, json: { error: agents.error } });
      else await route.fulfill({ json: { owner: command.node || state.owner, observed_at: Math.floor(Date.now() / 1000), lease_secs: 300, capacity: 64, sessions: agents.entries } });
      return;
    }
    if (command.operation !== 'messages') {
      await route.fulfill({ json: { node_id: state.owner, routes: [{ node: 'remote-device', url: 'https://private.example' }], grants: [] } }); return;
    }
    const action = command.command; actions.push(action);
    if (action.operation === 'send') {
      if (!state.outgoing.some(item => item.message.id === action.id)) state.outgoing.push({ message: { ...action, sender: state.owner, recipient: action.node, thread: action.thread || action.id, in_reply_to: action.in_reply_to || null, created_at: now, expires_at: now + 604800 }, status: 'queued', attempts: 0 });
      if (fail) { fail = false; await route.fulfill({ status: 503, json: { error: 'Response interrupted. Retry with the same message ID.' } }); return; }
    }
    if (action.operation === 'acknowledge') state.incoming.find(item => item.message.id === action.id)!.status = 'acknowledged';
    if (action.operation === 'archive') {
      state.incoming = state.incoming.filter(item => item.message.id !== action.id);
      state.outgoing = state.outgoing.filter(item => item.message.id !== action.id);
    }
    state.used = state.incoming.length + state.outgoing.length;
    await route.fulfill({ json: state });
  });
  return { actions, state, agents };
}

test('ambiguous submission retains the draft and retries the same ID without duplicating or executing it', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  const { actions, state } = await mailbox(page, true);
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  await expect(panel.getByRole('heading', { name: 'Device messages', exact: true })).toBeVisible();
  const field = panel.getByRole('textbox', { name: 'Message text', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  await field.fill('Keep the original draft\n请检查文件');
  await panel.getByRole('button', { name: 'Save and send', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Response interrupted');
  await expect(field).toHaveValue('Keep the original draft\n请检查文件');
  await expect(field).toBeFocused();
  await panel.getByRole('button', { name: 'Save and send', exact: true }).click();
  await expect(field).toHaveValue('');
  await expect(panel.locator('.message-status')).toHaveText('Queued locally');
  const sends = actions.filter(action => action.operation === 'send');
  expect(sends).toHaveLength(2); expect(sends[1]).toEqual(sends[0]); expect(state.outgoing).toHaveLength(1);
  await page.reload();
  await panel.getByRole('button', { name: /^Outbox/ }).click();
  await expect(panel.locator('.message-text')).toHaveText('Keep the original draft\n请检查文件');
  expect(actions.filter(action => action.operation === 'send')).toHaveLength(2);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Keep the original draft');
  expect(actions.some(action => ['exec', 'sessions', 'configure'].includes(action.operation))).toBe(false);
  expect(errors).toEqual([]);
});

test('incoming text is inert, replies retain the thread, acknowledgements and archiving are explicit', async ({ page }) => {
  const { actions } = await mailbox(page);
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  await expect(panel.locator('.message-text')).toContainText('<img src=x');
  await expect(panel.locator('.message-text img')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  await panel.getByRole('button', { name: 'Reply', exact: true }).click();
  await expect(panel.getByRole('combobox', { name: 'Target device', exact: true })).toHaveValue('remote-device');
  await expect(panel.getByRole('combobox', { name: 'Target device', exact: true })).toBeDisabled();
  await panel.getByRole('textbox', { name: 'Message text', exact: true }).fill('Reply received.');
  await panel.getByRole('button', { name: 'Save and send', exact: true }).click();
  expect(actions.find(action => action.operation === 'send')).toMatchObject({ node: 'remote-device', text: 'Reply received.', thread: incomingID, in_reply_to: incomingID });
  expect(actions.some(action => action.operation === 'acknowledge')).toBe(false);
  await panel.getByRole('button', { name: /^Inbox/ }).click();
  await panel.getByRole('button', { name: 'Acknowledge receipt', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('Acknowledged');
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Archive', exact: true }).click();
  expect(actions.some(action => action.operation === 'archive')).toBe(false);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Archive', exact: true }).click();
  await expect(panel.getByRole('heading', { name: 'No incoming messages', exact: true })).toBeVisible();
});

test('byte limits, responsive themes and failed initial loading provide usable feedback', async ({ page }) => {
  await mailbox(page);
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  const field = panel.getByRole('textbox', { name: 'Message text', exact: true });
  const send = panel.getByRole('button', { name: 'Save and send', exact: true });
  await expect(send).toBeDisabled();
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  await field.fill('🙂'.repeat(2049));
  await expect(field).toHaveAttribute('aria-invalid', 'true');
  await expect(panel.getByText('8196 / 8192 UTF-8 bytes · Plain text', { exact: true })).toBeVisible();
  await expect(send).toBeDisabled();
  await field.fill('A short draft');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375 && theme === 'dark' || width === 1440 && theme === 'light') await panel.screenshot({ path: '/tmp/cranibox-messages-' + width + '-' + theme + '.png' });
    }
  }
  await page.route('**/api/peers', route => route.fulfill({ status: 503, json: { error: 'Local service unavailable' } }));
  await page.reload();
  await expect(panel.getByRole('alert')).toContainText('Local service unavailable');
  await expect(panel.getByRole('status')).toContainText('Messages could not be read');
  await expect(panel.getByRole('button', { name: 'Refresh messages', exact: true })).toBeEnabled();
});

test('message access can be granted without command, desktop or terminal access', async ({ page }) => {
  const actions: any[] = [], token = 'b'.repeat(64);
  const catalog = { node_id: 'local-device', grants: [] as any[], routes: [] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'grant') {
      catalog.grants.push({ node: command.node, permissions: command.permissions });
      await route.fulfill({ json: { token, node: command.node, catalog } }); return;
    }
    await route.fulfill({ json: catalog });
  });
  await openNetworkSettings(page);
  const panel = page.getByRole('region', { name: 'Incoming access', exact: true });
  await panel.getByRole('button', { name: 'Grant device access', exact: true }).click();
  await panel.getByLabel(/^Caller node ID/).fill('another-device');
  const permission = panel.getByLabel('Device messages and collaboration tasks', { exact: true });
  await expect(permission).not.toBeChecked(); await permission.check();
  await panel.getByRole('button', { name: 'Create access token', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Save access token', exact: true })).toBeVisible();
  expect(actions.find(action => action.operation === 'grant')).toEqual({ operation: 'grant', node: 'another-device', require_verified: true, permissions: { exec: false, terminals: false, projects: false, desktop: false, computer: false, browser: false, messages: true, activity: false, spaces: false } });
});

test('addressed messages preserve the selected Agent and ID after an interrupted response', async ({ page }) => {
  const { actions, state } = await mailbox(page, true);
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  const picker = panel.getByRole('combobox', { name: 'Recipient Agent', exact: true });
  await picker.selectOption(agentID);
  const field = panel.getByRole('textbox', { name: 'Message text', exact: true });
  await field.fill('Review Agent, please check the diff.');
  const send = panel.getByRole('button', { name: 'Save and send', exact: true });
  await send.click();
  await expect(panel.getByRole('alert')).toContainText('Response interrupted');
  await expect(picker).toHaveValue(agentID);
  await expect(field).toHaveValue('Review Agent, please check the diff.');
  await send.click();
  await expect(field).toHaveValue('');
  const sent = actions.filter(action => action.operation === 'send');
  expect(sent).toHaveLength(2); expect(sent[0]).toEqual(sent[1]);
  expect(sent[0].target_session).toBe(agentID);
  expect(sent[0].sender_session).toBeUndefined();
  expect(state.outgoing).toHaveLength(1);
  await expect(panel.locator('.message-agent-address')).toContainText(agentID);
  await panel.getByRole('button', { name: 'Reply', exact: true }).click();
  await expect(picker).toBeDisabled();
  await field.fill('Following up with the same Agent.');
  await expect(send).toBeEnabled();
  await send.click();
  expect(actions.filter(action => action.operation === 'send').at(-1)).toMatchObject({ target_session: agentID, in_reply_to: sent[0].id });
});

test('unconfirmed and expired session choices block sending until an explicit device-inbox choice', async ({ page }) => {
  const { agents, actions } = await mailbox(page);
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  const picker = panel.getByRole('combobox', { name: 'Recipient Agent', exact: true });
  await picker.selectOption(agentID);
  await panel.getByRole('textbox', { name: 'Message text', exact: true }).fill('Keep this addressed draft.');
  agents.error = 'Session directory unavailable';
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Session directory unavailable');
  const send = panel.getByRole('button', { name: 'Save and send', exact: true });
  await expect(send).toBeDisabled(); await expect(picker).toHaveValue(agentID);
  agents.error = ''; agents.entries[0].expires_at = Math.floor(Date.now() / 1000) - 1; agents.entries[0].updated_at = agents.entries[0].expires_at - 300;
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  await expect(send).toBeDisabled(); await expect(picker).toHaveValue(agentID);
  expect(actions.some(action => action.operation === 'send')).toBe(false);
  await picker.selectOption('');
  await send.click();
  expect(actions.find(action => action.operation === 'send')).not.toHaveProperty('target_session');
});

test('replies lock the original sender session and cancellation explicitly releases its address', async ({ page }) => {
  const { state, actions, agents } = await mailbox(page);
  state.incoming[0].message.sender_session = agentID;
  state.incoming[0].message.target_session = '55555555-5555-4555-8555-555555555555';
  await page.goto('/#/messages');
  const panel = page.getByRole('region', { name: 'Device messages', exact: true });
  await panel.getByRole('button', { name: 'Reply', exact: true }).click();
  const picker = panel.getByRole('combobox', { name: 'Recipient Agent', exact: true });
  await expect(picker).toHaveValue(agentID); await expect(picker).toBeDisabled();
  await panel.getByRole('textbox', { name: 'Message text', exact: true }).fill('Reply to the author.');
  await panel.getByRole('button', { name: 'Save and send', exact: true }).click();
  expect(actions.find(action => action.operation === 'send')).toMatchObject({ target_session: agentID, thread: incomingID, in_reply_to: incomingID });
  await panel.getByRole('button', { name: /^Inbox/ }).click();
  agents.entries[0].expires_at = Math.floor(Date.now() / 1000) - 1;
  await panel.getByRole('button', { name: 'Reply', exact: true }).click();
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await panel.getByRole('textbox', { name: 'Message text', exact: true }).fill('An unsent reply draft.');
  await expect(panel.getByRole('button', { name: 'Save and send', exact: true })).toBeDisabled();
  await expect(picker).toHaveValue(agentID);
  await panel.getByRole('button', { name: 'Cancel reply', exact: true }).click();
  await expect(picker).toHaveValue(''); await expect(picker).toBeEnabled();
  await expect(panel.getByRole('textbox', { name: 'Message text', exact: true })).toHaveValue('An unsent reply draft.');
  await expect(panel.getByRole('button', { name: 'Save and send', exact: true })).toBeEnabled();
  await page.setViewportSize({ width: 375, height: 1000 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});
