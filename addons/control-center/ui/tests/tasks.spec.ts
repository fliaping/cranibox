import { expect, test, type Page } from '@playwright/test';

const id = '11111111-1111-4111-8111-111111111111', dispatch = '22222222-2222-4222-8222-222222222222';
async function tasks(page: Page, failCreate = false) {
  const now = Math.floor(Date.now() / 1000), actions: any[] = [];
  const task = { id, sender: 'remote-device', recipient: 'local-device', title: 'Review the build', description: '<img src=x onerror="window.unsafe=true">\nRead the build log and report the evidence.', created_at: now, expires_at: now + 604800 };
  const receipt = { task: id, dispatch, sender: task.sender, recipient: task.recipient, state: 'available', session: null, progress: null, result: null, revision: 1, updated_at: now };
  const state = { owner: 'local-device', capacity: 128, used: 1, max_attempts: 8, incoming: [{ task, current: receipt, history: [receipt], grant_active: true }], outgoing: [] as any[] };
  let fail = failCreate;
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'executions') { await route.fulfill({ json: { owner: state.owner, executions: [], used: 0, capacity: 128, parallel: 8, log_limit: 1048576 } }); return; }
    if (command.operation === 'agent_sessions') { await route.fulfill({ json: { owner: command.node || state.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] } }); return; }
    if (command.operation !== 'tasks') { await route.fulfill({ json: { node_id: state.owner, routes: [{ node: 'remote-device' }], grants: [] } }); return; }
    const action = command.command; actions.push(action);
    if (action.operation === 'create') {
      if (!state.outgoing.some(item => item.task.id === action.id)) {
        const current = { ...receipt, task: action.id, dispatch: action.dispatch, sender: state.owner, recipient: action.node, state: 'queued', revision: 0 };
        state.outgoing.push({ task: { ...action, sender: state.owner, recipient: action.node, created_at: now, expires_at: now + 604800 }, current, history: [current] });
      }
      if (fail) { fail = false; await route.fulfill({ status: 503, json: { error: 'Response interrupted. Synchronize with the same task ID.' } }); return; }
    }
    if (action.operation === 'cancel') state.outgoing.find(item => item.task.id === action.id).cancel_requested = true;
    if (action.operation === 'retry') {
      const item = state.outgoing.find(item => item.task.id === action.id);
      if (item.current.dispatch !== action.new_dispatch) { item.current = { ...item.current, dispatch: action.new_dispatch, state: 'queued', session: null, result: null, revision: 0 }; item.history.push(item.current); item.cancel_requested = false; }
    }
    if (action.operation === 'decline') { state.incoming[0].current = { ...receipt, state: 'failed', result: action.reason, revision: 2 }; state.incoming[0].history = [state.incoming[0].current]; }
    if (action.operation === 'archive') { state.incoming = state.incoming.filter(item => item.task.id !== action.id); state.outgoing = state.outgoing.filter(item => item.task.id !== action.id); }
    state.used = state.incoming.length + state.outgoing.length;
    await route.fulfill({ json: state });
  });
  return { actions, state };
}

async function taskRun(page: Page, failStart = false) {
  const fixture = await tasks(page), item = fixture.state.incoming[0] as any;
  const options = { adapter: 'codex', access: 'read_only', cwd: '/home/user/project' };
  const launch = { name: 'Task fixture', program: '/tools/codex', args: ['exec', '-'], cwd: options.cwd, stdin: item.task.description + '\nTask ID: ' + id };
  const binding = { id: '99999999-9999-4999-8999-999999999999', session: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', options, digest: 'b'.repeat(64) };
  const detail = { owner: fixture.state.owner, task: id, dispatch, run: binding, launch, execution: { id: binding.id, session_id: binding.session, state: 'running', exit_code: null as number | null }, stdout: { text: '审查🙂 <img src=x onerror=window.unsafe=true>\n', next_offset: 0 }, stderr: { text: '', next_offset: 0 } };
  detail.stdout.next_offset = Buffer.byteLength(detail.stdout.text);
  const control = { failStart, available: true, starts: 0, detail };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation !== 'tasks' || command.command.operation !== 'run') { await route.fallback(); return; }
    fixture.actions.push(command.command); const action = command.command.command;
    if (action.operation === 'prepare') { binding.options = action.options; detail.launch.cwd = action.options.cwd; await route.fulfill({ json: { owner: fixture.state.owner, task: id, dispatch, digest: binding.digest, options: action.options, launch, available: control.available } }); return; }
    if (action.operation === 'start') {
      if (!item.run) { control.starts++; item.run = { id: binding.id, session: binding.session }; item.current.state = 'running'; item.current.session = binding.session; item.current.revision++; }
      if (control.failStart) { control.failStart = false; await route.fulfill({ status: 503, json: { error: 'Launch response interrupted. Find the original run or retry unchanged.' } }); return; }
    }
    if (action.operation === 'get' && !item.run) { await route.fulfill({ status: 400, json: { error: 'No reviewed run for this dispatch' } }); return; }
    if (action.operation === 'stop') { detail.execution.state = 'cancelled'; item.current.state = 'cancel_requested'; }
    if (action.operation === 'finish') { item.current.state = action.outcome; item.current.result = action.result; item.current.revision++; await route.fulfill({ json: fixture.state }); return; }
    await route.fulfill({ json: { ...detail, stdout: action.stdout_offset ? { text: '', next_offset: detail.stdout.next_offset } : detail.stdout } });
  });
  return { ...fixture, item, control };
}

test('a task launch is reviewed explicitly, deduplicated after an uncertain response and reported separately', async ({ page }) => {
  const { actions, item, control } = await taskRun(page, true);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  await panel.getByRole('button', { name: 'Run with Agent', exact: true }).click();
  await panel.getByRole('textbox', { name: 'Task working directory', exact: true }).fill('/home/user/project');
  await panel.getByRole('button', { name: 'Review task execution', exact: true }).click();
  await expect(panel.getByLabel('Full task instructions')).toContainText('<img src=x');
  await expect(panel.getByLabel('Full task instructions').locator('img')).toHaveCount(0);
  expect(control.starts).toBe(0); expect(item.current.state).toBe('available');
  await panel.getByRole('button', { name: 'Confirm claim and start', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Launch response interrupted');
  await panel.getByRole('button', { name: 'Confirm claim and start', exact: true }).click();
  await expect(panel.getByLabel('Task output stdout')).toContainText('审查🙂');
  expect(control.starts).toBe(1);
  const starts = actions.filter(action => action.operation === 'run' && action.command.operation === 'start');
  expect(starts).toHaveLength(2); expect(starts[0]).toEqual(starts[1]);
  await expect(panel.getByRole('button', { name: 'Submit reviewed result', exact: true })).toBeDisabled();
  await page.reload();
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await expect(panel.getByLabel('Task output stdout')).toContainText('审查🙂');
  expect(control.starts).toBe(1);
  control.detail.execution.state = 'succeeded'; control.detail.execution.exit_code = 0;
  await panel.getByRole('button', { name: 'Find or refresh task run', exact: true }).click();
  await expect(panel.getByText('Process exited · review result · Exit code 0', { exact: true })).toBeVisible();
  expect(item.current.state).toBe('running');
  await panel.getByRole('combobox', { name: 'Task outcome', exact: true }).selectOption('failed');
  await panel.getByRole('textbox', { name: 'Result and verification evidence', exact: true }).fill('Exit zero, but the review lacks the required evidence.');
  await panel.getByRole('button', { name: 'Submit reviewed result', exact: true }).click();
  await expect(panel.locator('.task-result')).toContainText('review lacks the required evidence');
  expect(item.current.state).toBe('failed');
  expect(actions.filter(action => action.operation === 'run' && action.command.operation === 'finish')).toHaveLength(1);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('required evidence');
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
});

test('task run controls handle missing CLIs, narrow screens, explicit stop and UTF-8 result limits', async ({ page }) => {
  const { control } = await taskRun(page); control.available = false;
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  await panel.getByRole('button', { name: 'Run with Agent', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Task Agent', exact: true }).selectOption('claude_code');
  await expect(panel.getByText(/This is a tool restriction; file tools/)).toBeVisible();
  await panel.getByRole('textbox', { name: 'Task working directory', exact: true }).fill('/home/user/project');
  await panel.getByRole('button', { name: 'Review task execution', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Confirm claim and start', exact: true })).toBeDisabled();
  await expect(panel.getByRole('alert')).toContainText('CLI not found');
  control.available = true;
  await panel.getByRole('button', { name: 'Back to edit', exact: true }).click();
  await panel.getByRole('button', { name: 'Review task execution', exact: true }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
  await panel.getByRole('button', { name: 'Confirm claim and start', exact: true }).click();
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Stop task process', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Stop task process', exact: true })).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Stop task process', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Stop task process', exact: true })).toHaveCount(0);
  const result = panel.getByRole('textbox', { name: 'Result and verification evidence', exact: true });
  await result.fill('🙂'.repeat(2049));
  await expect(result).toHaveAttribute('aria-invalid', 'true');
  await expect(panel.getByRole('button', { name: 'Submit reviewed result', exact: true })).toBeDisabled();
  await result.fill('Stopped and inspected partial output.');
  await expect(panel.getByRole('button', { name: 'Submit reviewed result', exact: true })).toBeDisabled();
  await panel.getByRole('combobox', { name: 'Task outcome', exact: true }).selectOption('cancelled');
  await expect(panel.getByRole('button', { name: 'Submit reviewed result', exact: true })).toBeEnabled();
  for (const [width, theme] of [[375, 'dark'], [1440, 'light']] as const) {
    await page.setViewportSize({ width, height: 1000 });
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; (document.activeElement as HTMLElement)?.blur(); }, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ fullPage: true, path: '/tmp/cranibox-task-run-' + width + '-' + theme + '.png' });
  }
});

test('task communication is opt-in and candidate adoption preserves human review and drafts', async ({ page }) => {
  const { actions, control, item } = await taskRun(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  await panel.getByRole('button', { name: 'Run with Agent', exact: true }).click();
  const communication = panel.getByRole('checkbox', { name: 'Enable task communication tools', exact: true });
  await expect(communication).not.toBeChecked();
  await communication.check();
  await panel.getByRole('textbox', { name: 'Task working directory', exact: true }).fill('/home/user/project');
  await panel.getByRole('button', { name: 'Review task execution', exact: true }).click();
  await expect(panel.getByText('Task MCP communication enabled; candidate results are never submitted automatically.', { exact: true })).toBeVisible();
  expect(control.starts).toBe(0);
  expect(actions.find(action => action.command?.operation === 'prepare').command.options.communication).toBe(true);
  await panel.getByRole('button', { name: 'Confirm claim and start', exact: true }).click();
  const candidate = '<img src=x onerror="window.unsafe=true">\nInspected two files; please verify the evidence.';
  (control.detail.run as any).proposal = { outcome: 'succeeded', result: candidate, created_at: Math.floor(Date.now() / 1000) };
  await panel.getByRole('button', { name: 'Find or refresh task run', exact: true }).click();
  const proposal = panel.getByRole('region', { name: 'Agent candidate result', exact: true });
  await expect(proposal).toContainText(candidate);
  await expect(proposal.locator('img')).toHaveCount(0);
  const use = proposal.getByRole('button', { name: 'Use in report for review', exact: true });
  await expect(use).toBeDisabled();
  const result = panel.getByRole('textbox', { name: 'Result and verification evidence', exact: true });
  await result.fill('My investigation is still in progress.');
  control.detail.execution.state = 'succeeded'; control.detail.execution.exit_code = 0;
  await panel.getByRole('button', { name: 'Find or refresh task run', exact: true }).click();
  await expect(use).toBeEnabled();
  page.once('dialog', dialog => dialog.dismiss());
  await use.click();
  await expect(result).toHaveValue('My investigation is still in progress.');
  await expect(panel.getByRole('combobox', { name: 'Task outcome', exact: true })).toHaveValue('');
  page.once('dialog', dialog => dialog.accept());
  await use.click();
  await expect(result).toHaveValue(candidate);
  await expect(result).toBeFocused();
  expect(item.current.state).toBe('running');
  expect(actions.some(action => action.command?.operation === 'finish')).toBe(false);
  // A reload recovers the durable proposal but never adopts or reports it silently.
  await page.reload();
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await expect(proposal).toContainText(candidate);
  await expect(result).toHaveValue('');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
  await use.click();
  await panel.getByRole('combobox', { name: 'Task outcome', exact: true }).selectOption('failed');
  await result.fill('Operator checked the files: the candidate lacks required evidence.');
  await panel.getByRole('button', { name: 'Submit reviewed result', exact: true }).click();
  await expect(panel.locator('.task-result')).toContainText('candidate lacks required evidence');
  const reports = actions.filter(action => action.command?.operation === 'finish');
  expect(reports).toHaveLength(1);
  expect(reports[0].command).toMatchObject({ outcome: 'failed', result: 'Operator checked the files: the candidate lacks required evidence.' });
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Inspected two files');
});

test('an uncertain submission retains its IDs and draft and cannot start a terminal', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  const { actions, state } = await tasks(page, true);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  await panel.getByLabel('Task title', { exact: true }).fill('Check the release');
  await panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Read the changelog and report three findings.');
  await panel.getByRole('button', { name: 'Save and delegate', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Response interrupted');
  await expect(panel.getByLabel('Task title', { exact: true })).toBeFocused();
  await expect(panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true })).toHaveValue('Read the changelog and report three findings.');
  await panel.getByRole('button', { name: 'Save and delegate', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('Queued locally');
  const creates = actions.filter(action => action.operation === 'create');
  expect(creates).toHaveLength(2); expect(creates[1]).toEqual(creates[0]); expect(state.outgoing).toHaveLength(1);
  await page.reload();
  await expect(panel.getByRole('heading', { name: 'Check the release', exact: true })).toBeVisible();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Read the changelog');
  expect(actions.some(action => ['exec', 'sessions', 'claim', 'finish'].includes(action.operation))).toBe(false);
  expect(errors).toEqual([]);
});

test('cancellation is a request and new attempts require an explicit final report', async ({ page }) => {
  const { state, actions } = await tasks(page);
  const current = { ...state.incoming[0].current, sender: 'local-device', recipient: 'remote-device', state: 'running', session: '33333333-3333-4333-8333-333333333333', revision: 2 };
  const item = { task: { ...state.incoming[0].task, sender: 'local-device', recipient: 'remote-device' }, current, history: [current], synced_at: Math.floor(Date.now() / 1000), cancel_requested: false };
  state.outgoing.push(item);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await expect(panel.locator('.message-status')).toHaveText('Claimed by Agent');
  await expect(panel.getByRole('button', { name: 'Create new attempt', exact: true })).toHaveCount(0);
  await expect(panel.getByRole('button', { name: 'Archive', exact: true })).toHaveCount(0);
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Request cancellation', exact: true }).click();
  expect(actions.some(action => action.operation === 'cancel')).toBe(false);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Request cancellation', exact: true }).click();
  await expect(panel.getByText(/Execution is not yet confirmed stopped/)).toBeVisible();
  await expect(panel.locator('.message-status')).toHaveText('Claimed by Agent');
  item.current = { ...current, state: 'cancelled', result: 'Saved work and stopped.', revision: 4 } as any; item.history = [item.current];
  await panel.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('Cancellation confirmed');
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Create new attempt', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('Queued locally');
  expect(actions.find(action => action.operation === 'retry')).toMatchObject({ id, dispatch });
  expect(actions.find(action => action.operation === 'retry').new_dispatch).not.toBe(dispatch);
  await panel.getByText('Task, attempt and history', { exact: true }).click();
  await expect(panel.locator('.task-history li')).toHaveCount(2);
  await expect(panel.locator('.task-history')).toContainText('Saved work and stopped.');
});

test('incoming task data is inert and decline requires a reason before archiving', async ({ page }) => {
  const { actions } = await tasks(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: /^For this device/ }).click();
  await expect(panel.locator('.message-text')).toContainText('<img src=x');
  await expect(panel.locator('.message-text img')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  await expect(panel.getByRole('button', { name: 'Copy Agent instructions', exact: true })).toBeVisible();
  await panel.getByRole('button', { name: 'Decline task', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Confirm decline', exact: true })).toBeDisabled();
  await panel.getByRole('textbox', { name: 'Decline reason', exact: true }).fill('This device has no Agent for this task.');
  await panel.getByRole('button', { name: 'Confirm decline', exact: true }).click();
  await expect(panel.locator('.task-result')).toContainText('This device has no Agent for this task.');
  expect(actions.find(action => action.operation === 'decline')).toEqual({ operation: 'decline', id, dispatch, reason: 'This device has no Agent for this task.' });
  await panel.getByRole('button', { name: 'Archive', exact: true }).click();
  await expect(panel.getByRole('heading', { name: 'No tasks for this device', exact: true })).toBeVisible();
});

test('UTF-8 limits, responsive themes and service errors remain usable', async ({ page }) => {
  await tasks(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  await panel.getByLabel('Task title', { exact: true }).fill('🙂'.repeat(61));
  await panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Short description');
  await expect(panel.getByLabel('Task title', { exact: true })).toHaveAttribute('aria-invalid', 'true');
  await expect(panel.getByRole('button', { name: 'Save and delegate', exact: true })).toBeDisabled();
  await panel.getByLabel('Task title', { exact: true }).fill('Review the project');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await panel.getByRole('button', { name: /^For this device/ }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width <= 768) {
        const feed = await panel.locator('.messages-feed').boundingBox(), compose = await panel.locator('.messages-compose').boundingBox();
        expect(feed!.y).toBeLessThan(compose!.y);
        await panel.getByRole('button', { name: 'Delegate a task', exact: true }).click();
        await expect(panel.getByLabel('Task title', { exact: true })).toBeFocused();
      }
      if (width === 375 && theme === 'dark' || width === 1440 && theme === 'light') {
        await page.evaluate(() => { (document.activeElement as HTMLElement)?.blur(); window.scrollTo(0, 0); });
        await page.screenshot({ fullPage: true, path: '/tmp/cranibox-tasks-' + width + '-' + theme + '.png' });
      }
    }
  }
  await page.route('**/api/peers', route => route.fulfill({ status: 503, json: { error: 'Device service unavailable' } }));
  await page.reload();
  await expect(panel.getByRole('alert')).toContainText('Device service unavailable');
  await expect(panel.getByRole('status')).toContainText('Tasks could not be read');
  await expect(panel.getByRole('button', { name: 'Refresh tasks', exact: true })).toBeEnabled();
});

const session = '44444444-4444-4444-8444-444444444444';
async function agents(page: Page) {
  const fixture = await tasks(page);
  const now = Math.floor(Date.now() / 1000);
  const reports = [
    { id: session, name: 'Build reviewer', harness: 'custom-cli', status: 'ready', updated_at: now, expires_at: now + 300, revision: 1 },
    { id: '55555555-5555-4555-8555-555555555555', name: '<img src=x onerror="window.unsafe=true">', harness: 'old-cli', status: 'busy', updated_at: now - 400, expires_at: now - 100, revision: 2 },
    { id: '66666666-6666-4666-8666-666666666666', name: 'Finished reviewer', harness: 'custom-cli', status: 'ended', updated_at: now, expires_at: now + 300, revision: 3 },
  ];
  const state = { now, reports, fail: false, offline: false, calls: [] as any[] };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'agent_sessions') {
      state.calls.push(command);
      if (state.fail && command.node) { await route.fulfill({ status: 503, json: { error: 'Agent directory unavailable' } }); return; }
      await route.fulfill({ json: { owner: command.node || fixture.state.owner, observed_at: state.now, lease_secs: 300, capacity: 64, sessions: state.reports, ...(state.offline && command.node ? { sync: { state: 'offline', synced_at: state.now * 1000 } } : {}) } }); return;
    }
    if (command.operation === 'list') {
      await route.fulfill({ json: { node_id: fixture.state.owner, routes: [{ node: 'remote-device' }, { node: 'another-device' }], grants: [] } }); return;
    }
    await route.fallback();
  });
  return { ...fixture, directory: state };
}

test('registered Agent reports remain plain text and a targeted task retains its session', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { actions, directory } = await agents(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  const overview = panel.getByRole('region', { name: 'Agents across devices', exact: true });
  await overview.getByRole('checkbox', { name: 'Include expired and ended registrations' }).check();
  const local = overview.getByRole('article', { name: 'Device local-device', exact: true });
  await expect(local.locator('.agent-session-card')).toHaveCount(3);
  await expect(local.locator('.agent-session-card').first()).toContainText('Reports ready');
  await expect(local.locator('.agent-session-card').nth(1)).toContainText('Report expired');
  await expect(local.locator('.agent-session-card').nth(1)).toContainText('<img src=x');
  await expect(overview.locator('.agent-session-card img')).toHaveCount(0);
  await expect(local.locator('.agent-session-card').nth(2)).toContainText('Registration ended');
  await expect(local.locator('.agent-session-card').first()).toContainText('Session and last report');
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  const picker = panel.getByRole('combobox', { name: 'Target Agent', exact: true });
  await expect(picker.locator('option').nth(2)).toHaveAttribute('disabled', '');
  await expect(picker.locator('option').nth(3)).toHaveAttribute('disabled', '');
  await picker.selectOption(session);
  await panel.getByLabel('Task title', { exact: true }).fill('Review using the chosen session');
  await panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Read and report three findings.');
  await panel.getByRole('button', { name: 'Save and delegate', exact: true }).click();
  expect(actions.find(action => action.operation === 'create')).toMatchObject({ target_session: session, node: 'remote-device' });
  await panel.getByText('Task, attempt and history', { exact: true }).click();
  await expect(panel.locator('.task-card dl')).toContainText(session);
  expect(directory.calls.every(command => command.command.operation === 'list')).toBe(true);
  for (const [width, theme] of [[375, 'dark'], [1440, 'light']] as const) {
    await page.setViewportSize({ width, height: 1000 });
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; (document.activeElement as HTMLElement)?.blur(); window.scrollTo(0, 0); }, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ fullPage: true, path: '/tmp/cranibox-agent-sessions-' + width + '-' + theme + '.png' });
  }
});

test('directory failure and elapsed leases block a selected Agent without silently rerouting', async ({ page }) => {
  await page.clock.install();
  const { directory, actions } = await agents(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  const picker = panel.getByRole('combobox', { name: 'Target Agent', exact: true });
  await picker.selectOption(session);
  await panel.getByLabel('Task title', { exact: true }).fill('Wait for this Agent');
  await panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Preserve this target until renewed.');
  const submit = panel.getByRole('button', { name: 'Save and delegate', exact: true });
  await expect(submit).toBeEnabled();
  directory.fail = true;
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Agent directory unavailable');
  await expect(submit).toBeDisabled(); await expect(picker).toHaveValue(session);
  directory.fail = false;
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(submit).toBeEnabled();
  await page.clock.fastForward(301000);
  await expect(submit).toBeDisabled(); await expect(picker).toHaveValue(session);
  await expect(picker.locator('option:checked')).toContainText('Report expired');
  expect(actions.some(action => action.operation === 'create')).toBe(false);
  directory.now += 301; directory.reports[0] = { ...directory.reports[0], updated_at: directory.now, expires_at: directory.now + 300, revision: 2 };
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(submit).toBeEnabled();
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('another-device');
  await expect(panel.getByRole('combobox', { name: 'Target Agent', exact: true })).toHaveValue('');
  await panel.getByRole('button', { name: 'Save and delegate', exact: true }).click();
  expect(actions.find(action => action.operation === 'create')).toMatchObject({ node: 'another-device' });
  expect(actions.find(action => action.operation === 'create')).not.toHaveProperty('target_session');
});

async function questionTask(page: Page) {
  const fixture = await tasks(page);
  const base = fixture.state.incoming[0];
  const question = '77777777-7777-4777-8777-777777777777';
  const current = { ...base.current, state: 'running', sender: 'local-device', recipient: 'remote-device', session, revision: 3, questions: [{ id: question, text: '<img src=x onerror="window.unsafe=true">\nWhich release should I review?', answer: null }] };
  const item = { task: { ...base.task, sender: current.sender, recipient: current.recipient }, current, history: [current], pending_answer: null as any, synced_at: Math.floor(Date.now() / 1000) };
  fixture.state.outgoing.push(item);
  const control = { fail: true };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation !== 'tasks' || command.command.operation !== 'answer') { await route.fallback(); return; }
    fixture.actions.push(command.command);
    if (control.fail) { control.fail = false; await route.fulfill({ status: 503, json: { error: 'Reply response interrupted. Retry the same question and text.' } }); return; }
    item.pending_answer = { question: command.command.question, text: command.command.text };
    await route.fulfill({ json: fixture.state });
  });
  return { ...fixture, question, item, control };
}

test('task replies retain failed text, queue explicitly and confirm after the target reconnects', async ({ page }) => {
  const { actions, item, question } = await questionTask(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('Awaiting reply');
  await expect(panel.locator('.task-question img')).toHaveCount(0);
  await expect(panel.locator('.task-question')).toContainText('<img src=x');
  const text = panel.getByRole('textbox', { name: 'Reply to worker', exact: true });
  await text.fill('Review the checked release; report evidence before deployment.');
  await panel.getByRole('button', { name: 'Save and reply', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Reply response interrupted');
  await expect(text).toHaveValue('Review the checked release; report evidence before deployment.');
  await panel.getByRole('button', { name: 'Save and reply', exact: true }).click();
  await expect(panel.locator('.task-answer-pending')).toContainText('Reply queued on this device');
  await expect(panel.getByRole('textbox', { name: 'Reply to worker', exact: true })).toHaveCount(0);
  const replies = actions.filter(action => action.operation === 'answer');
  expect(replies).toHaveLength(2); expect(replies[0]).toEqual(replies[1]);
  expect(replies[0]).toEqual({ operation: 'answer', id, dispatch, question, text: 'Review the checked release; report evidence before deployment.' });
  await page.reload();
  await expect(panel.locator('.task-answer-pending')).toBeVisible();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Review the checked release');
  item.current.questions[0].answer = item.pending_answer.text; item.pending_answer = null; item.current.revision++;
  await panel.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(panel.locator('.task-answer')).toContainText('Reply confirmed by target');
  await expect(panel.locator('.message-status')).toHaveText('Claimed by Agent');
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  expect(actions.some(action => ['exec', 'sessions', 'claim', 'finish'].includes(action.operation))).toBe(false);
});

test('question forms enforce byte limits and disappear after cancellation without sending a reply', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { actions, item } = await questionTask(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  const text = panel.getByRole('textbox', { name: 'Reply to worker', exact: true });
  await text.fill('🙂'.repeat(1025));
  await expect(text).toHaveAttribute('aria-invalid', 'true');
  await expect(panel.getByRole('button', { name: 'Save and reply', exact: true })).toBeDisabled();
  await text.fill('Use the release branch.');
  for (const [width, theme] of [[375, 'dark'], [1440, 'light']] as const) {
    await page.setViewportSize({ width, height: 1000 });
    await page.evaluate(theme => { document.documentElement.dataset.theme = theme; (document.activeElement as HTMLElement)?.blur(); window.scrollTo(0, 0); }, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const contrast = await panel.locator('.task-question p, .task-reply label, .task-reply textarea, .task-reply small, .task-reply button').evaluateAll(elements => {
      const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
      const context = canvas.getContext('2d')!;
      const luminance = (channels: Uint8ClampedArray) => Array.from(channels.slice(0, 3)).map(value => { const channel = value / 255; return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4; }).reduce((sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index], 0);
      return elements.map(element => {
        context.fillStyle = '#fff'; context.fillRect(0, 0, 1, 1);
        const ancestors: Element[] = [];
        for (let parent: Element | null = element; parent; parent = parent.parentElement) ancestors.unshift(parent);
        for (const ancestor of ancestors) { context.fillStyle = getComputedStyle(ancestor).backgroundColor; context.fillRect(0, 0, 1, 1); }
        const background = luminance(context.getImageData(0, 0, 1, 1).data);
        context.fillStyle = getComputedStyle(element).color; context.fillRect(0, 0, 1, 1);
        const foreground = luminance(context.getImageData(0, 0, 1, 1).data);
        return (Math.max(background, foreground) + 0.05) / (Math.min(background, foreground) + 0.05);
      });
    });
    expect(contrast.length).toBeGreaterThan(0);
    expect(Math.min(...contrast)).toBeGreaterThanOrEqual(4.5);
    await page.screenshot({ fullPage: true, path: '/tmp/cranibox-task-questions-' + width + '-' + theme + '.png' });
  }
  (item.current as any).state = 'cancel_requested'; item.current.revision++;
  await panel.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(text).toHaveCount(0);
  await expect(panel.locator('.message-status')).toHaveText('Awaiting cancellation');
  await expect(panel.locator('.task-question')).toContainText('No reply received.');
  expect(actions.some(action => action.operation === 'answer')).toBe(false);
});

test('a reply draft stays with its attempt when polling observes a retry already running', async ({ page }) => {
  const { actions, item } = await questionTask(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  await panel.getByRole('button', { name: 'Pause updates', exact: true }).click();
  const text = panel.getByRole('textbox', { name: 'Reply to worker', exact: true });
  await text.fill('Only for the original attempt.');
  await panel.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(text).toHaveValue('Only for the original attempt.');
  const previous = { ...item.current, state: 'failed', result: 'The worker reported a failure.', revision: 4 };
  item.current = { ...item.current, dispatch: '88888888-8888-4888-8888-888888888888', questions: [{ ...item.current.questions[0], text: 'Confirm the new attempt’s release.' }] };
  item.history = [previous, item.current];
  await panel.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await expect(panel.getByText('Confirm the new attempt’s release.', { exact: true })).toBeVisible();
  await expect(text).toHaveValue('');
  await panel.getByText('Task, attempt and history', { exact: true }).click();
  await expect(panel.locator('.task-history .task-question')).toContainText('Which release should I review?');
  await expect(panel.locator('.task-history textarea')).toHaveCount(0);
  await expect(text).toHaveCount(1);
  expect(actions.some(action => action.operation === 'answer')).toBe(false);
});


test('offline Agent snapshots stay visible without allowing delegation and recover on refresh', async ({ page }) => {
  const { directory, actions } = await agents(page);
  await page.goto('/#/tasks');
  const panel = page.getByRole('region', { name: 'Collaboration tasks', exact: true });
  const overview = panel.getByRole('region', { name: 'Agents across devices', exact: true });
  const remote = overview.getByRole('article', { name: 'Device remote-device', exact: true });
  await panel.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('remote-device');
  const picker = panel.getByRole('combobox', { name: 'Target Agent', exact: true });
  await picker.selectOption(session);
  await panel.getByLabel('Task title', { exact: true }).fill('Wait for the owner');
  await panel.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Confirm the session before delegating.');
  const save = panel.getByRole('button', { name: 'Save and delegate', exact: true });
  await expect(save).toBeEnabled();
  directory.offline = true;
  await overview.getByRole('button', { name: 'Refresh all Agents', exact: true }).click();
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(remote).toContainText('Offline snapshot');
  await expect(remote).toContainText('Build reviewer');
  await expect(remote).toContainText('Last synced:');
  await expect(remote.locator('.agent-session-card').first().getByRole('button')).toBeDisabled();
  await expect(picker).toHaveValue(session);
  await expect(picker.locator('option[value="' + session + '"]')).toHaveJSProperty('disabled', true);
  await expect(save).toBeDisabled();
  expect(actions.some(action => action.operation === 'create')).toBe(false);
  directory.offline = false;
  await overview.getByRole('button', { name: 'Refresh all Agents', exact: true }).click();
  await panel.getByRole('button', { name: 'Refresh Agents', exact: true }).click();
  await expect(remote).not.toContainText('Offline snapshot');
  await expect(save).toBeEnabled();
});

test('Agent background sync is opt-in, persists selection and fits narrow screens', async ({ page }) => {
  await agents(page);
  const actions: any[] = [];
  const state = { owner: 'local-device', revision: 0, enabled: false, interval_seconds: 60, available_nodes: ['remote-device', 'another-device'], nodes: [] as any[] };
  await page.route('**/api/peers', async route => {
    const request = route.request().postDataJSON();
    if (request.operation !== 'agent_sync') return route.fallback();
    const command = request.command; actions.push(command);
    if (command.operation === 'configure') {
      state.revision++; state.enabled = true; state.interval_seconds = command.interval_seconds;
      state.nodes = command.nodes.map((node: string) => ({ node, connection_valid: true, phase: 'online', session_count: 2, last_success: Date.now() }));
    }
    if (command.operation === 'pause') { state.revision++; state.enabled = false; state.nodes.forEach(node => { node.phase = 'paused'; }); }
    return route.fulfill({ json: state });
  });
  await page.goto('/#/tasks');
  const panel = page.locator('.agent-federation .project-sync');
  await expect(panel).toBeVisible(); expect(actions).toHaveLength(0);
  await panel.locator(':scope > summary').click();
  await expect(panel).toContainText('Target message access is required');
  await panel.getByRole('button', { name: 'Set up background sync', exact: true }).click();
  await panel.getByRole('checkbox', { name: 'remote-device', exact: true }).check();
  await panel.getByRole('combobox', { name: 'Sync interval', exact: true }).selectOption('30');
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const row = await panel.locator('.project-sync-check').first().boundingBox();
    expect(row!.height).toBeGreaterThanOrEqual(44);
    if (width === 375 || width === 1440) await panel.screenshot({ path: '/tmp/cranibox-agent-background-sync-' + width + '.png' });
  }
  await panel.getByRole('button', { name: 'Enable background sync', exact: true }).click();
  expect(actions.find(command => command.operation === 'configure')).toEqual({ operation: 'configure', revision: 0, nodes: ['remote-device'], interval_seconds: 30 });
  await expect(panel.getByRole('status')).toContainText('Saved.');
  await expect(panel.getByLabel('Device sync status')).toContainText('2 registrations');
  await page.reload(); await panel.locator(':scope > summary').click();
  await expect(panel).toContainText('Background sync enabled');
  await panel.getByRole('button', { name: 'Pause sync', exact: true }).click();
  await expect(panel.getByLabel('Device sync status')).toContainText('Paused');
});
