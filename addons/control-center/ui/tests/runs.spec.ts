import { expect, test, type Page } from '@playwright/test';

async function executions(page: Page, uncertain = false) {
  const now = Math.floor(Date.now() / 1000), actions: any[] = [];
  const state = { owner: 'local-device', executions: [] as any[], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 };
  const launches = new Map<string, any>(), outputs = new Map<string, { stdout: string; stderr: string }>();
  let fail = uncertain;
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'list') { await route.fulfill({ json: { node_id: state.owner, routes: [], grants: [] } }); return; }
    if (command.operation === 'tasks') { await route.fulfill({ json: { owner: state.owner, incoming: [], outgoing: [], capacity: 128, used: 0, max_attempts: 8 } }); return; }
    if (command.operation === 'agent_sessions') { await route.fulfill({ json: { owner: state.owner, sessions: [], capacity: 64, observed_at: now, lease_secs: 300 } }); return; }
    if (command.operation !== 'executions') { await route.fulfill({ json: {} }); return; }
    const action = command.command; actions.push(action);
    if (action.operation === 'prepare_agent') {
      const agent = action.agent;
      await route.fulfill({ json: { adapter: agent.adapter, access: agent.access, available: true, launch: { name: agent.name, harness: agent.adapter === 'codex' ? 'codex' : 'claude-code', program: '/usr/local/bin/' + (agent.adapter === 'codex' ? 'codex' : 'claude'), args: ['fixture-noninteractive'], cwd: agent.cwd, stdin: agent.prompt } } }); return;
    }
    if (action.operation === 'start') {
      if (!launches.has(action.id)) {
        launches.set(action.id, action.launch); outputs.set(action.id, { stdout: '', stderr: '' });
        state.executions.push({ id: action.id, session_id: '44444444-4444-4444-8444-444444444444', name: action.launch.name, harness: action.launch.harness, state: 'queued', created_at: now, updated_at: now, pid: null, exit_code: null, error: null, stdout_truncated: false, stderr_truncated: false });
      }
      if (fail) { fail = false; await route.fulfill({ status: 503, json: { error: 'Launch response interrupted. Retry the same ID.' } }); return; }
    }
    if (action.operation === 'get') {
      const output = outputs.get(action.id)!;
      await route.fulfill({ json: { execution: state.executions.find(run => run.id === action.id), launch: launches.get(action.id), stdout: { text: output.stdout.slice(action.stdout_offset || 0), next_offset: output.stdout.length }, stderr: { text: output.stderr.slice(action.stderr_offset || 0), next_offset: output.stderr.length } } }); return;
    }
    if (action.operation === 'stop') state.executions.find(run => run.id === action.id).state = 'stopping';
    if (action.operation === 'archive') state.executions = state.executions.filter(run => run.id !== action.id);
    state.used = launches.size;
    await route.fulfill({ json: state });
  });
  return { state, actions, outputs };
}

async function prepare(page: Page) {
  await page.goto('/#/tasks');
  const panel = page.locator('.managed-runs');
  await panel.locator(':scope > summary').focus(); await page.keyboard.press('Enter');
  await expect(panel.getByText('No background runs yet.', { exact: false })).toBeVisible();
  await panel.locator('.managed-run-launch > summary').click();
  await panel.getByRole('combobox', { name: 'Agent / program', exact: true }).selectOption('custom');
  await panel.getByLabel('Run name', { exact: true }).fill('Build review');
  await panel.getByLabel('Executable', { exact: true }).fill('review-agent');
  await panel.getByLabel('Absolute working directory', { exact: true }).fill('/home/user/project');
  await panel.getByRole('textbox', { name: 'Arguments (JSON string array)', exact: true }).fill('["--noninteractive","review the patch"]');
  return panel;
}

async function remoteExecutions(page: Page) {
  await executions(page);
  const calls: any[] = [], launches = new Map<string, any>();
  const catalogs = Object.fromEntries(['remote-a', 'remote-b'].map(owner => [owner, { owner, executions: [] as any[], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 }]));
  let revoked = false, uncertain = true, hold = false, release = () => {};
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'list') return route.fulfill({ json: { node_id: 'local-device', routes: Object.keys(catalogs).map(node => ({ node, url: 'http://10.0.0.2:9091', allow_private_http: true })), grants: [] } });
    if (command.operation !== 'executions' || !command.node) return route.fallback();
    calls.push(command);
    const catalog = catalogs[command.node], action = command.command;
    if (revoked && command.node === 'remote-a') return route.fulfill({ status: 403, json: { error: 'Execution access revoked' } });
    if (action.operation === 'list' && command.node === 'remote-a' && hold) { hold = false; await new Promise<void>(resolve => { release = resolve; }); }
    if (action.operation === 'start') {
      if (!launches.has(action.id)) {
        launches.set(action.id, action.launch);
        catalog.executions.push({ id: action.id, session_id: '44444444-4444-4444-8444-444444444444', name: action.launch.name, harness: action.launch.harness, state: 'running', created_at: 1, updated_at: 1, pid: 123, exit_code: null, error: null, stdout_truncated: false, stderr_truncated: false });
      }
      if (uncertain) { uncertain = false; return route.fulfill({ status: 503, json: { error: 'Remote response interrupted' } }); }
    }
    if (action.operation === 'get') return route.fulfill({ json: { execution: catalog.executions.find(run => run.id === action.id), launch: launches.get(action.id), stdout: { text: 'Remote private output'.slice(action.stdout_offset || 0), next_offset: 21 }, stderr: { text: '', next_offset: 0 } } });
    return route.fulfill({ json: catalog });
  });
  return { calls, revoke: () => { revoked = true; }, holdNext: () => { hold = true; }, release: () => release() };
}

test('remote runs use the selected device, retry the same ID and clear output on revocation', async ({ page }) => {
  const remote = await remoteExecutions(page);
  const panel = await prepare(page), target = panel.getByRole('combobox', { name: 'Background execution device', exact: true });
  await target.selectOption('remote-a');
  await panel.locator('.managed-run-launch > summary').click();
  await expect(panel.getByLabel('Run name', { exact: true })).toHaveValue('');
  await panel.getByRole('combobox', { name: 'Agent / program', exact: true }).selectOption('custom');
  await panel.getByLabel('Run name', { exact: true }).fill('Remote review');
  await panel.getByLabel('Executable', { exact: true }).fill('/usr/bin/review-agent');
  await panel.getByLabel('Absolute working directory', { exact: true }).fill('/home/remote/project');
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.locator('.managed-run-review')).toContainText('remote-a');
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Remote response interrupted');
  await expect(panel.getByRole('alert')).toContainText('Request ID:');
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toHaveCount(1);
  const starts = remote.calls.filter(call => call.command.operation === 'start');
  expect(starts).toHaveLength(2); expect(starts[0]).toEqual(starts[1]); expect(starts[0].node).toBe('remote-a');
  await panel.getByRole('button', { name: 'View output', exact: true }).click();
  await expect(panel.getByLabel('stdout', { exact: true })).toContainText('Remote private output');
  await target.selectOption('remote-b');
  await expect(panel.locator('.managed-run-output')).toHaveCount(0);
  await expect(panel.locator('.managed-run-card')).toHaveCount(0);
  await target.selectOption('remote-a');
  await expect(panel.locator('.managed-run-card')).toHaveCount(1);
  await panel.getByRole('button', { name: 'View output', exact: true }).click();
  await expect(panel.getByLabel('stdout', { exact: true })).toBeVisible();
  remote.revoke();
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Execution access revoked');
  await expect(panel.locator('.managed-run-output')).toHaveCount(0);
  await expect(panel.locator('.managed-run-card')).toHaveCount(0);
});

test('a late response cannot replace another device and the selector fits phone themes', async ({ page }) => {
  const remote = await remoteExecutions(page);
  await page.goto('/#/tasks');
  const panel = page.locator('.managed-runs');
  await panel.locator(':scope > summary').click();
  const target = panel.getByRole('combobox', { name: 'Background execution device', exact: true });
  await target.selectOption('remote-a');
  await expect(panel.locator('.direct-actions > small')).toContainText('remote-a');
  remote.holdNext();
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Refresh background runs', exact: true })).toBeDisabled();
  await target.selectOption('remote-b');
  await expect(panel.locator('.direct-actions > small')).toContainText('remote-b');
  remote.release();
  await expect(panel.locator('.direct-actions > small')).toContainText('remote-b');
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const box = await target.boundingBox(); expect(box!.height).toBeGreaterThanOrEqual(44);
    }
  }
});

test('explicit launch review preserves the run ID after an uncertain response', async ({ page }) => {
  const { state, actions } = await executions(page, true);
  const panel = await prepare(page);
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.locator('.managed-run-review')).toContainText('/home/user/project');
  expect(actions.filter(action => action.operation === 'start')).toHaveLength(0);
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Launch response interrupted');
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('Launch request saved');
  const starts = actions.filter(action => action.operation === 'start');
  expect(starts).toHaveLength(2); expect(starts[1]).toEqual(starts[0]); expect(state.executions).toHaveLength(1);
  expect(starts[0].launch).toMatchObject({ program: 'review-agent', cwd: '/home/user/project', args: ['--noninteractive', 'review the patch'] });
  await page.reload(); await panel.locator(':scope > summary').click();
  await expect(panel.locator('.managed-run-card')).toContainText('Build review');
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('review the patch');
});

test('Agent presets prepare on the device, review instructions and preserve an uncertain launch ID', async ({ page }) => {
  const { actions, state } = await executions(page, true);
  await page.goto('/#/tasks');
  const panel = page.locator('.managed-runs');
  await panel.locator(':scope > summary').click();
  await panel.locator('.managed-run-launch > summary').click();
  await expect(panel.getByRole('combobox', { name: 'Agent / program', exact: true })).toHaveValue('codex');
  await expect(panel.getByLabel('Executable', { exact: true })).toHaveCount(0);
  await panel.getByLabel('Run name', { exact: true }).fill('Project review');
  await panel.getByLabel('Absolute working directory', { exact: true }).fill('/home/user/project');
  await panel.getByRole('textbox', { name: 'Task instructions', exact: true }).fill('Review the project\nKeep the public API unchanged.');
  await panel.getByRole('combobox', { name: 'Execution access', exact: true }).selectOption('workspace_write');
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.locator('.managed-run-review')).toContainText('Keep the public API unchanged.');
  expect(actions.filter(action => action.operation === 'start')).toHaveLength(0);
  expect(actions.find(action => action.operation === 'prepare_agent').agent).toMatchObject({ adapter: 'codex', access: 'workspace_write', cwd: '/home/user/project' });
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Launch response interrupted');
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Launch response interrupted');
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toHaveCount(1);
  const starts = actions.filter(action => action.operation === 'start');
  expect(starts[0]).toEqual(starts[1]); expect(state.executions).toHaveLength(1);
  expect(starts[0].launch.stdin).toContain('Keep the public API unchanged.');
  expect(JSON.stringify(starts[0].launch.args)).not.toContain('Keep the public API unchanged.');
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Keep the public API unchanged.');
});

test('missing Agent CLI cannot launch and review errors preserve the task on mobile', async ({ page }) => {
  const { actions } = await executions(page);
  await page.route('**/api/peers', async (route, request) => {
    const action = request.postDataJSON().command;
    if (action?.operation !== 'prepare_agent') { await route.fallback(); return; }
    await route.fulfill({ json: { adapter: 'claude_code', access: 'edit_files', available: false, launch: { name: action.agent.name, harness: 'claude-code', program: 'claude', args: [], cwd: action.agent.cwd, stdin: action.agent.prompt } } });
  });
  await page.goto('/#/tasks');
  const panel = page.locator('.managed-runs');
  await panel.locator(':scope > summary').click();
  await panel.locator('.managed-run-launch > summary').click();
  await panel.getByRole('combobox', { name: 'Agent / program', exact: true }).selectOption('claude_code');
  await panel.getByLabel('Run name', { exact: true }).fill('Update docs');
  await panel.getByLabel('Absolute working directory', { exact: true }).fill('/home/user/project');
  await panel.getByRole('textbox', { name: 'Task instructions', exact: true }).fill('更新说明，保留中文。');
  await panel.getByRole('combobox', { name: 'Execution access', exact: true }).selectOption('edit_files');
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('CLI was not found');
  await expect(panel.getByRole('button', { name: 'Confirm launch', exact: true })).toBeDisabled();
  for (const width of [375, 768, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await panel.getByRole('button', { name: 'Back to edit', exact: true }).click();
  await expect(panel.getByRole('textbox', { name: 'Task instructions', exact: true })).toHaveValue('更新说明，保留中文。');
  await page.route('**/api/peers', route => {
    if (route.request().postDataJSON().command?.operation === 'prepare_agent') return route.fulfill({ status: 400, json: { error: 'Working directory is unavailable' } });
    return route.fallback();
  });
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Working directory is unavailable');
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Working directory is unavailable');
  expect(actions.some(action => action.operation === 'start')).toBe(false);
});

test('plain output can be resumed and stopping waits for the node result', async ({ page }) => {
  const { state, actions, outputs } = await executions(page);
  const panel = await prepare(page);
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await panel.getByRole('button', { name: 'Confirm launch', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toHaveCount(1);
  const id = state.executions[0].id;
  state.executions[0].state = 'running'; outputs.set(id, { stdout: '<img src=x onerror="window.unsafe=true">\nfirst', stderr: 'diagnostic' });
  await panel.getByRole('button', { name: 'View output', exact: true }).click();
  await expect(panel.getByLabel('stdout', { exact: true })).toContainText('first');
  await expect(panel.locator('pre img')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
  outputs.get(id)!.stdout += '\nsecond';
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByLabel('stdout', { exact: true })).toHaveText('<img src=x onerror="window.unsafe=true">\nfirst\nsecond');
  page.once('dialog', dialog => dialog.dismiss());
  await panel.getByRole('button', { name: 'Stop run', exact: true }).click();
  expect(actions.filter(action => action.operation === 'stop')).toHaveLength(0);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Stop run', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toContainText('Stopping');
  await expect(panel.getByRole('button', { name: 'Archive run', exact: true })).toHaveCount(0);
  state.executions[0].state = 'cancelled';
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Archive run', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toHaveCount(0);
  await expect(panel.locator('.managed-run-output')).toHaveCount(0);
});

test('invalid arguments preserve the draft and layouts fit phones and desktop themes', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const { actions } = await executions(page);
  const panel = await prepare(page);
  await panel.getByRole('textbox', { name: 'Arguments (JSON string array)', exact: true }).fill('{"command":"invalid"}');
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Arguments must be a JSON string array');
  await expect(panel.getByLabel('Run name', { exact: true })).toHaveValue('Build review');
  expect(actions.filter(action => action.operation === 'start')).toHaveLength(0);
  await panel.getByRole('textbox', { name: 'Arguments (JSON string array)', exact: true }).fill('["--noninteractive"]');
  await panel.getByRole('button', { name: 'Review launch', exact: true }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      if (width === 375) {
        await page.evaluate(() => { (document.activeElement as HTMLElement)?.blur(); window.scrollTo(0, 0); });
        await page.screenshot({ path: '/tmp/cranibox-managed-runs-' + theme + '.png', fullPage: true });
      }
    }
  }
  await page.route('**/api/peers', route => route.fulfill({ status: 503, json: { error: 'Local service unavailable' } }));
  await panel.getByRole('button', { name: 'Refresh background runs', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Local service unavailable');
  await expect(panel.locator('.managed-run-review')).toContainText('/home/user/project');
});
