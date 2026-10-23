import { openNetworkSettings } from './network-page';
import { expect, test } from '@playwright/test';

test('handoff review starts only the destination and recovers its fixed run after reopening', async ({ page }) => {
  const id = '77777777-7777-4777-8777-777777777777', replica = '88888888-8888-4888-8888-888888888888', actions: any[] = [], errors: string[] = [];
  const destination = '/received/cranibox-' + id + '/project';
  let job: any, launch: any, started = false;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'source' }, { node: 'target' }] } });
    if (command.operation === 'projects') return route.fulfill({ json: { owner: command.node, revision: 1, permissions: { exec: true, projects: true }, projects: command.node === 'source' ? [{ id: 'handoff-project', name: 'Portable Agent work', replicas: [{ id: replica, node: 'source', path: '/source/project', sessions: [] }] }] : [] } });
    if (command.operation === 'copy_list') return route.fulfill({ json: { copies: job ? [job] : [] } });
    if (command.operation === 'copy_prepare') job = { id, source: 'source', target: 'target', target_root: '/received', digest: 'b'.repeat(64), state: 'prepared', created_at: 1792252800000, summary: { name: 'Portable Agent work', files: [{ path: 'main.rs', size: 32 }, { path: '.cranibox/handoffs/' + id + '.md', size: 200 }], directories: ['.cranibox', '.cranibox/handoffs'], skipped: ['.env'], bytes: 232, handoff: command.handoff, handoff_path: '.cranibox/handoffs/' + id + '.md' } };
    if (command.operation === 'copy_commit') job = { ...job, state: 'completed', result: { path: destination } };
    if (command.operation === 'copy_agent_prepare') {
      launch = { name: 'Portable Agent work · handoff', program: '/usr/bin/codex', args: ['exec', '--sandbox', 'read-only', '-'], cwd: destination, stdin: 'Verify the copied files.\n' + job.summary.handoff.goal };
      return route.fulfill({ json: { execution_id: id, available: true, digest: 'c'.repeat(64), launch } });
    }
    if (command.operation === 'copy_agent_start') { started = true; return route.fulfill({ status: 400, json: { error: 'Connection lost; launch outcome unknown' } }); }
    if (command.operation === 'copy_agent_status') {
      if (!started) return route.fulfill({ status: 400, json: { error: 'run unavailable' } });
      const previous = command.stdout_offset || 0, first = 'First chunk 中文\n', last = 'FINAL_OUTPUT\n', end = Buffer.byteLength(first) + Buffer.byteLength(last);
      return route.fulfill({ json: { owner: 'target', execution: { id, state: 'succeeded', exit_code: 0 }, launch, stdout: { text: !previous ? first : previous < end ? last : '', next_offset: !previous ? Buffer.byteLength(first) : end }, stderr: { text: '', next_offset: 0 } } });
    }
    return route.fulfill({ json: job });
  });
  await page.goto('/#/projects');
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Copy destination', exact: true }).selectOption('target');
  await panel.getByRole('button', { name: 'Include Agent handoff (optional)', exact: true }).click();
  const goal = panel.getByRole('textbox', { name: 'Goal and constraints (required)', exact: true });
  await goal.fill('文'.repeat(683));
  await expect(panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true })).toBeDisabled();
  await goal.fill('Continue the pending project work without repeating completed actions.');
  await panel.getByRole('textbox', { name: 'Progress and decisions', exact: true }).fill('The source Agent reviewed the existing files.');
  await panel.getByRole('textbox', { name: 'Next steps', exact: true }).fill('Implement the pending endpoint.');
  await panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true }).click();
  await panel.getByText('Review Agent handoff', { exact: true }).click();
  await expect(panel.locator('.projects-handoff-review')).toContainText('Implement the pending endpoint.');
  expect(actions.some(a => a.operation.startsWith('copy_agent_'))).toBe(false);
  await panel.getByRole('checkbox').check();
  await panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true }).click();
  const continuation = panel.getByRole('region', { name: 'Continue Agent on destination', exact: true });
  await expect(continuation).toContainText(destination);
  await continuation.getByRole('button', { name: 'Review destination Agent', exact: true }).click();
  await expect(continuation.getByLabel('Full continuation instructions', { exact: true })).toContainText('Continue the pending project work');
  expect(actions.some(a => a.operation === 'copy_agent_start')).toBe(false);
  await continuation.getByRole('button', { name: 'Confirm start on destination', exact: true }).click();
  await expect(continuation.getByRole('alert')).toContainText('outcome unknown');
  expect(actions.find(a => a.operation === 'copy_agent_start')).toEqual({ operation: 'copy_agent_start', id, adapter: 'codex', access: 'read_only', digest: 'c'.repeat(64) });
  await page.reload();
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'View copy record', exact: true }).selectOption(id);
  await continuation.getByRole('button', { name: 'Find existing continuation', exact: true }).click();
  await expect(continuation.getByLabel('Continuation stdout', { exact: true })).toContainText('First chunk 中文');
  await expect(continuation.getByLabel('Continuation stdout', { exact: true })).toContainText('FINAL_OUTPUT');
  await expect(continuation).toContainText('Process exited successfully');
  expect(actions.filter(a => a.operation === 'copy_agent_start')).toHaveLength(1);
  expect(actions.filter(a => a.operation === 'copy_agent_status').some(a => a.stdout_offset === Buffer.byteLength('First chunk 中文\n'))).toBe(true);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await continuation.scrollIntoViewIfNeeded();
      await page.screenshot({ path: '/tmp/cranibox-project-handoff-' + width + '-' + theme + '.png', animations: 'disabled' });
    }
  }
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('pending project work');
  expect(errors).toEqual([]);
});

test('a missing destination CLI prevents confirming a continuation', async ({ page }) => {
  const id = '99999999-9999-4999-8999-999999999999', actions: any[] = [];
  const job = { id, source: 'source', target: 'target', digest: 'a'.repeat(64), state: 'completed', created_at: 1792252800000, target_root: '/received', result: { path: '/received/project' }, summary: { name: 'Copied project', files: [], directories: [], skipped: [], bytes: 0, handoff: { goal: 'Finish work', progress: '', next_steps: '', verification: '' }, handoff_path: '.cranibox/handoffs/' + id + '.md' } };
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { routes: [] } });
    if (command.operation === 'copy_agent_prepare') return route.fulfill({ json: { execution_id: id, available: false, digest: 'b'.repeat(64), launch: { program: 'claude', args: ['--print'], cwd: '/received/project', stdin: 'Finish work' } } });
    return route.fulfill({ json: { copies: [job] } });
  });
  await page.goto('/#/projects');
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'View copy record', exact: true }).selectOption(id);
  const continuation = panel.getByRole('region', { name: 'Continue Agent on destination', exact: true });
  await continuation.getByRole('combobox', { name: 'Continuation Agent', exact: true }).selectOption('claude_code');
  await continuation.getByRole('button', { name: 'Review destination Agent', exact: true }).click();
  await expect(continuation.getByRole('alert')).toContainText('CLI is missing');
  await expect(continuation.getByRole('button', { name: 'Confirm start on destination', exact: true })).toBeDisabled();
  expect(actions.some(a => a.operation === 'copy_agent_start')).toBe(false);
});

test('the Web host can send and receive project copies using its own identity', async ({ page }) => {
  await openNetworkSettings(page);
  const host = await page.evaluate(async () => (await (await fetch('/api/peers', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Workspace-CSRF': (window as any).workspaceWeb.csrf }, body: JSON.stringify({ operation: 'identity' }) })).json()).node_id);
  const localReplica = '55555555-5555-4555-8555-555555555555', remoteReplica = '66666666-6666-4666-8666-666666666666';
  const actions: any[] = [];
  let copied = false, job: any;
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [{ id: 'host-copy', name: 'Host working files', replicas: [{ id: localReplica, node: host, path: '/projects/host', sessions: [] }] }] } }));
  await page.route('**/api/peers', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { node_id: host, routes: [{ node: 'target' }], grants: [] } });
    if (command.operation === 'projects') return route.fulfill({ json: { owner: 'target', revision: 1, permissions: { exec: true, projects: true, terminals: false }, projects: copied ? [{ id: 'host-copy', name: 'Host working files', replicas: [{ id: remoteReplica, node: 'target', path: '/projects/remote', sessions: [] }] }] : [] } });
    if (command.operation === 'copy_list') return route.fulfill({ json: { copies: job ? [job] : [] } });
    if (command.operation === 'copy_prepare') job = { id: 'host-job', source: command.source, target: command.target, target_root: '/projects/received', digest: 'a'.repeat(64), state: 'prepared', created_at: 1792180800000, summary: { name: 'Host working files', files: [{ path: 'main.rs', size: 32 }], directories: [], skipped: [], bytes: 32 } };
    if (command.operation === 'copy_commit') { copied = true; job = { ...job, state: 'completed', result: { path: '/projects/remote' } }; }
    return route.fulfill({ json: job });
  });
  await page.goto('/#/projects');
  await expect(page.getByRole('heading', { name: 'Host working files', exact: true })).toBeVisible();
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Copy destination', exact: true }).selectOption('target');
  await panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true }).click();
  expect(actions.find(a => a.operation === 'copy_prepare')).toEqual({ operation: 'copy_prepare', source: host, target: 'target', replica_id: localReplica });
  await expect(panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true })).toBeDisabled();
  await panel.getByRole('checkbox').check();
  await panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true }).click();
  const working = page.getByRole('combobox', { name: 'Working copy', exact: true });
  await expect(working.locator('option')).toHaveCount(2);
  await working.selectOption('direct:target/' + remoteReplica);
  await panel.getByRole('combobox', { name: 'Copy destination', exact: true }).selectOption(host);
  await panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true }).click();
  expect(actions.filter(a => a.operation === 'copy_prepare')[1]).toEqual({ operation: 'copy_prepare', source: 'target', target: host, replica_id: remoteReplica });
  await expect(panel.getByRole('checkbox')).not.toBeChecked();
  expect(actions.some(a => a.operation === 'connect' || a.operation === 'grant')).toBe(false);
});

test('reviewed project copies restore history and recover unknown outcomes without resending', async ({ page }) => {
  const actions: any[] = [], errors: string[] = [];
  const replica = '33333333-3333-4333-8333-333333333333';
  let job: any;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', async route => {
    const action = route.request().postDataJSON(); actions.push(action);
    if (action.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'source' }, { node: 'target' }] } });
    if (action.operation === 'projects') return route.fulfill({ json: { owner: action.node, revision: 1, permissions: { exec: true, projects: true, terminals: false }, projects: action.node === 'source' ? [{ id: 'project', name: 'Cranibox source', replicas: [{ id: replica, node: 'source', path: '/home/me/cranibox', sessions: [] }] }] : [] } });
    if (action.operation === 'copy_list') return route.fulfill({ json: { copies: job ? [job] : [] } });
    if (action.operation === 'copy_prepare') job = { id: 'copy-1', source: 'source', target: 'target', target_root: '/projects/received', digest: 'a'.repeat(64), state: 'prepared', created_at: 1792180800000, summary: { name: 'Cranibox source', files: [{ path: 'src/a-very-long-uncommitted-source-file-name.rs', size: 32768 }], directories: ['src'], skipped: ['.env', 'node_modules'], bytes: 32768 } };
    if (action.operation === 'copy_commit') job = { ...job, state: 'uncertain', error: 'Network disconnected. Check the target receipt.' };
    if (action.operation === 'copy_status') job = { ...job, state: 'completed', error: null, result: { path: '/projects/received/cranibox-copy/project' } };
    return route.fulfill({ json: job });
  });
  await page.goto('/#/projects');
  await expect(page.getByRole('heading', { name: 'Cranibox source', exact: true })).toBeVisible();
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Copy destination', exact: true }).selectOption('target');
  await panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true }).click();
  expect(actions.find(a => a.operation === 'copy_prepare')).toEqual({ operation: 'copy_prepare', source: 'source', target: 'target', replica_id: replica });
  await expect(panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true })).toBeDisabled();
  await panel.getByText('Review file manifest', { exact: true }).click();
  await expect(panel.locator('details').first()).toContainText('uncommitted-source-file-name.rs');
  await panel.getByText('Skipped entries: 2', { exact: true }).click();
  await expect(panel).toContainText('node_modules');
  expect(actions.some(a => a.operation === 'copy_commit')).toBe(false);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['dark', 'light']) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      await page.evaluate(() => window.scrollTo(0, 0));
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: '/tmp/cranibox-project-copy-' + width + '-' + theme + '.png', fullPage: true, animations: 'disabled' });
    }
  }
  await page.reload();
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'View copy record', exact: true }).selectOption('copy-1');
  await expect(panel.getByRole('checkbox')).not.toBeChecked();
  await panel.getByRole('checkbox').check();
  await panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Network disconnected');
  await panel.getByRole('button', { name: 'Check target receipt', exact: true }).click();
  await expect(panel.locator('.projects-copy-review')).toContainText('Completed');
  await expect(panel).toContainText('/projects/received/cranibox-copy/project');
  expect(actions.filter(a => a.operation === 'copy_commit')).toHaveLength(1);
  expect(actions.some(a => a.operation === 'sessions')).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('/projects/received');
  expect(errors).toEqual([]);
});

test('the copy receiving directory retains invalid input and can be disabled locally', async ({ page }) => {
  const catalog = { node_id: 'local', routes: [], grants: [], copy_root: null as string | null };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'copy_root') {
      if (command.path === '/missing') return route.fulfill({ status: 400, json: { error: 'copy root is unavailable' } });
      catalog.copy_root = command.path;
    }
    return route.fulfill({ json: catalog });
  });
  await openNetworkSettings(page);
  const form = page.getByRole('form', { name: 'Project copy receiving directory', exact: true });
  const field = form.getByLabel('Project copy receiving directory', { exact: true });
  await field.fill('/missing');
  await form.getByRole('button', { name: 'Save receiving directory', exact: true }).click();
  await expect(page.locator('.direct-devices').getByRole('alert')).toContainText('copy root is unavailable');
  await expect(field).toHaveValue('/missing');
  await field.fill('/projects');
  await form.getByRole('button', { name: 'Save receiving directory', exact: true }).click();
  await expect(form.getByRole('status')).toHaveText('Receiving directory saved.');
  await field.fill('');
  await form.getByRole('button', { name: 'Save receiving directory', exact: true }).click();
  await expect(form.getByRole('status')).toHaveText('Receiving project copies disabled.');
});

test('unavailable copy records can be removed without exposing old remote paths', async ({ page }) => {
  const id = '44444444-4444-4444-8444-444444444444', actions: any[] = [];
  let unavailable = [id];
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { routes: [] } });
    if (command.operation === 'copy_discard') unavailable = [];
    return route.fulfill({ json: { copies: [], unavailable } });
  });
  await page.goto('/#/projects');
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await expect(panel.locator('.projects-copy-unavailable')).toContainText(id);
  await expect(panel.locator('.projects-copy-review')).toHaveCount(0);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Remove unavailable record ' + id, exact: true }).click();
  await expect(panel.locator('.projects-copy-unavailable')).toHaveCount(0);
  expect(actions.filter(a => a.operation === 'copy_discard')).toEqual([{ operation: 'copy_discard', id }]);
  expect(actions.some(a => a.operation === 'copy_commit')).toBe(false);
});
