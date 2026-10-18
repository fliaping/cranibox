import { expect, test } from '@playwright/test';

test('chunk progress survives panel closure and paused preparation requires review before resuming a copy', async ({ page }) => {
  const actions: any[] = [], errors: string[] = [];
  const id = '99999999-9999-4999-8999-999999999999', replica = '88888888-8888-4888-8888-888888888888';
  let job: any, release = () => {}, commits = 0;
  const preparing = new Promise<void>(resolve => { release = resolve; });
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { routes: [{ node: 'source' }, { node: 'target' }] } });
    if (command.operation === 'projects') return route.fulfill({ json: { owner: command.node, revision: 1, permissions: { exec: true, projects: true }, projects: command.node === 'source' ? [{ id: 'project', name: 'Large portable project', replicas: [{ id: replica, node: 'source', path: '/source', sessions: [] }] }] : [] } });
    if (command.operation === 'copy_list') return route.fulfill({ json: { copies: job ? [job] : [] } });
    if (command.operation === 'copy_activity') return route.fulfill({ json: { copies: job ? [{ id, state: job.state, progress: job.progress, error: job.error }] : [] } });
    if (command.operation === 'copy_prepare') {
      job = { id, protocol: 2, source: 'source', target: 'target', target_root: '/received', digest: '', state: 'preparing', created_at: 1792252800000, progress: { phase: 'preparing', bytes: 1048576, total: 33554432 }, summary: { name: 'Large portable project', files: Array.from({ length: 250 }, (_, index) => ({ path: 'src/file-' + index + '.rs', size: 131072 })), directories: ['src'], skipped: ['.env'], bytes: 33554432 } };
      await preparing;
      job = { ...job, state: 'prepare_paused', error: 'Snapshot preparation paused. Resume this record.' };
      return route.fulfill({ status: 400, json: { error: job.error } });
    }
    if (command.operation === 'copy_prepare_resume') job = { ...job, state: 'prepared', digest: 'd'.repeat(64), progress: null, error: null };
    if (command.operation === 'copy_commit') job = ++commits === 1 ? { ...job, state: 'uncertain', progress: { phase: 'sending', bytes: 8388608, total: 33554432 }, error: 'Transfer paused. Check the target receipt.' } : { ...job, state: 'completed', result: { path: '/received/cranibox-' + id + '/project' }, error: null };
    if (command.operation === 'copy_status') job = { ...job, state: 'receiving', progress: { phase: 'sending', bytes: 16777216, total: 33554432 }, error: null };
    return route.fulfill({ json: job });
  });
  await page.goto('/#/projects');
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'Copy destination', exact: true }).selectOption('target');
  await panel.getByRole('button', { name: 'Prepare and preview snapshot', exact: true }).click();
  const progress = panel.getByRole('progressbar', { name: 'Project copy progress', exact: true });
  await expect(progress).toHaveAttribute('value', '1048576');
  await panel.getByRole('button', { name: 'Close copy panel', exact: true }).click();
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await expect(progress).toBeVisible();
  expect(actions.some(action => action.operation === 'copy_commit')).toBe(false);
  release();
  await panel.getByRole('button', { name: 'Resume snapshot preparation', exact: true }).click();
  const commit = panel.getByRole('button', { name: 'Copy reviewed snapshot', exact: true });
  await expect(commit).toBeDisabled();
  await panel.getByText('Review file manifest', { exact: true }).click();
  await expect(panel.locator('details').first().locator('li')).toHaveCount(200);
  await panel.getByRole('searchbox', { name: 'Filter paths', exact: true }).fill('file-249');
  await expect(panel.locator('details').first().locator('li')).toHaveCount(1);
  await expect(panel.locator('details').first()).toContainText('src/file-249.rs');
  await panel.getByRole('checkbox').check();
  await commit.click();
  await expect(progress).toHaveAttribute('value', '8388608');
  await panel.getByRole('button', { name: 'Check target receipt', exact: true }).click();
  await expect(progress).toHaveAttribute('value', '16777216');
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: '/tmp/cranibox-streaming-375-' + theme + '.png', fullPage: true, animations: 'disabled' });
  }
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: 'Resume original copy', exact: true }).click();
  await expect(panel.locator('.projects-copy-review')).toContainText('Completed');
  expect(actions.filter(action => action.operation === 'copy_commit')).toEqual([{ operation: 'copy_commit', id, digest: 'd'.repeat(64) }, { operation: 'copy_commit', id, digest: 'd'.repeat(64) }]);
  expect(errors).toEqual([]);
});

test('a late progress response cannot regress a completed receipt or repeat a write', async ({ page }) => {
  const id = '11111111-1111-4111-8111-111111111111', actions: any[] = [];
  let job: any = { id, protocol: 2, source: 'source', target: 'target', target_root: '/received', digest: 'e'.repeat(64), state: 'receiving', created_at: 1792252800000, progress: { phase: 'sending', bytes: 1048576, total: 2097152 }, summary: { name: 'Background transfer', files: [{ path: 'main.rs', size: 2097152 }], directories: [], skipped: [], bytes: 2097152 } };
  let release = () => {}, pending = false;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/projects', route => route.fulfill({ json: { revision: 1, projects: [] } }));
  await page.route('**/api/peers', async route => {
    const command = route.request().postDataJSON(); actions.push(command);
    if (command.operation === 'list') return route.fulfill({ json: { routes: [] } });
    if (command.operation === 'copy_list') return route.fulfill({ json: { copies: [job] } });
    if (command.operation === 'copy_activity') {
      const previous = structuredClone(job); pending = true;
      await gate;
      return route.fulfill({ json: { copies: [previous] } });
    }
    if (command.operation === 'copy_status') job = { ...job, state: 'completed', result: { path: '/received/project' } };
    return route.fulfill({ json: job });
  });
  await page.goto('/#/projects');
  const panel = page.getByRole('region', { name: 'Project file copies', exact: true });
  await panel.getByRole('button', { name: 'Copy project to device', exact: true }).click();
  await panel.getByRole('combobox', { name: 'View copy record', exact: true }).selectOption(id);
  await expect.poll(() => pending).toBe(true);
  await panel.getByRole('button', { name: 'Check target receipt', exact: true }).click();
  await expect(panel.locator('.projects-copy-review')).toContainText('Completed');
  release();
  await expect(panel.getByRole('progressbar')).toHaveCount(0);
  expect(actions.some(action => action.operation === 'copy_commit')).toBe(false);
});
