import { expect, test, type Page } from '@playwright/test';

const runId = '11111111-1111-4111-8111-111111111111';
async function fixture(page: Page, uncertain = false) {
  const now = Math.floor(Date.now() / 1000), actions: any[] = [];
  const state: any = { owner: 'local', incoming: [], outgoing: [], capacity: 128, used: 0, max_attempts: 8, collaborations: [], collaboration_used: 0, collaboration_capacity: 32 };
  const control = { uncertain, uncertainTask: false, state, actions };
  const stats = () => { for (const run of state.collaborations) { const items = state.outgoing.filter((item: any) => item.task.collaboration?.id === run.scope.id); run.stats = { total: items.length, active: items.filter((item: any) => !['succeeded', 'failed', 'cancelled'].includes(item.current.state)).length, succeeded: items.filter((item: any) => item.current.state === 'succeeded').length, failed: items.filter((item: any) => item.current.state === 'failed').length, cancelled: items.filter((item: any) => item.current.state === 'cancelled').length, questions: 0, blocked: 0, waiting: items.filter((item: any) => item.dependencies?.state === 'waiting').length }; } };
  const add = (id = runId, name = 'Release review') => { const run = { scope: { id, owner: 'local', name, goal: 'Review both devices. <img src=x onerror="window.unsafe=true">' }, members: ['peer-a', 'peer-b'], state: 'open', revision: 1, created_at: now, result: null, stats: { total: 0, active: 0, succeeded: 0, failed: 0, cancelled: 0, questions: 0, blocked: 0 } }; state.collaborations.push(run); state.collaboration_used++; return run; };
  const task = (scope: any, incoming = false) => { const id = crypto.randomUUID(), dispatch = crypto.randomUUID(); const item = { task: { id, sender: incoming ? scope.owner : 'local', recipient: incoming ? 'local' : 'peer-a', title: incoming ? 'Incoming from ' + scope.owner : 'Review files', description: 'Report evidence', collaboration: scope, created_at: now, expires_at: now + 604800 }, current: { task: id, dispatch, sender: incoming ? scope.owner : 'local', recipient: incoming ? 'local' : 'peer-a', state: 'available', session: null, result: null, progress: null, updated_at: now, revision: 1 }, history: [] as any[], grant_active: true }; item.history.push(item.current); state[incoming ? 'incoming' : 'outgoing'].push(item); state.used++; stats(); return item; };
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON();
    if (command.operation === 'list') return route.fulfill({ json: { node_id: 'local', routes: ['peer-a', 'peer-b', 'outsider'].map(node => ({ node, url: 'https://' + node + '.example.com', allow_private_http: false })), grants: [] } });
    if (command.operation === 'agent_sessions') return route.fulfill({ json: { owner: command.node || 'local', observed_at: now, lease_secs: 300, capacity: 64, sessions: [] } });
    if (command.operation === 'executions') return route.fulfill({ json: { owner: 'local', executions: [], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 } });
    if (command.operation !== 'tasks') return route.fulfill({ status: 400 });
    const action = command.command; actions.push(action);
    if (action.operation === 'collaboration') {
      const update = action.command;
      let run = state.collaborations.find((run: any) => run.scope.id === update.id);
      if (update.operation === 'create') {
        if (!run) { run = add(update.id, update.name); run.scope.goal = update.goal; run.members = update.members; if (update.project_id) run.scope.project_id = update.project_id; }
        if (control.uncertain) { control.uncertain = false; return route.fulfill({ status: 503, json: { error: 'Response interrupted; retry unchanged.' } }); }
      } else {
        if (run.revision !== update.revision) return route.fulfill({ status: 400, json: { error: 'Collaboration changed; refresh its revision.' } });
        if (update.operation === 'stop') { run.state = 'stopping'; for (const item of state.outgoing) if (item.task.collaboration?.id === run.scope.id) item.cancel_requested = true; }
        if (update.operation === 'close') { run.state = 'closed'; run.result = update.result; }
        if (update.operation === 'archive') { state.collaborations = state.collaborations.filter((other: any) => other !== run); state.outgoing = state.outgoing.filter((item: any) => item.task.collaboration?.id !== run.scope.id); }
        run.revision++;
      }
    }
    if (action.operation === 'create' && !state.outgoing.some((item: any) => item.task.id === action.id)) {
      const run = state.collaborations.find((run: any) => run.scope.id === action.run_id), item: any = task(run?.scope);
      item.task = { ...item.task, id: action.id, title: action.title, description: action.description, recipient: action.node }; item.current.task = action.id; item.current.dispatch = action.dispatch;
      if (action.depends_on?.length) { item.current.state = 'queued'; item.current.revision = 0; item.dependencies = { state: 'waiting', prerequisites: action.depends_on.map((id: string) => { const parent = state.outgoing.find((other: any) => other.task.id === id); return { id, title: parent.task.title, node: parent.task.recipient, current: parent.current }; }) }; }
    }
    if (action.operation === 'create' && control.uncertainTask) { control.uncertainTask = false; return route.fulfill({ status: 503, json: { error: 'Response interrupted; retry unchanged.' } }); }
    if (action.operation === 'cancel') { const item = state.outgoing.find((item: any) => item.task.id === action.id); item.cancel_requested = true; if (item.dependencies?.state === 'waiting') { item.dependencies.state = 'cancelled_before_dispatch'; item.current.state = 'cancelled'; item.current.result = 'Cancelled locally before delegation; this attempt was never sent.'; item.current.revision++; } }
    stats(); return route.fulfill({ json: state });
  });
  const open = async () => { await page.goto('/#/tasks'); await page.getByRole('button', { name: 'Pause updates', exact: true }).click(); };
  return { ...control, control, add, task, stats, open, panel: page.getByRole('region', { name: 'Collaboration runs', exact: true }) };
}

test('creation keeps an uncertain request ID and member selection limits task targets without losing drafts', async ({ page }) => {
  const { control, open, panel } = await fixture(page, true);
  await page.addInitScript(() => Object.defineProperty(crypto, 'randomUUID', { value: undefined }));
  await open(); await panel.getByRole('button', { name: 'New collaboration', exact: true }).click();
  const form = panel.getByRole('form', { name: 'Create collaboration run', exact: true });
  await expect(form.getByLabel('Collaboration name', { exact: true })).toBeFocused();
  await form.getByLabel('Collaboration name', { exact: true }).fill('Review two devices');
  await form.getByLabel('Shared goal and completion criteria', { exact: true }).fill('Report evidence from the same project.');
  await form.getByRole('checkbox', { name: 'peer-a', exact: true }).check();
  await form.getByRole('checkbox', { name: 'peer-b', exact: true }).check();
  await form.getByRole('button', { name: 'Create collaboration', exact: true }).click();
  await expect(panel.getByRole('alert')).toContainText('Response interrupted');
  await expect(form.getByLabel('Collaboration name', { exact: true })).toHaveValue('Review two devices');
  await form.getByRole('button', { name: 'Create collaboration', exact: true }).click();
  const creates = control.actions.filter(action => action.operation === 'collaboration'); expect(creates).toHaveLength(2); expect(creates[0]).toEqual(creates[1]); expect(control.state.collaborations).toHaveLength(1);
  const title = page.getByLabel('Task title', { exact: true }); await title.fill('Preserve this draft'); await page.getByLabel('Description and completion criteria', { exact: true }).fill('Check both build logs.');
  await page.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('outsider');
  await panel.getByRole('button', { name: 'Add delegation', exact: true }).click();
  await expect(title).toBeFocused(); await expect(title).toHaveValue('Preserve this draft');
  const target = page.getByRole('combobox', { name: 'Target device', exact: true }); await expect(target).toHaveValue(''); expect(await target.locator('option').allTextContents()).toEqual(['Choose a connected device', 'peer-a', 'peer-b']);
  await target.selectOption('peer-a'); await page.getByRole('button', { name: 'Save and delegate', exact: true }).click();
  expect(control.actions.find(action => action.operation === 'create').run_id).toBe(creates[0].command.id);
  await expect(page.getByRole('article', { name: 'Task Preserve this draft', exact: true })).toBeVisible();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Report evidence');
});

test('cancellation requires confirmation and final reports before reviewed closure and whole-run archive', async ({ page }) => {
  const { add, task, open, panel, control } = await fixture(page); const run = add(), item = task(run.scope);
  await open(); const card = panel.getByRole('article', { name: 'Collaboration Release review', exact: true });
  await expect(card.getByRole('button', { name: 'Review and close collaboration', exact: true })).toBeDisabled();
  page.once('dialog', dialog => dialog.dismiss()); await card.getByRole('button', { name: 'Request cancellation of all', exact: true }).click(); expect(run.state).toBe('open');
  page.once('dialog', dialog => dialog.accept()); await card.getByRole('button', { name: 'Request cancellation of all', exact: true }).click();
  await expect(card).toContainText('Awaiting cancellation'); await expect(card).toContainText('0 / 1 with final reports'); expect(item.cancel_requested).toBe(true);
  await expect(card.getByRole('button', { name: 'Review and close collaboration', exact: true })).toBeDisabled();
  item.current.state = 'cancelled'; item.current.result = 'Stopped safely'; await page.getByRole('button', { name: 'Refresh tasks', exact: true }).click();
  await card.getByRole('button', { name: 'Review and close collaboration', exact: true }).click();
  await card.getByRole('textbox', { name: 'Reviewed collaboration summary', exact: true }).fill('All workers confirmed stop. <img src=x onerror="window.unsafe=true">');
  await card.getByRole('button', { name: 'Confirm close collaboration', exact: true }).click();
  await expect(card).toContainText('Closed'); await expect(card.locator('img')).toHaveCount(0); await expect(card).toContainText('All workers confirmed stop.');
  page.once('dialog', dialog => dialog.accept()); await card.getByRole('button', { name: 'Archive collaboration', exact: true }).click(); await expect(card).toHaveCount(0);
  expect(control.actions.filter(action => action.operation === 'collaboration').map(action => action.command.operation)).toEqual(['stop', 'close', 'archive']);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
});

test('incoming scopes with identical RunIDs stay separated by coordinator and display plain text', async ({ page }) => {
  const { task, open } = await fixture(page);
  task({ id: runId, owner: 'peer-a', name: 'Same name', goal: 'First goal <img src=x>' }, true); task({ id: runId, owner: 'peer-b', name: 'Same name', goal: 'Second goal' }, true);
  await open(); await page.getByRole('button', { name: /^For this device/ }).click();
  const filter = page.getByRole('combobox', { name: 'Filter by collaboration', exact: true }); await filter.selectOption('peer-a/' + runId);
  await expect(page.locator('.task-card')).toHaveCount(1); await expect(page.locator('.task-card')).toContainText('Incoming from peer-a');
  await page.locator('.task-collaboration summary').click(); await expect(page.locator('.task-collaboration')).toContainText('First goal <img src=x>'); await expect(page.locator('.task-card img')).toHaveCount(0);
  await filter.selectOption('peer-b/' + runId); await expect(page.locator('.task-card')).toContainText('Incoming from peer-b');
  await filter.selectOption('unscoped'); await expect(page.getByRole('heading', { name: 'No tasks in this view', exact: true })).toBeVisible(); await page.getByRole('button', { name: 'Clear filter', exact: true }).click(); await expect(page.locator('.task-card')).toHaveCount(2);
});

test('UTF-8 limits, optional project validation, mobile layouts and stale close responses preserve intent', async ({ page }) => {
  const { add, open, panel } = await fixture(page); const run = add(); await open();
  await panel.getByRole('button', { name: 'New collaboration', exact: true }).click(); const form = panel.getByRole('form', { name: 'Create collaboration run', exact: true });
  await form.getByLabel('Collaboration name', { exact: true }).fill('🙂'.repeat(61)); await form.getByLabel('Shared goal and completion criteria', { exact: true }).fill('Report evidence'); await form.getByRole('checkbox', { name: 'peer-a', exact: true }).check();
  await expect(form.getByRole('button', { name: 'Create collaboration', exact: true })).toBeDisabled(); await form.getByLabel('Collaboration name', { exact: true }).fill('Valid name');
  await form.getByText('Link a project (optional)', { exact: true }).click(); await form.getByLabel('ProjectID', { exact: true }).fill('00000000-0000-0000-0000-000000000000'); await expect(form.getByRole('button', { name: 'Create collaboration', exact: true })).toBeDisabled(); await form.getByLabel('ProjectID', { exact: true }).fill(runId); await expect(form.getByRole('button', { name: 'Create collaboration', exact: true })).toBeEnabled();
  const card = panel.getByRole('article', { name: 'Collaboration Release review', exact: true }); await card.getByRole('button', { name: 'Review and close collaboration', exact: true }).click(); await card.getByRole('textbox', { name: 'Reviewed collaboration summary', exact: true }).fill('Keep this review while refreshing.');
  run.revision++; await card.getByRole('button', { name: 'Confirm close collaboration', exact: true }).click(); await expect(panel.getByRole('alert')).toContainText('Collaboration changed'); await expect(card.getByRole('textbox', { name: 'Reviewed collaboration summary', exact: true })).toHaveValue('Keep this review while refreshing.');
  for (const width of [375, 768, 1440]) { await page.setViewportSize({ width, height: 1000 }); for (const theme of ['light', 'dark']) { await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); } }
});

test('dependency selection stays within a run, enforces eight parents and keeps IDs after an uncertain save', async ({ page }) => {
  const { add, task, open, control } = await fixture(page);
  const run = add(), other = add('22222222-2222-4222-8222-222222222222', 'Other work');
  const parents = Array.from({ length: 9 }, (_, index) => { const item = task(run.scope); item.task.title = 'Build ' + index; return item; });
  task(other.scope).task.title = 'Unrelated build';
  await open(); await page.getByRole('article', { name: 'Collaboration Release review', exact: true }).getByRole('button', { name: 'Add delegation', exact: true }).click();
  const form = page.getByRole('form', { name: 'Delegate a task', exact: true });
  await form.getByText('Prerequisite tasks (optional)', { exact: true }).click();
  await expect(form.getByRole('checkbox')).toHaveCount(9);
  for (let index = 0; index < 8; index++) await form.getByRole('checkbox', { name: 'Wait for Build ' + index + ' (peer-a)', exact: true }).check();
  await expect(form.getByRole('checkbox', { name: 'Wait for Build 8 (peer-a)', exact: true })).toBeDisabled();
  await form.getByRole('combobox', { name: 'Target device', exact: true }).selectOption('peer-b');
  await form.getByRole('textbox', { name: 'Task title', exact: true }).fill('Test builds');
  await form.getByRole('textbox', { name: 'Description and completion criteria', exact: true }).fill('Test the prepared revision.');
  control.uncertainTask = true;
  const submit = form.getByRole('button', { name: 'Save and wait for prerequisites', exact: true }); await submit.click();
  await expect(page.getByRole('alert')).toContainText('Response interrupted');
  await expect(form.getByRole('textbox', { name: 'Task title', exact: true })).toBeFocused();
  await expect(form.locator('input[type=checkbox]:checked')).toHaveCount(8);
  await submit.click();
  const creates = control.actions.filter(action => action.operation === 'create'); expect(creates).toHaveLength(2); expect(creates[0]).toEqual(creates[1]); expect(creates[0].depends_on).toEqual(parents.slice(0, 8).map(item => item.task.id).sort());
  await expect(form.locator('input[type=checkbox]:checked')).toHaveCount(0);
  await form.getByRole('checkbox', { name: 'Wait for Build 0 (peer-a)', exact: true }).check();
  await form.getByRole('combobox', { name: 'Delegate within collaboration', exact: true }).selectOption(other.scope.id);
  await expect(form.getByRole('checkbox')).toHaveCount(1); await expect(form.getByRole('checkbox')).not.toBeChecked();
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('Test the prepared revision');
});

test('waiting explains failed prerequisites, renders reports as text and permits local cancellation after connection changes', async ({ page }) => {
  const { add, task, open, control } = await fixture(page); const run = add(), parent = task(run.scope), child: any = task(run.scope);
  parent.task.title = 'Compile'; parent.current.state = 'failed'; parent.current.result = 'Compiler missing <img src=x onerror="window.unsafe=true">' as any;
  child.task.title = 'Tests'; child.current.state = 'queued'; child.blocked = true; child.error = 'Connection or credentials changed.';
  child.dependencies = { state: 'waiting', prerequisites: [{ id: parent.task.id, title: parent.task.title, node: 'peer-a', current: parent.current }] };
  await open(); const card = page.getByRole('article', { name: 'Task Tests', exact: true });
  await expect(card).toContainText('this task remains unsent'); await expect(card.getByRole('button', { name: 'Cancel waiting task', exact: true })).toBeEnabled();
  await card.getByText('Prerequisites and reports', { exact: false }).click();
  await expect(card).toContainText('Compiler missing <img src=x'); await expect(card.locator('img')).toHaveCount(0);
  await card.getByRole('button', { name: 'Recheck prerequisites', exact: true }).click(); expect(control.actions.some(action => action.operation === 'sync')).toBe(true);
  page.once('dialog', dialog => dialog.dismiss()); await card.getByRole('button', { name: 'Cancel waiting task', exact: true }).click(); expect(control.actions.some(action => action.operation === 'cancel')).toBe(false);
  page.once('dialog', dialog => dialog.accept()); await card.getByRole('button', { name: 'Cancel waiting task', exact: true }).click();
  await expect(card).toContainText('This attempt was never sent'); await expect(card).toContainText('Local cancellation'); await expect(card.getByText('Worker report', { exact: true })).toHaveCount(0); await expect(card.getByRole('button', { name: 'Cancel waiting task', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).unsafe)).toBeUndefined();
});

test('released dependencies retain reviewable receipts after reopen and fit both themes on mobile and desktop', async ({ page }) => {
  const { add, task, open } = await fixture(page); const run = add(), parent = task(run.scope), child: any = task(run.scope);
  parent.current.state = 'succeeded'; parent.current.result = 'Build evidence verified.' as any; child.task.title = 'Test prepared build';
  child.dependencies = { state: 'released', prerequisites: [{ id: parent.task.id, title: 'Prepared build', node: 'peer-a', current: parent.current }] };
  await open(); await page.reload(); await page.getByRole('button', { name: 'Pause updates', exact: true }).click();
  const card = page.getByRole('article', { name: 'Task Test prepared build', exact: true });
  await expect(card).toContainText('Delegation is released'); await expect(card).toContainText('Awaiting Agent');
  await card.getByText('Prerequisites and reports', { exact: false }).click(); await expect(card).toContainText('Build evidence verified.');
  for (const width of [375, 768, 1440]) { await page.setViewportSize({ width, height: 1000 }); for (const theme of ['light', 'dark']) { await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); } }
});
