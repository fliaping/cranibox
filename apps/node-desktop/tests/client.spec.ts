import { expect, test } from '@playwright/test';

test('Computer Use bridge and local emergency controls use the shared native IPC', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const catalog: any = { node_id: 'desktop-direct-id', routes: [], grants: [], computer: null, computer_paused: false };
    internals.invoke = async (command: string, payload: any) => {
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push(structuredClone({ command, payload }));
      const action = payload.command;
      if (action.operation === 'computer_sharing') catalog.computer = action.computer;
      if (action.operation === 'computer_owner') {
        if (action.command.action !== 'status') catalog.computer_paused = action.command.action === 'emergency_stop';
        return { paused: catalog.computer_paused, backend_available: true };
      }
      return structuredClone(catalog);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: 'Agent 桌面操作', exact: true });
  await panel.getByLabel('本机桥接地址', { exact: true }).fill('127.0.0.1:8765');
  await panel.getByRole('button', { name: '保存桌面桥接', exact: true }).click();
  await expect(panel).toContainText('桌面桥接已保存');
  await panel.getByRole('button', { name: '紧急停止 Agent 输入', exact: true }).click();
  await expect(panel).toContainText('已紧急停止');
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: '在此设备恢复', exact: true }).click();
  await expect(panel).toContainText('桌面桥接就绪');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation.startsWith('computer')));
  expect(calls.map((c: any) => c.payload.command)).toEqual([
    { operation: 'computer_sharing', computer: { address: '127.0.0.1:8765' } },
    { operation: 'computer_owner', command: { action: 'emergency_stop' } },
    { operation: 'computer_owner', command: { action: 'resume' } },
  ]);
});

test('Browser Use sharing and local emergency controls use the shared native IPC', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const catalog: any = { node_id: 'desktop-direct-id', routes: [], grants: [], browser: null, browser_paused: false };
    internals.invoke = async (command: string, payload: any) => {
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push(structuredClone({ command, payload }));
      const action = payload.command;
      if (action.operation === 'browser_sharing') catalog.browser = action.browser;
      if (action.operation === 'browser_owner') {
        if (action.command.action !== 'status') catalog.browser_paused = action.command.action === 'emergency_stop';
        return { paused: catalog.browser_paused, backend_available: true, browser: { version: 'TestChromium/1' } };
      }
      return structuredClone(catalog);
    };
  });
  await page.goto('/');
  await page.locator('summary').filter({ hasText: 'Agent 浏览器操作' }).click();
  const panel = page.getByRole('region', { name: 'Agent 浏览器操作', exact: true });
  await panel.getByLabel('本机 Chromium CDP 地址', { exact: true }).fill('127.0.0.1:9222');
  await panel.getByRole('button', { name: '保存浏览器共享', exact: true }).click();
  await expect(panel).toContainText('浏览器共享已保存');
  await panel.getByRole('button', { name: '紧急停止浏览器操作', exact: true }).click();
  await expect(panel).toContainText('已紧急停止');
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: '在此设备恢复浏览器共享', exact: true }).click();
  await expect(panel).toContainText('浏览器已就绪');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation.startsWith('browser')));
  expect(calls.map((c: any) => c.payload.command)).toEqual([
    { operation: 'browser_sharing', browser: { address: '127.0.0.1:9222' } },
    { operation: 'browser_owner', command: { action: 'emergency_stop' } },
    { operation: 'browser_owner', command: { action: 'resume' } },
  ]);
});

test('native Tailscale setup uses the reviewed plan through IPC and survives navigation', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const status: any = { node_id: 'own-device', easytier: { installed: false, enabled: false, phase: 'stopped', devices: [], connections: [] }, tailscale: { installed: false, state: 'NotInstalled', devices: [], setup: { phase: 'idle' } } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'network') {
        (window as any).calls.push(structuredClone({ command, payload }));
        if (payload.command.operation === 'tailscale_setup_plan') return { available: true, id: 'native-reviewed-plan', platform: 'windows', action: 'install', requires_admin: true, expires_in_seconds: 600, commands: [{ program: 'winget.exe', args: ['install', '--id', 'Tailscale.Tailscale'] }] };
        if (payload.command.operation === 'tailscale_setup') status.tailscale.setup = { phase: 'running', action: 'install' };
        return structuredClone(status);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '设备网络', exact: true }).click();
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('link', { name: '打开官方下载页面', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.find((c: any) => c.command === 'open_network_link')?.payload.url)).toBe('https://tailscale.com/download');
  await ts.getByRole('button', { name: '检查 Tailscale 安装方案', exact: true }).click();
  const execute = ts.getByRole('button', { name: '确认并执行', exact: true });
  await expect(execute).toBeDisabled();
  await ts.getByLabel('我已审阅，允许在当前节点执行以上操作', { exact: true }).check();
  await execute.click();
  await expect(ts).toContainText('正在执行安装方案');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await expect(ts).toContainText('正在执行安装方案');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'network' && c.payload.command.operation !== 'status'));
  expect(calls.map((c: any) => c.payload.command)).toEqual([{ operation: 'tailscale_setup_plan' }, { operation: 'tailscale_setup', plan_id: 'native-reviewed-plan' }]);
});

test('native background catalog sync configures and pauses the shared Rust node policy', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const state: any = { owner: 'own-device', revision: 0, enabled: false, interval_seconds: 60, available_nodes: ['trusted-device'], nodes: [] };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: state.owner, online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'local_projects') return { revision: 0, projects: [] };
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push(structuredClone({ command, payload }));
      const action = payload.command;
      if (action.operation === 'list') return { node_id: state.owner, routes: [], grants: [] };
      if (action.operation !== 'project_sync') return previous(command, payload);
      const update = action.command;
      if (update.operation === 'configure') { state.revision++; state.enabled = true; state.interval_seconds = update.interval_seconds; state.nodes = update.nodes.map((node: string) => ({ node, connection_valid: true, phase: 'waiting', last_attempt: null, last_success: null, catalog_revision: null, project_count: null })); }
      if (update.operation === 'pause') { state.revision++; state.enabled = false; state.nodes.forEach((node: any) => { node.phase = 'paused'; }); }
      return structuredClone(state);
    };
  });
  await page.goto('/'); await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '项目与会话', exact: true }).click();
  const panel = page.locator('.project-sync'); await panel.locator(':scope > summary').click();
  await panel.getByRole('button', { name: '设置后台同步', exact: true }).click();
  await panel.getByRole('checkbox', { name: 'trusted-device', exact: true }).check();
  await panel.getByRole('combobox', { name: '同步间隔', exact: true }).selectOption('30');
  await panel.getByRole('button', { name: '启用后台同步', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('已保存');
  await panel.getByRole('button', { name: '暂停同步', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('现有快照仍可查看');
  const calls = await page.evaluate(() => (window as any).calls);
  const updates = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'project_sync').map((call: any) => call.payload.command.command);
  expect(updates.filter((action: any) => action.operation === 'configure')).toEqual([{ operation: 'configure', revision: 0, nodes: ['trusted-device'], interval_seconds: 30 }]);
  expect(updates.at(-1)).toEqual({ operation: 'pause', revision: 1 });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});

test('native automatic workers use the reviewed local policy through shared Rust IPC', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke, now = Math.floor(Date.now() / 1000);
    const state: any = { owner: 'own-device', revision: 0, policy: null, senders: [{ node: 'trusted-device', selected: false, binding_active: false }], runs: [], ready: null };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: state.owner, online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push(structuredClone({ command, payload })); const action = payload.command;
      if (action.operation === 'list') return { node_id: state.owner, routes: [], grants: [] };
      if (action.operation === 'agent_sessions') return { owner: state.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] };
      if (action.operation === 'executions') return { owner: state.owner, executions: [], used: 0, capacity: 128, parallel: 8, log_limit: 1048576 };
      if (action.operation === 'tasks') {
        if (action.command.operation !== 'automatic') return { owner: state.owner, incoming: [], outgoing: [], used: 0, capacity: 128, max_attempts: 8 };
        const update = action.command.command;
        if (update.operation === 'prepare') return { owner: state.owner, revision: update.revision, options: update.options, digest: 'a'.repeat(64), available: true, launch: { program: '/tools/codex', args: ['exec', '-'], cwd: update.options.agent.cwd } };
        if (update.operation === 'save') { state.policy = structuredClone(update.options); state.revision++; state.senders[0].binding_active = true; state.ready = { directory: true, agent: true }; }
        if (update.operation === 'pause') { state.policy.enabled = false; state.revision++; }
        return structuredClone(state);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/'); await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.locator('.automatic-worker'); await panel.locator(':scope > summary').click();
  await panel.getByRole('button', { name: '配置自动接单', exact: true }).click();
  await panel.getByRole('checkbox', { name: '确认保存后启用自动接单', exact: true }).check();
  await panel.getByRole('checkbox', { name: 'trusted-device', exact: true }).check();
  await panel.getByRole('textbox', { name: '固定工作目录', exact: true }).fill('/home/user/project');
  await panel.getByRole('button', { name: '检查接单配置', exact: true }).click();
  const review = panel.getByRole('region', { name: '确认自动接单配置', exact: true });
  await expect(review.getByRole('heading')).toBeFocused();
  await expect(review.getByLabel('已核对的发送设备')).toContainText('trusted-device');
  const save = review.getByRole('button', { name: '确认启用自动接单', exact: true }); await expect(save).toBeDisabled();
  await review.getByRole('checkbox').check(); await save.click();
  await expect(panel.getByRole('status')).toContainText('自动接单配置已保存');
  await panel.getByRole('button', { name: '暂停新任务', exact: true }).click();
  await expect(panel.getByRole('status')).toContainText('已启动的进程继续运行');
  const calls = await page.evaluate(() => (window as any).calls);
  const updates = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'automatic').map((call: any) => call.payload.command.command.command);
  expect(updates.filter((action: any) => action.operation === 'save')).toEqual([{ operation: 'save', revision: 0, options: { enabled: true, senders: ['trusted-device'], agent: { adapter: 'codex', access: 'read_only', cwd: '/home/user/project', communication: true }, parallel: 1, report: 'human_review' }, digest: 'a'.repeat(64) }]);
  expect(updates.at(-1)).toEqual({ operation: 'pause', revision: 1 });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});

test('resumable snapshots share progress and explicit review through native IPC', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    let job: any = { id: 'stream-copy', protocol: 2, source: 'source', target: 'own-device', target_root: '/received', digest: '', state: 'prepare_paused', created_at: 1792252800000, progress: { phase: 'preparing', bytes: 1048576, total: 33554432 }, summary: { name: '可续传项目', files: [{ path: 'main.rs', size: 33554432 }], directories: [], skipped: ['.env'], bytes: 33554432 } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'local_projects') return { revision: 1, projects: [] };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: [], grants: [] };
        if (action.operation === 'copy_list') return { copies: [structuredClone(job)] };
        if (action.operation === 'copy_activity') return { copies: [{ id: job.id, state: job.state, progress: job.progress }] };
        if (action.operation === 'copy_prepare_resume') job = { ...job, digest: 'a'.repeat(64), state: 'prepared', progress: null };
        if (action.operation === 'copy_commit') job = { ...job, state: 'uncertain', progress: { phase: 'sending', bytes: 16777216, total: 33554432 } };
        if (action.operation === 'copy_status') job = { ...job, state: 'completed', result: { path: '/received/project' } };
        return structuredClone(job);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '项目与会话', exact: true }).click();
  const panel = page.getByRole('region', { name: '项目文件复制', exact: true });
  await panel.getByRole('button', { name: '复制项目到设备', exact: true }).click();
  await panel.getByRole('combobox', { name: '查看复制记录', exact: true }).selectOption('stream-copy');
  await expect(panel.getByRole('progressbar', { name: '项目复制进度', exact: true })).toHaveAttribute('value', '1048576');
  await panel.getByRole('button', { name: '继续生成快照', exact: true }).click();
  const commit = panel.getByRole('button', { name: '确认复制此快照', exact: true });
  await expect(commit).toBeDisabled();
  await panel.getByRole('checkbox').check();
  await commit.click();
  await expect(panel.getByRole('progressbar')).toHaveAttribute('value', '16777216');
  await panel.getByRole('button', { name: '查询目标回执', exact: true }).click();
  await expect(panel.locator('.projects-copy-review')).toContainText('已完成');
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'copy_commit').map((call: any) => call.payload.command))).toEqual([{ operation: 'copy_commit', id: 'stream-copy', digest: 'a'.repeat(64) }]);
  expect(errors).toEqual([]);
});

test('native space desktop and editor launch through Rust IPC and recover from browser errors', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    let fail = true;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'direct_peers' && payload.command.operation === 'list') return { node_id: 'own-device', routes: [], grants: [] };
      if (command === 'spaces') return { platform: 'linux', default_image: 'local:test', setup: null, spaces: [{ id: 'space-app', name: '应用空间', provider: 'docker', image: 'local:test', node: 'space-node', phase: 'ready', volume: 'persistent', error: null, ports: { desktop: 43001, code: 48443 } }] };
      if (command === 'space_application') {
        (window as any).calls.push({ command, payload });
        if (fail) { fail = false; throw Error('System browser unavailable'); }
        return;
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '虚拟空间', exact: true }).click();
  const space = page.getByRole('article', { name: '应用空间', exact: true });
  await space.getByRole('button', { name: '桌面', exact: true }).click();
  await expect(page.locator('.virtual-spaces > [role=alert]')).toContainText('System browser unavailable');
  await space.getByRole('button', { name: '编辑器', exact: true }).click();
  await expect(page.getByText('已在浏览器中打开。关闭标签页不会停止空间。', { exact: true })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'space_application'));
  expect(calls).toEqual([{ command: 'space_application', payload: { id: 'space-app', application: 'desktop' } }, { command: 'space_application', payload: { id: 'space-app', application: 'code' } }]);
});

test('native device paths use shared controls and the local Rust IPC adapter', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const device = { node: 'remote-device', url: 'http://10.0.0.2:9091', allow_private_http: true, paths: [{ url: 'http://10.0.0.2:9091', allow_private_http: true }] };
    const catalog = { node_id: 'own-device', routes: [device], grants: [] };
    internals.invoke = async (command: string, payload: any) => {
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push({ command, payload });
      const action = payload.command;
      if (action.operation === 'add_path') device.paths.push({ url: action.url, allow_private_http: action.allow_private_http });
      if (action.operation === 'probe') return { nodes: [{ id: device.node, os: 'linux', arch: 'x86_64', terminal_sessions: true }], connection: { url: device.paths.at(-1)!.url, fallback: true, path_count: device.paths.length } };
      return structuredClone(catalog);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '设备网络', exact: true }).click();
  const panel = page.getByRole('region', { name: '设备连接路径', exact: true });
  await panel.getByRole('button', { name: '添加路径', exact: true }).click();
  const form = panel.getByRole('form', { name: '添加设备路径', exact: true });
  await form.getByLabel('路径地址', { exact: true }).fill('http://100.64.0.2:9091');
  await form.getByLabel(/^现有目标授权 Token/).fill('e'.repeat(64));
  await form.getByRole('checkbox').check();
  await form.getByRole('button', { name: '检查并保存路径', exact: true }).click();
  await expect(panel.locator('li')).toHaveCount(2);
  await expect(panel.getByRole('button', { name: '添加路径', exact: true })).toBeFocused();
  await page.getByRole('button', { name: '检查连接', exact: true }).click();
  await expect(panel.locator('li').nth(1)).toContainText('上次检查可用');
  await expect(panel).toContainText('上次检查通过备用路径连接');
  await page.setViewportSize({ width: 375, height: 950 });
  for (const theme of ['dark', 'light']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  const saved = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'add_path'));
  expect(saved).toEqual([{ command: 'direct_peers', payload: { command: { operation: 'add_path', node: 'remote-device', url: 'http://100.64.0.2:9091', token: 'e'.repeat(64), allow_private_http: true } } }]);
});

for (const { agent, targetDevice } of [{ agent: 'custom', targetDevice: 'own-device' }, { agent: 'codex', targetDevice: 'own-device' }, { agent: 'custom', targetDevice: 'remote-device' }] as const) {
test('native background launches use IPC and remain after leaving the page: ' + agent + ' on ' + targetDevice, async ({ page }) => {
  await page.addInitScript(({ targetDevice }) => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const now = Math.floor(Date.now() / 1000), launches = new Map<string, any>();
    const catalog = { owner: targetDevice, executions: [] as any[], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: targetDevice === 'own-device' ? [] : [{ node: targetDevice, url: 'http://10.0.0.2:9091', allow_private_http: true }], grants: [] };
        if (action.operation === 'tasks') return { owner: catalog.owner, incoming: [], outgoing: [], capacity: 128, used: 0, max_attempts: 8 };
        if (action.operation === 'agent_sessions') return { owner: catalog.owner, sessions: [], capacity: 64, observed_at: now, lease_secs: 300 };
        if (action.operation === 'executions') {
          const update = action.command;
          if (update.operation === 'prepare_agent') return { adapter: update.agent.adapter, access: update.agent.access, available: true, launch: { name: update.agent.name, harness: 'codex', program: '/usr/bin/codex', args: ['exec', '-'], cwd: update.agent.cwd, stdin: update.agent.prompt } };
          if (update.operation === 'start' && !launches.has(update.id)) {
            launches.set(update.id, update.launch);
            catalog.executions.push({ id: update.id, session_id: '44444444-4444-4444-8444-444444444444', name: update.launch.name, harness: update.launch.harness, state: 'running', created_at: now, updated_at: now, pid: 123, exit_code: null, error: null, stdout_truncated: false, stderr_truncated: false });
          }
          if (update.operation === 'get') return { execution: catalog.executions[0], launch: launches.get(update.id), stdout: { text: '设备输出', next_offset: 12 }, stderr: { text: '', next_offset: 0 } };
          return structuredClone(catalog);
        }
      }
      return previous(command, payload);
    };
  }, { targetDevice });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.locator('.managed-runs');
  await panel.locator(':scope > summary').click();
  if (targetDevice !== 'own-device') await panel.getByRole('combobox', { name: '后台执行设备', exact: true }).selectOption(targetDevice);
  await panel.locator('.managed-run-launch > summary').click();
  await panel.getByRole('combobox', { name: 'Agent / 程序', exact: true }).selectOption(agent);
  await panel.getByLabel('运行名称', { exact: true }).fill('原生后台审查');
  if (agent === 'custom') await panel.getByLabel('可执行程序', { exact: true }).fill('review-agent');
  else await panel.getByLabel('任务说明', { exact: true }).fill('审查本机项目，保留中文说明。');
  await panel.getByLabel('完整工作目录', { exact: true }).fill('/home/user/project');
  await panel.getByRole('button', { name: '检查启动内容', exact: true }).click();
  await expect(panel.locator('.managed-run-review')).toContainText(targetDevice);
  await panel.getByRole('button', { name: '确认启动', exact: true }).click();
  await expect(panel.locator('.managed-run-card')).toContainText('进程运行中');
  await nav.getByRole('button', { name: '连接设置', exact: true }).click();
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  await panel.locator(':scope > summary').click();
  if (targetDevice !== 'own-device') await panel.getByRole('combobox', { name: '后台执行设备', exact: true }).selectOption(targetDevice);
  await panel.getByRole('button', { name: '查看输出', exact: true }).click();
  await expect(panel.getByLabel('stdout', { exact: true })).toHaveText('设备输出');
  await page.setViewportSize({ width: 375, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const calls = await page.evaluate(() => (window as any).calls);
  const starts = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'executions' && call.payload.command.command.operation === 'start');
  expect(starts).toHaveLength(1);
  expect(starts[0].payload.command.node).toBe(targetDevice === 'own-device' ? undefined : targetDevice);
  expect(starts[0].payload.command.command.launch).toMatchObject(agent === 'custom' ? { program: 'review-agent', args: [], cwd: '/home/user/project' } : { program: '/usr/bin/codex', args: ['exec', '-'], cwd: '/home/user/project', stdin: '审查本机项目，保留中文说明。' });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});
}

test('native virtual spaces show authorized remote directories without granting control or using a relay', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    (window as any).spaceDirectoryGranted = true;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'spaces') {
        (window as any).calls.push({ command, payload });
        return { spaces: [], setup: null, platform: 'macos', default_image: 'cranibox-test:latest' };
      }
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: [{ node: 'linux-manager', url: 'http://10.0.0.2:9091', allow_private_http: true }], grants: [] };
        if (action.operation === 'probe') return { nodes: [{ id: 'linux-manager', spaces: (window as any).spaceDirectoryGranted }] };
        if (action.operation === 'space_catalog') return { owner: 'linux-manager', spaces: [{ id: '11111111-1111-4111-8111-111111111111', name: '远端 Agent 空间', node: 'remote-space', provider: 'docker', phase: 'ready' }] };
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '虚拟空间', exact: true }).click();
  const panel = page.getByRole('region', { name: '其他设备的虚拟空间', exact: true });
  await expect(panel.getByText('远端 Agent 空间', { exact: true })).toBeVisible();
  await expect(panel).toContainText('linux-manager');
  await expect(panel).toContainText('remote-space');
  await expect(panel).toContainText('未检测容器');
  await nav.getByRole('button', { name: '设备网络', exact: true }).click();
  await nav.getByRole('button', { name: '虚拟空间', exact: true }).click();
  await expect(panel.getByText('远端 Agent 空间', { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 375, height: 1000 });
  for (const theme of ['light', 'dark']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.evaluate(() => { (window as any).spaceDirectoryGranted = false; window.dispatchEvent(new Event('cranibox:peers-changed')); });
  const details = panel.locator('.remote-space-details');
  await expect(details.locator('summary')).toContainText('1 个未共享');
  await expect(details).not.toHaveAttribute('open');
  await expect(panel.getByText('远端 Agent 空间', { exact: true })).toHaveCount(0);
  await details.locator('summary').focus();
  await page.keyboard.press('Enter');
  await expect(details).toContainText('未共享的设备没有授予空间目录权限');
  await expect(panel.getByRole('article', { name: 'linux-manager', exact: true })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((call: any) => call.command === 'spaces').every((call: any) => call.payload.command.operation === 'list')).toBe(true);
  expect(calls.filter((call: any) => call.command === 'direct_peers').every((call: any) => ['list', 'probe', 'space_catalog'].includes(call.payload.command.operation))).toBe(true);
  expect(calls.some((call: any) => ['configure', 'terminal_open', 'reconnect'].includes(call.command))).toBe(false);
  expect(errors).toEqual([]);
});

test('native project handoffs use reviewed Rust IPC and recover the same continuation', async ({ page }) => {
  const id = '77777777-7777-4777-8777-777777777777';
  await page.addInitScript(({ id }) => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const destination = '/received/project', handoff = { goal: '继续处理待完成的端点', progress: '已有文件已检查', next_steps: '实现接口并验证', verification: '' };
    const job = { id, source: 'remote', target: 'own-device', digest: 'a'.repeat(64), state: 'completed', created_at: 1792252800000, target_root: '/received', result: { path: destination }, summary: { name: '原生交接', files: [], directories: [], skipped: [], bytes: 0, handoff, handoff_path: '.cranibox/handoffs/' + id + '.md' } };
    const launch = { name: '原生交接', program: '/usr/bin/codex', args: ['exec', '-'], cwd: destination, stdin: handoff.goal };
    let started = false;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'local_projects') return { revision: 1, projects: [] };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: [], grants: [] };
        if (action.operation === 'copy_list') return { copies: [job] };
        if (action.operation === 'copy_agent_prepare') return { launch, available: true, execution_id: id, digest: 'b'.repeat(64) };
        if (action.operation === 'copy_agent_start') started = true;
        if (['copy_agent_start', 'copy_agent_status'].includes(action.operation)) {
          if (!started) throw Error('run unavailable');
          return { owner: 'own-device', launch, execution: { id, state: 'succeeded', exit_code: 0 }, stdout: { text: action.stdout_offset ? '' : '原生接续结果', next_offset: 18 }, stderr: { text: '', next_offset: 0 } };
        }
      }
      return previous(command, payload);
    };
  }, { id });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '项目与会话', exact: true }).click();
  const panel = page.getByRole('region', { name: '项目文件复制', exact: true });
  const openHistory = async () => {
    await panel.getByRole('button', { name: '复制项目到设备', exact: true }).click();
    await panel.getByRole('combobox', { name: '查看复制记录', exact: true }).selectOption(id);
  };
  await openHistory();
  const continuation = panel.getByRole('region', { name: '在目标设备接续 Agent', exact: true });
  await continuation.getByRole('button', { name: '检查目标 Agent', exact: true }).click();
  await expect(continuation.getByLabel('完整接续指令', { exact: true })).toHaveText('继续处理待完成的端点');
  expect(await page.evaluate(() => (window as any).calls.some((c: any) => c.payload?.command?.operation === 'copy_agent_start'))).toBe(false);
  await continuation.getByRole('button', { name: '确认在目标启动', exact: true }).click();
  await expect(continuation.getByLabel('接续 stdout', { exact: true })).toHaveText('原生接续结果');
  await nav.getByRole('button', { name: '连接设置', exact: true }).click();
  await nav.getByRole('button', { name: '项目与会话', exact: true }).click();
  await openHistory();
  await continuation.getByRole('button', { name: '查找已有接续任务', exact: true }).click();
  await expect(continuation.getByLabel('接续 stdout', { exact: true })).toHaveText('原生接续结果');
  const commands = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'direct_peers').map((c: any) => c.payload.command));
  expect(commands.filter((c: any) => c.operation === 'copy_agent_start')).toEqual([{ operation: 'copy_agent_start', id, adapter: 'codex', access: 'read_only', digest: 'b'.repeat(64) }]);
  expect(commands.some((c: any) => c.operation === 'connect')).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('待完成的端点');
});

test('local project copies work in both directions without a relay or self connection', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    let received = false, job: any;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'local_projects') return { revision: 1, projects: received ? [{ id: 'local-copy', name: '跨设备项目', replicas: [{ id: '66666666-6666-4666-8666-666666666666', node: 'own-device', path: '/projects/local', sessions: [] }] }] : [] };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: [{ node: 'source', url: 'https://source.example.com', allow_private_http: false }], grants: [] };
        if (action.operation === 'projects') return { owner: 'source', revision: 1, permissions: { exec: true, projects: true, terminals: false }, projects: [{ id: 'local-copy', name: '跨设备项目', replicas: [{ id: '55555555-5555-4555-8555-555555555555', node: 'source', path: '/projects/source', sessions: [] }] }] };
        if (action.operation === 'copy_list') return { copies: job ? [job] : [] };
        if (action.operation === 'copy_prepare') job = { id: 'local-job', source: action.source, target: action.target, target_root: '/projects/received', digest: 'a'.repeat(64), state: 'prepared', created_at: 1792180800000, summary: { name: '跨设备项目', files: [{ path: 'main.rs', size: 32 }], directories: [], skipped: [], bytes: 32 } };
        if (action.operation === 'copy_commit') { received = true; job = { ...job, state: 'completed', result: { path: '/projects/local' } }; }
        return structuredClone(job);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.getByRole('heading', { name: '跨设备项目', exact: true })).toBeVisible();
  const panel = page.getByRole('region', { name: '项目文件复制', exact: true });
  await panel.getByRole('button', { name: '复制项目到设备', exact: true }).click();
  await panel.getByRole('combobox', { name: '目标设备', exact: true }).selectOption('own-device');
  await panel.getByRole('button', { name: '生成并预览快照', exact: true }).click();
  await panel.getByRole('checkbox').check();
  await panel.getByRole('button', { name: '确认复制此快照', exact: true }).click();
  const working = page.getByRole('combobox', { name: '工作副本', exact: true });
  await expect(working.locator('option')).toHaveCount(2);
  await working.selectOption('local:own-device/66666666-6666-4666-8666-666666666666');
  await panel.getByRole('combobox', { name: '目标设备', exact: true }).selectOption('source');
  await panel.getByRole('button', { name: '生成并预览快照', exact: true }).click();
  await expect(panel.getByRole('checkbox')).not.toBeChecked();
  await expect(panel.getByRole('button', { name: '确认复制此快照', exact: true })).toBeDisabled();
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation === 'copy_prepare').map((c: any) => c.payload.command)).toEqual([
    { operation: 'copy_prepare', source: 'source', target: 'own-device', replica_id: '55555555-5555-4555-8555-555555555555' },
    { operation: 'copy_prepare', source: 'own-device', target: 'source', replica_id: '66666666-6666-4666-8666-666666666666' },
  ]);
  expect(calls.some((c: any) => c.command === 'configure' || c.command === 'terminal_open' || c.payload?.command?.operation === 'connect')).toBe(false);
});

test('reviewed project copies use shared native IPC and require explicit approval', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    let job: any;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'local_projects') return { revision: 1, projects: [] };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: 'own-device', routes: [{ node: 'source', url: 'https://source.example.com', allow_private_http: false }, { node: 'target', url: 'https://target.example.com', allow_private_http: false }], grants: [] };
        if (action.operation === 'projects') return { owner: action.node, revision: 1, permissions: { exec: true, projects: true, terminals: false }, projects: action.node === 'source' ? [{ id: 'project', name: '待复制项目', replicas: [{ id: '33333333-3333-4333-8333-333333333333', node: 'source', path: '/projects/source', sessions: [] }] }] : [] };
        if (action.operation === 'copy_list') return { copies: job ? [job] : [] };
        if (action.operation === 'copy_prepare') job = { id: 'copy-1', source: 'source', target: 'target', target_root: '/projects/received', digest: 'a'.repeat(64), state: 'prepared', created_at: 1792180800000, summary: { name: '待复制项目', files: [{ path: 'src/main.rs', size: 64 }], directories: ['src'], skipped: ['.env'], bytes: 64 } };
        if (action.operation === 'copy_commit') job = { ...job, state: 'completed', result: { path: '/projects/received/cranibox-copy/project' } };
        return structuredClone(job);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.getByRole('heading', { name: '待复制项目', exact: true })).toBeVisible();
  const panel = page.getByRole('region', { name: '项目文件复制', exact: true });
  await panel.getByRole('button', { name: '复制项目到设备', exact: true }).click();
  await panel.getByRole('combobox', { name: '目标设备', exact: true }).selectOption('target');
  await panel.getByRole('button', { name: '生成并预览快照', exact: true }).click();
  const commit = panel.getByRole('button', { name: '确认复制此快照', exact: true });
  await expect(commit).toBeDisabled();
  await panel.getByText('检查文件清单', { exact: true }).click();
  await expect(panel.locator('details').first()).toContainText('src/main.rs');
  await panel.getByRole('checkbox').check();
  await commit.click();
  await expect(panel.locator('.projects-copy-review')).toContainText('已完成');
  await expect(panel).toContainText('/projects/received/cranibox-copy/project');
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation.startsWith('copy_')).map((c: any) => c.payload.command)).toEqual([{ operation: 'copy_list' }, { operation: 'copy_prepare', source: 'source', target: 'target', replica_id: '33333333-3333-4333-8333-333333333333' }, { operation: 'copy_commit', id: 'copy-1', digest: 'a'.repeat(64) }]);
  expect(calls.some((c: any) => c.command === 'configure' || c.command === 'terminal_open')).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('/projects/received');
  expect(errors).toEqual([]);
});

test('virtual spaces use shared native IPC and open their enrolled terminal without a relay', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const catalog = { spaces: [] as any[], setup: null, platform: 'macos', default_image: 'cranibox-test:latest' };
    const shell = { id: '11111111-1111-4111-8111-111111111111', name: 'Space Shell', cwd: '/config', status: 'running' };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'spaces') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'preflight') return { ready: true, engine: { name: 'Colima', version: 'test', os: 'Linux' }, detail: 'Ready', notice: '', actions: [] };
        if (payload.command.operation === 'create') catalog.spaces = [{ id: 'space-1', name: payload.command.name, image: payload.command.image, provider: payload.command.provider, node: 'space-node', phase: 'ready', volume: 'persistent-data', error: null, ports: {} }];
        return structuredClone(catalog);
      }
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'list') return { node_id: 'own-device', routes: catalog.spaces.map(space => ({ node: space.node, url: 'http://127.0.0.1:9020' })), grants: [], listener: null };
        if (payload.command.operation === 'sessions') return payload.command.command.operation === 'create' ? { installed: true, session: shell } : { installed: true, sessions: [shell] };
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '虚拟空间', exact: true }).click();
  const section = page.getByRole('region', { name: '虚拟空间', exact: true });
  await section.getByRole('button', { name: '新建空间', exact: true }).click();
  await expect(section.getByRole('radio', { name: /Colima/ })).toBeChecked();
  await section.getByRole('button', { name: '检测环境', exact: true }).click();
  await section.getByLabel('空间名称', { exact: true }).fill('Agent 开发空间');
  await section.getByRole('button', { name: '下一步', exact: true }).click();
  await section.getByRole('button', { name: '创建并启动', exact: true }).click();
  const card = section.getByRole('article', { name: 'Agent 开发空间', exact: true });
  await expect(card.getByRole('status')).toHaveText('可用');
  await card.getByRole('button', { name: '终端', exact: true }).click();
  await expect(page.locator('#floating-terminal .terminal-pane-head')).toContainText('已连接');
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.find((call: any) => call.command === 'spaces' && call.payload.command.operation === 'create').payload.command).toEqual({ operation: 'create', name: 'Agent 开发空间', provider: 'colima', image: 'cranibox-test:latest', pull_policy: 'always' });
  expect(calls.find((call: any) => call.command === 'terminal_open').payload.options).toMatchObject({ direct: true, local: false, node: 'space-node', session_id: '11111111-1111-4111-8111-111111111111' });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_sessions')).toBe(false);
  expect(errors).toEqual([]);
});

test('the shared incoming listener controls use native IPC without a relay', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const catalog = { node_id: 'own-device', routes: [], grants: [], listener: null as any, listener_runtime: { active: false, address: null as string | null, applied: true } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'listener') {
          catalog.listener = payload.command.listener;
          catalog.listener_runtime = { active: !!catalog.listener, address: catalog.listener ? '127.0.0.1:9020' : null, applied: true };
        }
        return structuredClone(catalog);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const listener = page.locator('.direct-listener');
  await listener.getByRole('button', { name: '配置监听', exact: true }).click();
  await expect(listener.getByLabel('监听地址', { exact: true })).toHaveValue('127.0.0.1:9020');
  await listener.getByRole('button', { name: '保存并启用', exact: true }).click();
  await expect(listener.getByRole('status')).toContainText('正在监听 127.0.0.1:9020');
  await listener.getByRole('button', { name: '配置监听', exact: true }).click();
  page.once('dialog', dialog => dialog.accept());
  await listener.getByRole('button', { name: '关闭入站接入', exact: true }).click();
  await expect(listener.getByRole('status')).toHaveText('入站接入已关闭');
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation === 'listener').map((c: any) => c.payload.command)).toEqual([{ operation: 'listener', listener: { address: '127.0.0.1:9020', allow_insecure: false } }, { operation: 'listener', listener: null }]);
  expect(calls.some((c: any) => c.command === 'configure' || c.command === 'reconnect')).toBe(false);
});

test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => console.error('Client script error:', error.stack));
  await page.addInitScript(() => {
    const settings = { server: 'wss://workspace.example.com', id: 'laptop', name: '我的笔记本', token_file: '/private/node.token', allow_insecure: false };
    const catalog = { revision: 1, projects: [{ id: 'project-1', name: '跨设备项目', replicas: [{ id: 'replica-1', node: 'desktop', path: '/home/me/cranibox', sessions: [] as { id: string; terminal_id: string }[] }] }] };
    const shell = { id: '11111111-1111-4111-8111-111111111111', name: '开发 Shell', cwd: '/home/me/cranibox', status: 'running' };
    const callbacks = new Map<number, Function>(); let next = 1, attachment = 0;
    (window as any).calls = [];
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: (callback: Function) => { const id = next++; callbacks.set(id, callback); return id; },
      unregisterCallback: (id: number) => callbacks.delete(id),
      invoke: async (command: string, payload?: any) => {
      (window as any).calls.push({ command, payload: command === 'terminal_open' ? { options: payload.options } : payload });
      if (command === 'snapshot') return { settings, status: { state: 'connected', message: null }, autostart: true, error: null, quit_requested: false };
      if (command === 'peers') return { nodes: [{ id: 'laptop', name: '我的笔记本', online: true, os: 'macos', mesh_group: 'personal', terminal_sessions: false }, { id: 'desktop', name: '开发主机', online: true, os: 'linux', mesh_group: 'personal', terminal_sessions: true }] };
      if (command === 'mcp_config') {
        if ((window as any).failMcp) throw Error('无法生成 MCP 配置，请重试');
        if ((window as any).delayMcp) await new Promise(resolve => setTimeout(resolve, 250));
        return { mcpServers: { 'cranibox-devices': { command: '/apps/cranibox-node-desktop', args: ['mcp', '--config', '/private/settings.json'] }, ...(payload?.includeRuns ? { 'cranibox-runs': { command: '/apps/cranibox-node-desktop', args: ['--config', '/private/settings.json', 'peer', 'runs', 'mcp'] } } : {}) } };
      }
      if (command === 'projects') {
        const action = payload.command;
        if (action.operation === 'bind') { catalog.projects[0].replicas[0].sessions = [{ id: 'binding-1', terminal_id: action.terminal_id }]; catalog.revision++; }
        if (action.operation === 'unbind') { catalog.projects[0].replicas[0].sessions = []; catalog.revision++; }
        return structuredClone(catalog);
      }
      if (command === 'terminal_sessions') return payload.command.operation === 'create' ? { installed: true, session: shell } : { installed: true, sessions: [shell] };
      if (command === 'terminal_open') {
        const callback = callbacks.get(payload.events.id)!;
        callback({ index: 0, message: { type: 'ready' } });
        callback({ index: 1, message: { type: 'output', data: btoa('gateway Shell') } });
        let index = 2;
        (window as any).disconnectTerminal = () => callback({ index: index++, message: { type: 'closed' } });
        return 'gateway-' + ++attachment;
      }
      return null;
    } };
  });
});
test('device networking uses native IPC and continues after the view closes', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const status = { node_id: 'own-device', easytier: { installed: true, enabled: false, phase: 'stopped', version: '2.6.4', error: null, devices: [], connections: [] }, tailscale: { installed: true, state: 'NeedsLogin', devices: [], login_pending: false, login_url: '' } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') return { node_id: 'own-device', routes: [], grants: [] };
      if (command === 'network') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'easy_tier_join') Object.assign(status.easytier, { enabled: true, phase: 'running', network: payload.command.network, peer: payload.command.peers[0], peers: payload.command.peers });
        if (payload.command.operation === 'tailscale_login') Object.assign(status.tailscale, { login_pending: true, login_url: 'https://login.tailscale.com/a/native-fixture' });
        return structuredClone(status);
      }
      if (command === 'open_network_link') { (window as any).calls.push({ command, payload }); return; }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: '设备组网', exact: true });
  await expect(panel).toContainText('网页关闭后后台继续连接');
  const et = panel.getByRole('article', { name: 'EasyTier', exact: true });
  await et.getByRole('button', { name: '手动填写地址', exact: true }).click();
  await et.getByLabel('网络入口', { exact: false }).fill('tcp://entry.example.com:11010\nwss://backup.example.com/mesh');
  await et.getByLabel('网络 Token', { exact: false }).first().fill('c'.repeat(64));
  await et.getByRole('button', { name: '连接 EasyTier', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('后台运行中');
  const ts = panel.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('button', { name: '登录 Tailscale', exact: true }).click();
  await ts.getByRole('link', { name: '打开 Tailscale 登录', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.find((c: any) => c.command === 'open_network_link')?.payload.url)).toBe('https://login.tailscale.com/a/native-fixture');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('后台运行中');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'network' && c.payload.command.operation !== 'status'));
  expect(calls.map((c: any) => c.payload.command)).toEqual([{ operation: 'easy_tier_join', network: 'cranibox', token: 'c'.repeat(64), peers: ['tcp://entry.example.com:11010', 'wss://backup.example.com/mesh'], listen_port: null }, { operation: 'tailscale_login' }]);
  await expect(et.locator('.network-saved')).toContainText('wss://backup.example.com/mesh');
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('c'.repeat(64));
});
test('continuous Tailscale discovery is saved through native IPC and restored after navigation', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const status = { node_id: 'own-device', easytier: { installed: false, enabled: false, phase: 'stopped', version: '2.6.4', error: null, devices: [], connections: [] }, tailscale: { state: 'Running', devices: [{ name: 'Tailnet Mac', addresses: ['100.64.0.2'], online: true, path: 'direct' }], discovery: { enabled: false, port: null as number | null, phase: 'disabled', devices: [] as any[], observed_at: null as number | null, checked: 0, skipped: 0, ambiguous: 0, error: null } } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'network') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'tailscale_discovery') Object.assign(status.tailscale.discovery, { enabled: payload.command.ports.length > 0, port: payload.command.ports[0] ?? null, ports: payload.command.ports, phase: payload.command.ports.length ? 'ready' : 'disabled', observed_at: payload.command.ports.length ? Math.floor(Date.now() / 1000) : null, devices: payload.command.ports.length ? [{ node_id: 'remote-mac', address: '100.64.0.2', port: 1234, url: 'http://100.64.0.2:1234' }] : [], checked: 1 });
        return structuredClone(status);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: 'Tailscale 持续发现', exact: true });
  await panel.getByLabel('节点监听端口', { exact: true }).fill('1234, 4321');
  await panel.getByRole('button', { name: '启用持续发现', exact: true }).click();
  await expect(panel.getByRole('list')).toContainText('remote-mac');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await expect(panel.getByLabel('节点监听端口', { exact: true })).toHaveValue('1234, 4321');
  await panel.getByRole('button', { name: '停止发现', exact: true }).click();
  await expect(panel).toContainText('持续发现未启用');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'network' && c.payload.command.operation !== 'status'));
  expect(calls.map((c: any) => c.payload.command)).toEqual([{ operation: 'tailscale_discovery', ports: [1234, 4321] }, { operation: 'tailscale_discovery', ports: [] }]);
});
test('Tailscale discovery and authorized connection use native IPC without a gateway', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    let connected = false;
    const status = { node_id: 'own-device', easytier: { installed: false, enabled: false, phase: 'stopped', version: '2.6.4', error: null, devices: [], connections: [] }, tailscale: { installed: true, state: 'Running', devices: [{ name: 'Tailnet Mac', addresses: ['100.64.0.2'], online: true, path: 'direct' }] } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') return { node_id: 'own-device', routes: connected ? [{ node: 'remote-mac', url: 'http://100.64.0.2:1234' }] : [], grants: [] };
      if (command === 'network') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'tailscale_discover') return { node_id: 'remote-mac', address: '100.64.0.2', port: payload.command.port, url: 'http://100.64.0.2:' + payload.command.port };
        if (payload.command.operation === 'tailscale_connect') { connected = true; return { network: structuredClone(status), catalog: {} }; }
        return structuredClone(status);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const ts = page.getByRole('article', { name: 'Tailscale', exact: true });
  await ts.getByRole('button', { name: '查找节点', exact: true }).click();
  const form = ts.getByRole('form', { name: '接入 Tailscale 节点', exact: true });
  await form.getByLabel('目标节点监听端口', { exact: false }).fill('1234');
  await form.getByRole('button', { name: '核对节点身份', exact: true }).click();
  await expect(form).toContainText('remote-mac'); await expect(form).toContainText('own-device');
  await form.getByLabel('目标访问 Token', { exact: true }).fill('c'.repeat(64));
  await form.getByRole('button', { name: '保存设备连接', exact: true }).click();
  await expect(form).toHaveCount(0); await expect(page.locator('.direct-list')).toContainText('remote-mac');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await expect(ts).toContainText('Tailnet Mac');
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'network' && c.payload.command.operation !== 'status'));
  expect(calls.map((c: any) => c.payload.command)).toEqual([{ operation: 'tailscale_discover', address: '100.64.0.2', port: 1234 }, { operation: 'tailscale_connect', address: '100.64.0.2', port: 1234, node: 'remote-mac', token: 'c'.repeat(64) }]);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('c'.repeat(64));
});
test('network invitations join through native IPC without browser requests or stored credentials', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const status = { node_id: 'own-device', easytier: { installed: true, enabled: false, phase: 'stopped', version: '2.6.4', error: null, devices: [], connections: [] }, tailscale: { state: 'NotInstalled', devices: [] } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') return { node_id: 'own-device', routes: [], grants: [] };
      if (command === 'network') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'easy_tier_enroll') Object.assign(status.easytier, { enabled: true, phase: 'running', network: 'home', peer: 'tcp://entry:11010', peers: ['tcp://entry:11010'] });
        return structuredClone(status);
      }
      return previous(command, payload);
    };
  });
  const externalRequests: string[] = [];
  page.on('request', request => { if (request.url().startsWith('https://workspace.example.com')) externalRequests.push(request.url()); });
  await page.goto('/');
  const et = page.getByRole('article', { name: 'EasyTier', exact: true });
  await expect(et).toContainText('若连接中断，在原设备使用相同地址和邀请重试');
  await et.getByLabel('Workspace 地址', { exact: true }).fill('https://workspace.example.com');
  await et.getByLabel('邀请 Token', { exact: true }).fill('cbi_' + 'd'.repeat(64));
  await et.getByRole('button', { name: '通过邀请加入', exact: true }).click();
  await expect(et.getByRole('status')).toHaveText('后台运行中');
  await expect(et.getByLabel('邀请 Token', { exact: true })).toHaveCount(0);
  const enrollments = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'network' && call.payload.command.operation === 'easy_tier_enroll'));
  expect(enrollments.map((call: any) => call.payload.command)).toEqual([{ operation: 'easy_tier_enroll', url: 'https://workspace.example.com', token: 'cbi_' + 'd'.repeat(64), listen_port: null }]);
  expect(externalRequests).toEqual([]);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('d'.repeat(64));
});

test('shared project view links shells and opens their original device', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.getByRole('heading', { name: '跨设备项目', exact: true })).toBeVisible();
  await expect(page.locator('.projects-location')).toContainText('开发主机');
  await page.getByLabel('关联已有会话', { exact: true }).selectOption('11111111-1111-4111-8111-111111111111');
  await page.getByRole('button', { name: '关联', exact: true }).click();
  await expect(page.locator('.projects-sessions')).toContainText('开发 Shell');
  await page.getByRole('button', { name: '打开', exact: true }).click();
  const floating = page.locator('#floating-terminal');
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  const attachment = await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'terminal_open'));
  expect(attachment.payload.options).toMatchObject({ node: 'desktop', direct: false, session_id: '11111111-1111-4111-8111-111111111111' });
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  await page.getByRole('button', { name: '解除关联 开发 Shell', exact: true }).click();
  await expect(page.locator('.projects-sessions li')).toHaveCount(0);
  await page.getByRole('button', { name: '在此目录新建 Shell', exact: true }).click();
  await expect(page.locator('.projects-sessions')).toContainText('开发 Shell');
  const creation = await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'terminal_sessions' && call.payload.command.operation === 'create'));
  expect(creation.payload.node).toBe('desktop');
  expect(creation.payload.command.cwd).toBe('/home/me/cranibox');
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: '/tmp/cranibox-projects-desktop-' + width + '.png', fullPage: true });
  }
  expect(errors).toEqual([]);
});
test('the shared native project view uses direct catalogs and terminals without a gateway', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const shell = { id: '11111111-1111-4111-8111-111111111111', name: '直连项目 Shell', cwd: '/Users/me/project', status: 'running' };
    const catalog = { owner: 'remote-mac', revision: 1, permissions: { projects: true, terminals: true }, projects: [{ id: '22222222-2222-4222-8222-222222222222', name: '无需网关的项目', replicas: [{ id: '33333333-3333-4333-8333-333333333333', node: 'remote-mac', path: shell.cwd, sessions: [] as any[] }] }] };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, status: { state: 'idle' }, autostart: false, error: null, quit_requested: false };
      if (command === 'projects') throw Error('Workspace 网关不可用');
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'list') return { node_id: 'local-id', routes: [{ node: 'remote-mac', url: 'https://mac.example.com' }] };
        if (payload.command.operation === 'sessions') return { installed: true, sessions: [shell] };
        if (payload.command.command.operation === 'bind') { catalog.projects[0].replicas[0].sessions = [{ id: 'binding', terminal_id: payload.command.command.terminal_id }]; catalog.revision++; }
        return structuredClone(catalog);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.getByRole('heading', { name: '无需网关的项目', exact: true })).toBeVisible();
  await expect(page.locator('.projects-sources')).toContainText('1/1 已读取');
  await expect(page.locator('.projects-location')).toContainText('remote-mac · 直连');
  await page.getByLabel('关联已有会话', { exact: true }).selectOption('11111111-1111-4111-8111-111111111111');
  await page.getByRole('button', { name: '关联', exact: true }).click();
  await page.locator('.projects-sessions').getByRole('button', { name: '打开', exact: true }).click();
  await expect(page.locator('#floating-terminal .terminal-pane-head')).toContainText('已连接');
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.find((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'bind').payload.command).toEqual({ operation: 'projects', node: 'remote-mac', command: { operation: 'bind', replica_id: '33333333-3333-4333-8333-333333333333', terminal_id: '11111111-1111-4111-8111-111111111111' } });
  expect(calls.find((call: any) => call.command === 'terminal_open').payload.options).toMatchObject({ direct: true, node: 'remote-mac', session_id: '11111111-1111-4111-8111-111111111111' });
  await page.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  expect(calls.some((call: any) => call.command === 'terminal_sessions' || call.payload?.command?.operation === 'create')).toBe(false);
});
test('this device manages its projects, Shell and autostart without configuring a relay', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const catalog = { revision: 0, projects: [] as any[] };
    const shell = { id: '11111111-1111-4111-8111-111111111111', name: '本机项目 Shell', cwd: '/Users/me/project', status: 'running' };
    let autostart = false;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart, error: null, quit_requested: false };
      if (command === 'projects' || command === 'terminal_sessions' || command === 'configure') throw Error('Gateway must not be used');
      if (command === 'direct_peers') return { node_id: 'own-device', routes: [], grants: [] };
      if (command === 'set_autostart') { (window as any).calls.push({ command, payload }); autostart = payload.enabled; return; }
      if (command === 'local_projects') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'register') {
          catalog.projects.push({ id: '22222222-2222-4222-8222-222222222222', name: action.name, replicas: [{ id: '33333333-3333-4333-8333-333333333333', node: 'own-device', path: action.path, sessions: [] }] }); catalog.revision++;
        }
        if (action.operation === 'bind') { catalog.projects[0].replicas[0].sessions = [{ id: 'binding', terminal_id: action.terminal_id }]; catalog.revision++; }
        return structuredClone(catalog);
      }
      if (command === 'local_sessions') {
        (window as any).calls.push({ command, payload });
        return payload.command.operation === 'create' ? { installed: true, session: shell } : { installed: true, sessions: [shell] };
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await expect(page.getByRole('region', { name: '这台设备', exact: true })).toContainText('own-device');
  await expect(page.getByRole('region', { name: '节点连接状态' })).toContainText('本机服务运行中');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '连接设置', exact: true }).click();
  await expect(page.getByLabel('节点 ID', { exact: true })).toHaveValue('own-device');
  await page.getByRole('checkbox', { name: /登录系统后自动启动/ }).check();
  await expect(page.getByRole('checkbox', { name: /登录系统后自动启动/ })).toBeChecked();
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.locator('.projects-sources')).toContainText('1/1 已读取');
  await page.getByRole('button', { name: '注册项目', exact: true }).click();
  await page.getByLabel('项目名称', { exact: true }).fill('离线也能用的项目');
  await expect(page.getByLabel('所在设备', { exact: true })).toHaveValue('local:own-device');
  await page.getByLabel('设备上的绝对目录', { exact: true }).fill('/Users/me/project');
  await page.getByRole('button', { name: '保存目录', exact: true }).click();
  await expect(page.locator('.projects-location')).toContainText('这台设备');
  await page.getByRole('button', { name: '在此目录新建 Shell', exact: true }).click();
  const floating = page.locator('#floating-terminal');
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  await page.locator('.projects-sessions').getByRole('button', { name: '打开', exact: true }).click();
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: '/tmp/cranibox-own-project-' + width + '.png', fullPage: true });
  }
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((call: any) => call.command === 'terminal_open')).toHaveLength(2);
  expect(calls.filter((call: any) => call.command === 'terminal_open').every((call: any) => call.payload.options.local && !call.payload.options.direct && call.payload.options.node === 'own-device' && call.payload.options.session_id === '11111111-1111-4111-8111-111111111111')).toBe(true);
  expect(calls.filter((call: any) => call.command === 'local_sessions' && call.payload.command.operation === 'create')).toHaveLength(1);
  expect(calls.find((call: any) => call.command === 'local_projects' && call.payload.command.operation === 'bind').payload.command.replica_id).toBe('33333333-3333-4333-8333-333333333333');
  expect(calls.some((call: any) => call.command === 'configure' || call.payload?.command?.operation === 'end')).toBe(false);
  expect(errors).toEqual([]);
});

test('native project snapshots stay read-only and refresh after the owner reconnects', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    (window as any).projectOffline = true;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, status: { state: 'idle' }, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'list') return { node_id: 'own-device', routes: [{ node: 'remote-mac', url: 'https://mac.example.com', allow_private_http: false }] };
        if (payload.command.operation === 'sessions') return { installed: true, sessions: [] };
        const offline = (window as any).projectOffline;
        return { owner: 'remote-mac', revision: 1, permissions: { projects: true, terminals: !offline },
          ...(offline ? { sync: { state: 'offline', synced_at: 1792180800000 } } : {}),
          projects: [{ id: 'project', name: '保留的项目目录', replicas: [{ id: 'copy', node: 'remote-mac', path: '/Users/me/private-offline-project', sessions: [] }] }] };
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.locator('.projects-snapshot')).toContainText('上次同步：');
  await expect(page.getByRole('button', { name: '移除目录记录', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '在此目录新建 Shell', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '注册项目', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await expect(page.locator('.projects-location')).toContainText('/Users/me/private-offline-project');
  expect(await page.evaluate(() => (window as any).calls.some((c: any) => c.command === 'direct_peers' && c.payload.command.operation === 'sessions'))).toBe(false);
  await page.evaluate(() => { (window as any).projectOffline = false; });
  await page.getByRole('button', { name: '刷新项目', exact: true }).click();
  await expect(page.locator('.projects-snapshot')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '移除目录记录', exact: true })).toBeEnabled();
  expect(await page.evaluate(() => (window as any).calls.some((c: any) => c.command === 'projects' || c.command === 'terminal_sessions' || c.payload?.command?.command?.operation === 'create'))).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('/Users/me/private-offline-project');
});
test('migrated gateway identities still mark this device and preserve its saved registration', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: { server: 'wss://workspace.example.com', id: 'legacy-laptop', name: '我的笔记本', token_file: '/private/node.token', allow_insecure: false }, local: { id: 'node-persistent', online: true }, status: { state: 'connected' }, autostart: false, error: null, quit_requested: false };
      if (command === 'peers') return { nodes: [{ id: 'node-persistent', aliases: ['legacy-laptop'], name: '我的笔记本', online: true, terminal_sessions: true }] };
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const device = page.locator('.devices li').filter({ hasText: 'node-persistent' });
  await expect(device.locator('.tag')).toHaveText('本机');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '连接设置', exact: true }).click();
  await expect(page.getByLabel('节点 ID', { exact: true })).toHaveValue('legacy-laptop');
  await expect(page.getByLabel('节点 ID', { exact: true }).locator('..')).toContainText('node-persistent');
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'configure'))).toBe(false);
});
test('gateway devices open in the app and reconnect to the same Shell', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('button', { name: '打开 我的笔记本 的终端', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '打开 开发主机 的终端', exact: true }).click();
  const floating = page.locator('#floating-terminal');
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: 'Ctrl+C', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.some((c: any) => c.command === 'terminal_input' && c.payload.input.data === '\x03'))).toBe(true);
  await page.evaluate(() => (window as any).disconnectTerminal());
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_open').length)).toBe(2);
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: '最小化终端窗口', exact: true }).click();
  const count = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_close').length);
  await page.getByRole('button', { name: '项目与会话', exact: true }).click();
  await page.getByRole('button', { name: '恢复终端窗口', exact: true }).click();
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_close').length)).toBe(count);
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  for (const width of [375, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole('button', { name: '设备网络', exact: true }).click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: '/tmp/cranibox-gateway-devices-' + width + '.png', fullPage: true });
  }
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((c: any) => c.command === 'terminal_open').every((c: any) => c.payload.options.node === 'desktop' && !c.payload.options.direct && c.payload.options.session_id === '11111111-1111-4111-8111-111111111111')).toBe(true);
  expect(calls.some((c: any) => c.command === 'open_terminal' || c.command === 'terminal_sessions' && ['create', 'end'].includes(c.payload.command.operation))).toBe(false);
});
test('devices, MCP and reconnect preserve the service', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: '我的笔记本' })).toBeVisible();
  await expect(page.getByText('开发主机')).toBeVisible();
  await page.getByRole('button', { name: '重新连接', exact: true }).click();
  await page.getByRole('button', { name: 'Agent 接入', exact: true }).click();
  expect(await page.getByRole('textbox', { name: 'MCP 配置' }).inputValue()).toContain('cranibox-devices');
  expect(await page.getByRole('textbox', { name: 'MCP 配置' }).inputValue()).not.toContain('token');
  await page.getByRole('button', { name: '收起到系统托盘' }).click();
  const calls = await page.evaluate(() => (window as any).calls.map((call: any) => call.command));
  expect(calls).toContain('reconnect'); expect(calls).toContain('hide_window'); expect(calls).not.toContain('quit');
});
test('Agent execution MCP is opt-in, clears stale config and fits small screens', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Agent 接入', exact: true }).click();
  const option = page.getByRole('checkbox', { name: '添加 Agent 后台运行工具' });
  const config = page.getByRole('textbox', { name: 'MCP 配置' });
  await expect(option).not.toBeChecked();
  await expect(config).not.toHaveValue('');
  expect(await config.inputValue()).not.toContain('cranibox-runs');
  await page.evaluate(() => { (window as any).delayMcp = true; });
  await option.check();
  await expect(page.getByRole('button', { name: '复制配置', exact: true })).toBeDisabled();
  await expect(config).toHaveValue('');
  await expect(config).toHaveValue(/cranibox-runs/);
  const value = JSON.parse(await config.inputValue());
  expect(value.mcpServers['cranibox-runs'].args).toEqual(['--config', '/private/settings.json', 'peer', 'runs', 'mcp']);
  await page.evaluate(() => { (window as any).delayMcp = false; (window as any).failMcp = true; });
  await option.uncheck();
  await expect(page.getByRole('alert')).toContainText('无法生成 MCP 配置');
  await expect(config).toHaveValue('');
  await expect(page.getByRole('button', { name: '复制配置', exact: true })).toBeDisabled();
  await page.evaluate(() => { (window as any).failMcp = false; });
  await option.check();
  await expect(config).toHaveValue(/cranibox-runs/);
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(option).toBeVisible();
    await page.screenshot({ path: '/tmp/cranibox-agent-runs-mcp-' + width + '.png', fullPage: true });
  }
  await option.uncheck();
  await expect(config).not.toHaveValue(/cranibox-runs/);
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.some((call: any) => call.command === 'mcp_config' && call.payload?.includeRuns === true)).toBe(true);
  expect(calls.some((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'executions')).toBe(false);
});
test('quitting requires confirmation and cancel keeps the service', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: '退出客户端', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).not.toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'quit'))).toBe(false);
  await page.getByRole('button', { name: '退出客户端', exact: true }).click();
  await page.getByRole('button', { name: '退出并结束会话', exact: true }).click();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'quit'))).toBe(true);
});
test('narrow layout and credential saving', async ({ page }) => {
  await page.setViewportSize({ width: 440, height: 780 });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '连接设置', exact: true }).click();
  await page.getByLabel('节点 Token', { exact: true }).fill('a'.repeat(64));
  await page.getByRole('button', { name: '保存并连接', exact: true }).click();
  await expect(page.getByText('设置已保存，后台节点正在连接。')).toBeVisible();
  await page.getByRole('button', { name: '连接设置', exact: true }).click();
  await expect(page.getByLabel('节点 Token', { exact: true })).toHaveValue('');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.setViewportSize({ width: 880, height: 720 });
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await page.screenshot({ path: '/tmp/cranibox-node-desktop-preview.png', fullPage: true });
});

test('direct devices work through IPC without a configured Workspace relay', async ({ page }) => {
  await page.addInitScript(() => {
    const previous = (window as any).__TAURI_INTERNALS__.invoke;
    const catalog = { node_id: 'desktop-direct-id', routes: [{ node: 'remote-mac', url: 'https://mac.example.com', allow_private_http: false }] };
    (window as any).__TAURI_INTERNALS__.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, status: { state: 'idle' }, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'probe') return { nodes: [{ id: 'remote-mac', os: 'macos', arch: 'aarch64', exec: true, terminal_sessions: false }] };
        if (['exec', 'job'].includes(action.operation)) return { node: 'remote-mac', id: '11111111-1111-4111-8111-111111111111', outcome: 'exited', exit_code: 0, stdout: 'IPC direct output' };
        return catalog;
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: '直连设备', exact: true });
  await expect(panel.locator('.direct-identity')).toContainText('desktop-direct-id');
  await panel.getByRole('button', { name: '检查连接', exact: true }).click();
  await expect(panel.locator('.direct-probe')).toContainText('允许执行命令');
  await panel.getByLabel('程序', { exact: true }).fill('hostname');
  await panel.getByRole('button', { name: '执行直连任务', exact: true }).click();
  await expect(panel.getByLabel('标准输出', { exact: true })).toContainText('IPC direct output');
  await page.reload();
  await expect(panel.getByLabel('标准输出', { exact: true })).toContainText('IPC direct output');
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation === 'exec'))).toHaveLength(0);
  await page.setViewportSize({ width: 375, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: '/tmp/cranibox-direct-desktop-mobile.png', fullPage: true });
});

test('shared incoming access uses native IPC with explicit terminal permission', async ({ page }) => {
  await page.addInitScript(() => {
    const previous = (window as any).__TAURI_INTERNALS__.invoke;
    const catalog = { node_id: 'desktop-direct-id', routes: [], grants: [] as any[], listener: { address: '127.0.0.1:9140', allow_insecure: false } };
    (window as any).__TAURI_INTERNALS__.invoke = async (command: string, payload: any) => {
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push({ command, payload });
      const action = payload.command;
      if (action.operation === 'grant') {
        catalog.grants = [{ node: action.node, permissions: action.permissions }];
        return { catalog: structuredClone(catalog), node: action.node, token: 'c'.repeat(64) };
      }
      if (action.operation === 'revoke') catalog.grants = [];
      return structuredClone(catalog);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: '入站授权', exact: true });
  await expect(panel).toContainText('127.0.0.1:9140');
  await panel.getByRole('button', { name: '授权设备', exact: true }).click();
  await panel.getByLabel(/^被授权设备 ID/).fill('another-desktop');
  await panel.getByLabel('管理和接入终端会话', { exact: true }).check();
  await panel.getByRole('button', { name: '创建授权 Token', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '保存授权 Token', exact: true });
  await expect(dialog.getByLabel('新授权 Token', { exact: true })).toHaveValue('c'.repeat(64));
  await page.setViewportSize({ width: 375, height: 850 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: '/tmp/cranibox-incoming-desktop-375.png' });
  await dialog.getByRole('button', { name: '已保存，关闭', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  const calls = await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'direct_peers' && c.payload.command.operation === 'grant'));
  expect(calls).toEqual([{ command: 'direct_peers', payload: { command: { operation: 'grant', node: 'another-desktop', permissions: { exec: false, terminals: true, projects: false, desktop: false, computer: false, browser: false, messages: false, spaces: false } } } }]);
  page.once('dialog', dialog => dialog.accept());
  await panel.getByRole('button', { name: '撤销授权 another-desktop', exact: true }).click();
  await expect(panel.getByLabel('已授权设备', { exact: true })).toContainText('尚未授权其他设备');
});

test('direct terminal shares the floating UI and detaches without ending its Shell', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const callbacks = new Map<number, Function>(); let next = 1, attachment = 0;
    internals.transformCallback = (callback: Function) => { const id = next++; callbacks.set(id, callback); return id; };
    internals.unregisterCallback = (id: number) => callbacks.delete(id);
    const shell = { id: '11111111-1111-4111-8111-111111111111', name: '原有 Shell', cwd: '/home/me/project', status: 'running' };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, status: { state: 'idle' }, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'probe') return { nodes: [{ id: 'remote-mac', exec: false, terminal_sessions: true }] };
        if (payload.command.operation === 'sessions') return { installed: true, sessions: [shell] };
        return { node_id: 'local-id', routes: [{ node: 'remote-mac', url: 'https://mac.example.com', allow_private_http: false }], grants: [] };
      }
      if (command === 'terminal_open') {
        (window as any).calls.push({ command, payload: { options: payload.options } });
        const callback = callbacks.get(payload.events.id)!;
        callback({ index: 0, message: { type: 'ready' } });
        callback({ index: 1, message: { type: 'output', data: btoa('same native Shell') } });
        return 'attachment-' + ++attachment;
      }
      if (command.startsWith('terminal_')) { (window as any).calls.push({ command, payload }); return null; }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const panel = page.getByRole('region', { name: '直连设备', exact: true });
  await expect(panel.getByRole('button', { name: '打开终端', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: '检查连接', exact: true }).click();
  await panel.getByRole('button', { name: '打开终端', exact: true }).click();
  const floating = page.locator('#floating-terminal');
  await expect(floating.getByRole('tab', { name: '原有 Shell', exact: true })).toBeVisible();
  await expect(floating.locator('.terminal-pane-head')).toContainText('已连接');
  await floating.getByRole('button', { name: 'Ctrl+C', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.some((c: any) => c.command === 'terminal_input' && c.payload.input.data === '\x03'))).toBe(true);
  await floating.getByRole('button', { name: '最小化终端窗口', exact: true }).click();
  await expect(floating).not.toBeVisible();
  expect(await page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_close'))).toHaveLength(0);
  await page.getByRole('button', { name: '恢复终端窗口', exact: true }).click();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const width of [375, 1440]) {
    await page.setViewportSize({ width, height: 850 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: '/tmp/cranibox-native-terminal-' + width + '.png' });
  }
  await floating.getByRole('button', { name: '关闭终端窗口', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_close').length)).toBe(1);
  await page.getByRole('button', { name: '打开终端窗口', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((c: any) => c.command === 'terminal_open').length)).toBe(2);
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((c: any) => c.command === 'terminal_open').every((c: any) => c.payload.options.node === 'remote-mac' && c.payload.options.direct && c.payload.options.session_id === '11111111-1111-4111-8111-111111111111')).toBe(true);
  expect(calls.some((c: any) => c.command === 'direct_peers' && ['create', 'end'].includes(c.payload.command.command?.operation))).toBe(false);
  expect(errors).toEqual([]);
});

test('relay desktop devices share one list with direct routes and expose actionable errors', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: { server: 'wss://workspace.example.com', id: 'own-device' }, local: { id: 'own-device', online: true }, status: { state: 'connected' }, autostart: true, error: null, quit_requested: false };
      if (command === 'peers') return { nodes: [
        { id: 'target', name: '测试工作站', online: true, desktop_relay: true },
        { id: 'offline', name: '离线电脑', online: false, desktop_relay: true },
        { id: 'legacy', name: '旧版电脑', online: true },
      ] };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'list') return { node_id: 'own-device', routes: [{ node: 'target', url: 'https://target.example.com' }], grants: [], desktop: null };
      }
      if (command === 'desktop_open') {
        (window as any).calls.push({ command, payload: { options: payload.options } });
        throw Error('desktop capability not granted');
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  const section = page.getByRole('region', { name: '跨设备桌面', exact: true });
  const target = section.getByRole('button').filter({ hasText: '测试工作站' });
  await expect(target).toHaveCount(1);
  await target.click();
  await section.getByLabel('连接方式', { exact: true }).selectOption('relay');
  await expect(section.locator('.desktop-route-options')).toContainText('own-device');
  await section.getByRole('button', { name: '连接桌面', exact: true }).click();
  await expect(section.locator('.vnc-viewer [role="alert"]')).toContainText('desktop capability not granted');
  expect(await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'desktop_open').payload.options)).toEqual({ node: 'target', local: false, relay: true });
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'probe'))).toBe(false);
  await section.getByLabel('连接方式', { exact: true }).selectOption('direct');
  await expect(section.locator('.vnc-viewer')).toHaveCount(0);
  await expect(section.locator('.desktop-route-options')).toContainText('已保存的地址');
  await section.getByRole('button').filter({ hasText: '离线电脑' }).click();
  await expect(section.getByRole('button', { name: '连接桌面', exact: true })).toBeDisabled();
  await expect(section.locator('.desktop-route-options')).toContainText('设备离线');
  await section.getByRole('button').filter({ hasText: '旧版电脑' }).click();
  await expect(section.getByRole('button', { name: '连接桌面', exact: true })).toBeDisabled();
  await expect(section.locator('.desktop-route-options')).toContainText('升级目标设备客户端');
});

test('leaving a pending native desktop releases the attachment when IPC finishes', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') return { node_id: 'own-device', routes: [], grants: [], desktop: { address: '127.0.0.1:5900' } };
      if (command === 'desktop_open') {
        (window as any).calls.push({ command, payload: { options: payload.options } });
        await new Promise<void>(resolve => { (window as any).completeDesktop = resolve; });
        return 'pending-desktop-attachment';
      }
      if (command === 'desktop_close') { (window as any).calls.push({ command, payload }); return; }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  await page.getByRole('button', { name: '连接桌面', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).completeDesktop)).toBe('function');
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await page.evaluate(() => (window as any).completeDesktop());
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_close').map((call: any) => call.payload))).toEqual([{ id: 'pending-desktop-attachment' }]);
  expect(errors).toEqual([]);
});

test('the native Selkies channel decodes fragmented JPEG frames and batches pasted input', async ({ page }) => {
  const { readFileSync } = await import('node:fs');
  const images: string[] = JSON.parse(readFileSync(new URL('../../../addons/control-center/ui/tests/selkies-frames.json', import.meta.url), 'utf8'));
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(images => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const callbacks = new Map<number, Function>(); let next = 1, index = 0, sequence = 0, pending = false, typed = false;
    let callback: Function; const queue: Uint8Array[] = [];
    let input = new Uint8Array(), clipboard = '';
    (window as any).clipboardWrites = [];
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (value: string) => { (window as any).clipboardWrites.push(value); } } });
    internals.transformCallback = (fn: Function) => { const id = next++; callbacks.set(id, fn); return id; };
    internals.unregisterCallback = (id: number) => callbacks.delete(id);
    function output(bytes: Uint8Array) { queue.push(bytes); flush(); }
    function flush() {
      if (pending || !queue.length) return; pending = true;
      let binary = ''; for (const byte of queue.shift()!) binary += String.fromCharCode(byte);
      callback({ index: index++, message: { type: 'data', data: btoa(binary), sequence: ++sequence } });
    }
    function frame(kind: number, data: Uint8Array) {
      const bytes = new Uint8Array(5 + data.length); bytes[0] = kind;
      new DataView(bytes.buffer).setUint32(1, data.length); bytes.set(data, 5);
      output(bytes.subarray(0, 3)); output(bytes.subarray(3));
    }
    function text(value: string) { frame(0, new TextEncoder().encode(value)); }
    (window as any).replyClipboard = (value?: string) => text(value ?? 'clipboard,' + clipboard);
    function picture() {
      const image = Uint8Array.from(atob(images[typed ? 1 : 0]), char => char.charCodeAt(0));
      const bytes = new Uint8Array(6 + image.length); bytes.set([3, 0, 0, typed ? 2 : 1, 0, 0]); bytes.set(image, 6); frame(1, bytes);
    }
    const catalog = { node_id: 'own-device', routes: [], grants: [], desktop: { address: '127.0.0.1:8082', protocol: 'selkies' } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') return catalog;
      if (command === 'desktop_open') {
        callback = callbacks.get(payload.events.id)!; callback({ index: index++, message: { type: 'ready' } });
        const magic = new TextEncoder().encode('CRANIBOX-SELKIES/1\n'); output(magic.subarray(0, 3)); output(magic.subarray(3));
        text('MODE websockets'); return 'selkies-attachment';
      }
      if (command === 'desktop_ack') {
        if (!pending || payload.sequence !== sequence) throw Error('Out-of-order Selkies acknowledgement');
        pending = false; flush(); return;
      }
      if (command === 'desktop_input') {
        const chunk = Uint8Array.from(atob(payload.data), char => char.charCodeAt(0)), bytes = new Uint8Array(input.length + chunk.length);
        bytes.set(input); bytes.set(chunk, input.length); input = bytes;
        const view = new DataView(bytes.buffer);
        const commands: string[] = [];
        let offset = 0;
        while (offset + 5 <= bytes.length) {
          const size = view.getUint32(offset + 1); if (bytes[offset] !== 0) throw Error('Invalid input frame');
          if (offset + 5 + size > bytes.length) break;
          commands.push(new TextDecoder().decode(bytes.subarray(offset + 5, offset + 5 + size))); offset += 5 + size;
        }
        input = bytes.slice(offset);
        (window as any).calls.push({ command, commands });
        for (const value of commands) {
          if (value.startsWith('SETTINGS,')) { text(JSON.stringify({ type: 'stream_resolution', width: 160, height: 100 })); picture(); }
          if (value === 'kd,97' && !typed) { typed = true; picture(); }
          if (value.startsWith('cw,')) { clipboard = value.slice(3); text('clipboard,' + clipboard); }
          if (value === 'cr' && !(window as any).holdClipboardFetch) text('clipboard,' + clipboard);
        }
        return;
      }
      if (command === 'desktop_close') { (window as any).calls.push({ command, payload }); return; }
      return previous(command, payload);
    };
  }, images);
  await page.goto('/'); await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  const section = page.getByRole('region', { name: '跨设备桌面', exact: true });
  await section.getByRole('button', { name: '连接桌面', exact: true }).click();
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('已连接');
  const red = () => section.locator('canvas').evaluate((canvas: HTMLCanvasElement) => canvas.getContext('2d')!.getImageData(10, 10, 1, 1).data[0]);
  expect(await red()).toBeLessThan(100);
  await section.getByRole('button', { name: '键盘', exact: true }).click();
  await section.getByLabel('远程键盘输入', { exact: true }).fill('cranibox-selkies-input-20261018');
  await expect.poll(red).toBeGreaterThan(180);
  const batches = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_input').map((call: any) => call.commands));
  expect(batches.flat().filter((command: string) => command.startsWith('kd,')).length).toBe('cranibox-selkies-input-20261018'.length);
  expect(batches.length).toBeLessThan(10);
  await section.getByRole('button', { name: '剪贴板', exact: true }).click();
  const panel = section.getByRole('region', { name: '桌面文字剪贴板', exact: true });
  const unicode = '原生剪贴板 👋\n' + 'a'.repeat(65400);
  await panel.getByLabel('发送到远端', { exact: true }).fill(unicode);
  await panel.getByRole('button', { name: '发送文字', exact: true }).click();
  await expect(panel.getByLabel('远端剪贴板', { exact: true })).toHaveValue(unicode);
  expect(await page.evaluate(() => (window as any).clipboardWrites)).toEqual([]);
  await panel.getByRole('button', { name: '复制到本机', exact: true }).click();
  expect(await page.evaluate(() => (window as any).clipboardWrites)).toEqual([unicode]);
  await section.getByRole('button', { name: '允许输入', exact: true }).click();
  await expect(panel.getByRole('button', { name: '发送文字', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: '获取远端文字', exact: true }).click();
  await expect(panel.getByLabel('远端剪贴板', { exact: true })).toHaveValue(unicode);
  await expect(panel.getByRole('status')).toHaveText('已收到远端文字。确认内容后可复制到本机。');
  await page.clock.install({ time: new Date('2026-10-19T12:00:00Z') });
  await page.clock.pauseAt(new Date('2026-10-19T12:00:01Z'));
  await page.evaluate(() => { (window as any).holdClipboardFetch = true; });
  const fetch = panel.getByRole('button', { name: '获取远端文字', exact: true });
  await fetch.click();
  const pending = panel.getByRole('button', { name: '正在获取…', exact: true });
  await expect(pending).toBeDisabled(); await expect(pending).toHaveAttribute('aria-busy', 'true');
  await page.clock.fastForward(9999); await expect(pending).toBeDisabled();
  await page.clock.fastForward(1);
  await expect(panel.getByRole('alert')).toContainText('10 秒内未收到远端文字');
  await expect(fetch).toBeEnabled();
  await expect(panel.getByLabel('远端剪贴板', { exact: true })).toHaveValue(unicode);
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('已连接');
  // A late response with identical text must clear the timeout too.
  await page.evaluate(() => (window as any).replyClipboard());
  await expect(panel.getByRole('alert')).toHaveCount(0); await expect(fetch).toBeEnabled();
  await fetch.click(); await expect(pending).toBeDisabled();
  await page.evaluate(() => (window as any).replyClipboard());
  await expect(fetch).toBeEnabled();
  await page.clock.fastForward(10_001); await expect(panel.getByRole('alert')).toHaveCount(0);
  await fetch.click(); await expect(pending).toBeDisabled();
  await page.evaluate(() => (window as any).replyClipboard('clipboard,%%%%'));
  await expect(fetch).toBeEnabled(); await expect(panel.getByRole('alert')).toContainText('已忽略');
  await page.clock.fastForward(10_001); await expect(panel.getByRole('alert')).not.toContainText('10 秒');
  await fetch.click();
  await page.clock.fastForward(10_000); await expect(panel.getByRole('alert')).toContainText('10 秒内未收到远端文字');
  await page.evaluate(() => (window as any).replyClipboard('clipboard,'));
  await expect(fetch).toBeEnabled(); await expect(panel.getByLabel('远端剪贴板', { exact: true })).toHaveValue('');
  await expect(panel.getByRole('alert')).toHaveCount(0);
  await fetch.click(); await expect(pending).toBeDisabled();
  await section.getByRole('button', { name: '剪贴板', exact: true }).click();
  await page.clock.fastForward(10_001);
  await section.getByRole('button', { name: '剪贴板', exact: true }).click();
  await expect(fetch).toBeEnabled(); await expect(panel.getByRole('alert')).toHaveCount(0);
  await page.evaluate(() => (window as any).replyClipboard());
  await expect(panel.getByLabel('远端剪贴板', { exact: true })).toHaveValue(unicode);
  await page.evaluate(() => {
    navigator.clipboard.writeText = () => new Promise((_resolve, reject) => { (window as any).rejectClipboardCopy = reject; });
    document.execCommand = () => { (window as any).calls.push({ command: 'unexpectedClipboardFallback' }); return false; };
  });
  await panel.getByRole('button', { name: '复制到本机', exact: true }).click();
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await page.evaluate(() => (window as any).rejectClipboardCopy(Error('Copy cancelled after leaving')));
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_close').length)).toBe(1);
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'unexpectedClipboardFallback'))).toEqual([]);
  expect(errors).toEqual([]);
});

for (const source of ['node', 'decoder']) test('native desktop failures retain the ' + source + ' reason when closing the viewer', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(source => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const callbacks = new Map<number, Function>(); let next = 1, index = 0;
    let callback: Function;
    internals.transformCallback = (fn: Function) => { const id = next++; callbacks.set(id, fn); return id; };
    internals.unregisterCallback = (id: number) => callbacks.delete(id);
    (window as any).failDesktop = () => {
      if (source === 'node') callback({ index: index++, message: { type: 'error', message: 'Selkies provider read timed out' } });
      else {
        const frame = new Uint8Array(5); new DataView(frame.buffer).setUint32(1, 16 * 1024 * 1024 + 1);
        callback({ index: index++, message: { type: 'data', data: btoa(String.fromCharCode(...frame)), sequence: 2 } });
      }
    };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') return { node_id: 'own-device', routes: [], grants: [], desktop: { address: '127.0.0.1:8082', protocol: 'selkies' } };
      if (command === 'desktop_open') {
        callback = callbacks.get(payload.events.id)!;
        callback({ index: index++, message: { type: 'ready' } });
        callback({ index: index++, message: { type: 'data', data: btoa('CRANIBOX-SELKIES/1\n'), sequence: 1 } });
        return 'failed-desktop-attachment';
      }
      if (command === 'desktop_ack') return;
      if (command === 'desktop_close') { (window as any).calls.push({ command, payload }); return; }
      return previous(command, payload);
    };
  }, source);
  await page.goto('/'); await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  const section = page.getByRole('region', { name: '跨设备桌面', exact: true });
  await section.getByRole('button', { name: '连接桌面', exact: true }).click();
  await expect(section.locator('canvas.selkies-screen')).toHaveCount(1);
  await page.evaluate(() => (window as any).failDesktop());
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('已断开');
  await expect(section.locator('.vnc-viewer [role="alert"]')).toHaveText(source === 'node' ? 'Selkies provider read timed out' : 'Invalid Selkies frame size or type');
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_close').map((call: any) => call.payload))).toEqual([{ id: 'failed-desktop-attachment' }]);
  expect(errors).toEqual([]);
});

test('the native desktop channel completes RFB, acknowledges frames and releases the viewer', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const callbacks = new Map<number, Function>(); let next = 1, index = 0, sequence = 0, pending = false, phase = 0;
    let callback: Function, red = 2, green = 1, blue = 0, typed = false;
    const queue: Uint8Array[] = [];
    internals.transformCallback = (fn: Function) => { const id = next++; callbacks.set(id, fn); return id; };
    internals.unregisterCallback = (id: number) => callbacks.delete(id);
    function flush() {
      if (pending || !queue.length) return;
      pending = true; let binary = ''; for (const byte of queue.shift()!) binary += String.fromCharCode(byte);
      callback({ index: index++, message: { type: 'data', data: btoa(binary), sequence: ++sequence } });
    }
    function output(bytes: Uint8Array) { queue.push(bytes); flush(); }
    function frame() {
      const bytes = new Uint8Array(16 + 160 * 100 * 4), view = new DataView(bytes.buffer);
      view.setUint16(2, 1); view.setUint16(8, 160); view.setUint16(10, 100);
      for (let at = 16; at < bytes.length; at += 4) { bytes[at + red] = typed ? 212 : 64; bytes[at + green] = typed ? 100 : 139; bytes[at + blue] = typed ? 56 : 83; }
      output(bytes);
    }
    const catalog = { node_id: 'own-device', routes: [], grants: [], desktop: { address: '127.0.0.1:5900' } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: false, error: null, quit_requested: false };
      if (command === 'direct_peers') return catalog;
      if (command === 'desktop_open') {
        (window as any).calls.push({ command, payload: { options: payload.options } });
        callback = callbacks.get(payload.events.id)!; callback({ index: index++, message: { type: 'ready' } });
        output(new TextEncoder().encode('RFB 003.008\n')); return 'desktop-attachment';
      }
      if (command === 'desktop_ack') {
        (window as any).calls.push({ command, payload });
        if (!pending || payload.sequence !== sequence) throw Error('Out-of-order desktop acknowledgement');
        pending = false; flush(); return;
      }
      if (command === 'desktop_input') {
        (window as any).calls.push({ command, payload });
        const bytes = Uint8Array.from(atob(payload.data), char => char.charCodeAt(0)), view = new DataView(bytes.buffer);
        if (phase === 0) { if (new TextDecoder().decode(bytes) !== 'RFB 003.008\n') throw Error('Reused RFB input buffer'); phase++; output(new Uint8Array([1, 1])); }
        else if (phase === 1) { phase++; output(new Uint8Array(4)); }
        else if (phase === 2) {
          phase++; const init = new Uint8Array(24); const data = new DataView(init.buffer);
          data.setUint16(0, 160); data.setUint16(2, 100); init.set([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0], 4); output(init);
        } else {
          let offset = 0;
          while (offset < bytes.length) {
            const type = bytes[offset];
            const size = type === 0 ? 20 : type === 2 ? 4 + 4 * view.getUint16(offset + 2) : type === 3 ? 10 : type === 4 ? 8 : type === 5 ? 6 : bytes.length;
            if (type === 0) { red = bytes[offset + 14] / 8; green = bytes[offset + 15] / 8; blue = bytes[offset + 16] / 8; }
            if (type === 3 && !bytes[offset + 1]) frame();
            if (type === 4 && bytes[offset + 1]) { typed = true; frame(); }
            offset += size;
          }
        }
        return;
      }
      if (command === 'desktop_close') { (window as any).calls.push({ command, payload }); return; }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  const section = page.getByRole('region', { name: '跨设备桌面', exact: true });
  await section.getByRole('button', { name: '连接桌面', exact: true }).click();
  await expect(section.locator('.vnc-toolbar [role="status"]')).toHaveText('已连接');
  const pixel = () => section.locator('canvas').evaluate((canvas: HTMLCanvasElement) => Array.from(canvas.getContext('2d')!.getImageData(10, 10, 1, 1).data).slice(0,3));
  await expect.poll(pixel).toEqual([64,139,83]);
  await section.getByRole('button', { name: '键盘', exact: true }).click();
  await section.getByLabel('远程键盘输入', { exact: true }).fill('a');
  await expect.poll(pixel).toEqual([212,100,56]);
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_close').length)).toBe(1);
  expect(await page.evaluate(() => (window as any).calls.find((call: any) => call.command === 'desktop_open').payload.options)).toEqual({ node: 'own-device', local: true });
  expect(await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_ack').length)).toBeGreaterThan(3);
  expect(errors).toEqual([]);
});

for (const wayland of [false, true]) test('managed desktop service uses native IPC and stays enabled after navigating away' + (wayland ? ' on Wayland' : ' on X11'), async ({ page }) => {
  await page.addInitScript((wayland: boolean) => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const status = { platform: 'linux', provider: wayland ? 'wayvnc' : 'x11vnc', username: wayland ? 'cranibox' : null, output: '', supported: true, installed: true, can_install: false, wayland, display: wayland ? 'wayland-0' : ':1', port: 5900, managed: false, running: false, enabled: false, phase: 'stopped' };
    const catalog = { node_id: 'own-device', routes: [], grants: [], desktop: null as null | { address: string } };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') return structuredClone(catalog);
      if (command === 'desktop_service') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'start') { Object.assign(status, { managed: true, enabled: true, running: true, phase: 'running' }); catalog.desktop = { address: '127.0.0.1:5900' }; }
        if (payload.command.operation === 'stop') { Object.assign(status, { enabled: false, running: false, phase: 'stopped' }); catalog.desktop = null; }
        if (payload.command.operation === 'credentials') return { password: 'nativePW' };
        return structuredClone(status);
      }
      return previous(command, payload);
    };
  }, wayland);
  await page.goto('/');
  await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  const section = page.getByRole('region', { name: '本机 VNC 服务', exact: true });
  await section.getByRole('button', { name: '启动并共享桌面', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('后台运行中');
  await page.getByRole('button', { name: '设备网络', exact: true }).click();
  expect(await page.evaluate(() => (window as any).calls.some((call: any) => call.command === 'desktop_service' && call.payload.command.operation === 'stop'))).toBe(false);
  await page.getByRole('button', { name: '远程桌面', exact: true }).click();
  await expect(section.getByRole('status')).toContainText('后台运行中');
  await section.getByRole('button', { name: '查看 VNC 密码', exact: true }).click();
  await expect(section.getByLabel('VNC 密码', { exact: true })).toHaveValue('nativePW');
  if (wayland) await expect(section.getByLabel('VNC 用户名', { exact: true })).toHaveValue('cranibox');
  await section.getByRole('button', { name: '停止托管服务', exact: true }).click();
  await expect(section.getByRole('button', { name: '启动并共享桌面', exact: true })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).calls.filter((call: any) => call.command === 'desktop_service' && call.payload.command.operation !== 'status'));
  expect(calls.map((call: any) => call.payload.command)).toEqual([{ operation: 'start', display: wayland ? 'wayland-0' : ':1', port: 5900, ...(wayland ? { provider: 'wayvnc', output: '' } : {}) }, { operation: 'credentials' }, { operation: 'stop' }]);
});

test('native task cards review, recover and report a bound Agent run through Rust IPC', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const now = Math.floor(Date.now() / 1000), id = '11111111-1111-4111-8111-111111111111', dispatch = '22222222-2222-4222-8222-222222222222';
    const receipt = { task: id, dispatch, sender: 'remote-device', recipient: 'own-device', state: 'available', session: null as string | null, result: null as string | null, revision: 1, updated_at: now };
    const item = { task: { id, sender: receipt.sender, recipient: receipt.recipient, title: '检查任务执行', description: '检查文件，报告证据。', created_at: now, expires_at: now + 604800 }, current: receipt, history: [receipt], grant_active: true, run: null as any };
    const ledger = { owner: 'own-device', incoming: [item], outgoing: [], used: 1, capacity: 128, max_attempts: 8 };
    let review: any, detail: any;
    (window as any).finishTaskProcess = () => { detail.execution.state = 'succeeded'; detail.execution.exit_code = 0; };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: ledger.owner, online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push({ command, payload });
      const action = payload.command;
      if (action.operation === 'list') return { node_id: ledger.owner, routes: [], grants: [] };
      if (action.operation === 'agent_sessions') return { owner: ledger.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] };
      if (action.operation === 'executions') return { owner: ledger.owner, executions: [], used: 0, capacity: 128, parallel: 8, log_limit: 1048576 };
      if (action.operation !== 'tasks') return previous(command, payload);
      if (action.command.operation !== 'run') return structuredClone(ledger);
      const run = action.command.command;
      if (run.operation === 'prepare') { review = { owner: ledger.owner, task: id, dispatch, options: run.options, digest: 'b'.repeat(64), available: true, launch: { program: '/tools/codex', args: ['exec', '-'], cwd: run.options.cwd, stdin: item.task.description } }; return structuredClone(review); }
      if (run.operation === 'start') {
        item.run = { id: '99999999-9999-4999-8999-999999999999', session: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' };
        receipt.state = 'running'; receipt.session = item.run.session; receipt.revision++;
        detail = { owner: ledger.owner, task: id, dispatch, run: { ...item.run, options: review.options, digest: review.digest, proposal: { outcome: 'succeeded', result: '候选：已检查两份文件，请人工核对。', created_at: now } }, launch: review.launch, execution: { id: item.run.id, session_id: item.run.session, state: 'running', exit_code: null }, stdout: { text: '本机任务输出🙂', next_offset: 22 }, stderr: { text: '', next_offset: 0 } };
      }
      if (run.operation === 'finish') { receipt.state = run.outcome; receipt.result = run.result; receipt.revision++; return structuredClone(ledger); }
      return structuredClone({ ...detail, stdout: run.stdout_offset ? { text: '', next_offset: detail.stdout.next_offset } : detail.stdout });
    };
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.getByRole('region', { name: '协作任务', exact: true });
  await panel.getByRole('button', { name: /^发给本机/ }).click();
  await panel.getByRole('button', { name: '用 Agent 处理', exact: true }).click();
  await panel.getByRole('textbox', { name: '任务工作目录', exact: true }).fill('/home/user/project');
  await panel.getByRole('checkbox', { name: '启用任务沟通工具', exact: true }).check();
  await panel.getByRole('button', { name: '检查任务执行', exact: true }).click();
  await expect(panel.getByLabel('完整任务指令')).toContainText('检查文件，报告证据。');
  await panel.getByRole('button', { name: '确认领取并启动', exact: true }).click();
  await expect(panel.getByLabel('任务输出 stdout')).toContainText('本机任务输出🙂');
  await expect(panel.getByRole('button', { name: '提交已核对的结果', exact: true })).toBeDisabled();
  await nav.getByRole('button', { name: '连接设置', exact: true }).click();
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  await panel.getByRole('button', { name: /^发给本机/ }).click();
  await expect(panel.getByLabel('任务输出 stdout')).toContainText('本机任务输出🙂');
  await page.evaluate(() => (window as any).finishTaskProcess());
  await panel.getByRole('button', { name: '查找或刷新任务运行', exact: true }).click();
  await expect(panel.getByRole('region', { name: 'Agent 候选结果', exact: true })).toContainText('请人工核对');
  await panel.getByRole('button', { name: '填入报告供我核对', exact: true }).click();
  await expect(panel.getByRole('textbox', { name: '结果与验证证据', exact: true })).toHaveValue('候选：已检查两份文件，请人工核对。');
  await panel.getByRole('textbox', { name: '结果与验证证据', exact: true }).fill('检查了实际输出与文件，所需证据齐全。');
  await panel.getByRole('button', { name: '提交已核对的结果', exact: true }).click();
  await expect(panel.locator('.task-result')).toContainText('所需证据齐全');
  const calls = await page.evaluate(() => (window as any).calls);
  const launches = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'tasks' && call.payload.command.command.command?.operation === 'start');
  expect(launches).toHaveLength(1);
  expect(launches[0].payload.command.command.command.options.communication).toBe(true);
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});

test('collaboration tasks share IPC and retain explicit reports without relay settings', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const now = Math.floor(Date.now() / 1000), id = '11111111-1111-4111-8111-111111111111';
    const task = { id, sender: 'remote-device', recipient: 'own-device', title: '检查构建', description: '检查文件并报告证据。', created_at: now, expires_at: now + 604800 };
    const receipt = { task: id, dispatch: '22222222-2222-4222-8222-222222222222', sender: task.sender, recipient: task.recipient, state: 'running', session: '33333333-3333-4333-8333-333333333333', progress: '已检查两份文件。', result: null, revision: 3, updated_at: now };
    const ledger = { owner: 'own-device', capacity: 128, used: 1, max_attempts: 8, incoming: [{ task, current: receipt, history: [receipt], grant_active: true }], outgoing: [] as any[] };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: ledger.owner, routes: [{ node: 'remote-device', url: 'https://device.example.com', allow_private_http: false }], grants: [] };
        if (action.operation === 'agent_sessions') return { owner: action.node || ledger.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [{ id:'44444444-4444-4444-8444-444444444444', name:'本机审查 Agent', harness:'custom-cli', status:'ready', updated_at:now, expires_at:now+300, revision:1 }] };
        if (action.operation === 'tasks') {
          const update = action.command;
          if (update.operation === 'create') { const current = { ...receipt, task: update.id, dispatch: update.dispatch, sender: ledger.owner, recipient: update.node, state: 'queued', session: null, progress: null, revision: 0 }; ledger.outgoing.push({ task: { ...update, sender: ledger.owner, recipient: update.node, created_at: now, expires_at: now + 604800 }, current, history: [current] }); }
          return structuredClone(ledger);
        }
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.getByRole('region', { name: '协作任务', exact: true });
  await panel.getByRole('button', { name: /^发给本机/ }).click();
  await expect(panel.getByText('已检查两份文件。', { exact: true })).toBeVisible();
  await expect(panel.locator('.message-status')).toHaveText('Agent 已领取');
  const agents = panel.getByRole('region', { name: '所有设备的 Agent', exact: true });
  await expect(agents.getByRole('article', { name: '设备 own-device', exact: true })).toContainText('本机审查 Agent');
  await agents.getByRole('article', { name: '设备 remote-device', exact: true }).getByRole('button', { name: '委托给此 Agent', exact: true }).click();
  await expect(panel.getByRole('combobox', { name: '执行设备', exact: true })).toHaveValue('remote-device');
  await expect(panel.getByRole('combobox', { name: '执行 Agent', exact: true })).toHaveValue('44444444-4444-4444-8444-444444444444');
  await panel.getByLabel('任务标题', { exact: true }).fill('检查发布');
  await panel.getByLabel('任务说明与完成条件', { exact: true }).fill('检查更新日志并报告三项发现。');
  await panel.getByRole('button', { name: '保存并委托', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('本机排队');
  await nav.getByRole('button', { name: '连接设置', exact: true }).click();
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  await expect(panel.getByRole('heading', { name: '检查发布', exact: true })).toBeVisible();
  await page.setViewportSize({ width: 375, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await expect(panel.getByRole('region', { name: '所有设备的 Agent', exact: true }).getByRole('article', { name: '设备 own-device', exact: true }).locator('.agent-session-card')).toContainText('本机审查 Agent');
  const calls = await page.evaluate(() => (window as any).calls);
  expect(calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'create')).toHaveLength(1);
  expect(calls.find((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'create').payload.command.command.target_session).toBe('44444444-4444-4444-8444-444444444444');
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
  expect(errors).toEqual([]);
});

test('device messages use shared IPC without a relay and persist after leaving the view', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const now = Math.floor(Date.now() / 1000);
    const state = { owner: 'own-device', capacity: 256, used: 1, max_text_bytes: 8192, outgoing: [] as any[], incoming: [{ message: { id: '11111111-1111-4111-8111-111111111111', thread: '11111111-1111-4111-8111-111111111111', sender: 'remote-device', recipient: 'own-device', text: '另一台设备的 Agent 留言。', created_at: now, expires_at: now + 604800, in_reply_to: null, sender_session: '44444444-4444-4444-8444-444444444444' }, status: 'received' }] };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        if (payload.command.operation === 'list') return { node_id: state.owner, routes: [{ node: 'remote-device', url: 'https://device.example.com', allow_private_http: false }], grants: [] };
        if (payload.command.operation === 'agent_sessions') return { owner: payload.command.node, observed_at: now, lease_secs: 300, capacity: 64, sessions: [{ id: '44444444-4444-4444-8444-444444444444', name: '远程审查 Agent', harness: 'native-cli', status: 'ready', revision: 1, updated_at: now, expires_at: now + 300 }] };
        if (payload.command.operation === 'messages') {
          const action = payload.command.command;
          if (action.operation === 'acknowledge') state.incoming[0].status = 'acknowledged';
          if (action.operation === 'send') state.outgoing.push({ message: { ...action, sender: state.owner, recipient: action.node, created_at: now, expires_at: now + 604800 }, status: 'queued' });
          state.used = state.incoming.length + state.outgoing.length;
          return structuredClone(state);
        }
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '设备消息', exact: true }).click();
  const panel = page.getByRole('region', { name: '设备消息', exact: true });
  await expect(panel.locator('.message-text')).toHaveText('另一台设备的 Agent 留言。');
  await panel.getByRole('button', { name: '确认收件', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('已确认收件');
  await panel.getByRole('button', { name: '回复', exact: true }).click();
  await expect(panel.getByRole('combobox', { name: '收件 Agent', exact: true })).toHaveValue('44444444-4444-4444-8444-444444444444');
  await expect(panel.getByRole('combobox', { name: '收件 Agent', exact: true })).toBeDisabled();
  await panel.getByLabel('消息内容', { exact: true }).fill('已在本机接收，稍后处理。');
  await panel.getByRole('button', { name: '保存并发送', exact: true }).click();
  await expect(panel.locator('.message-status')).toHaveText('本机排队');
  await page.getByRole('button', { name: '连接设置', exact: true }).click();
  await page.getByRole('button', { name: '设备消息', exact: true }).click();
  await panel.getByRole('button', { name: /^发件箱/ }).click();
  await expect(panel.locator('.message-text')).toHaveText('已在本机接收，稍后处理。');
  await page.setViewportSize({ width: 375, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const calls = await page.evaluate(() => (window as any).calls);
  const sent = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.operation === 'messages' && call.payload.command.command.operation === 'send');
  expect(sent).toHaveLength(1);
  expect(sent[0].payload.command.command).toMatchObject({ node: 'remote-device', target_session: '44444444-4444-4444-8444-444444444444', thread: '11111111-1111-4111-8111-111111111111', in_reply_to: '11111111-1111-4111-8111-111111111111' });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('稍后处理');
  expect(errors).toEqual([]);
});

test('task questions use native IPC and retain queued replies after navigation', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke;
    const now = Math.floor(Date.now() / 1000), id = '11111111-1111-4111-8111-111111111111';
    const task = { id, sender: 'own-device', recipient: 'remote-device', title: '检查发布', description: '检查日志并报告证据。', created_at: now, expires_at: now + 604800 };
    const receipt = { task: id, dispatch: '22222222-2222-4222-8222-222222222222', sender: task.sender, recipient: task.recipient, state: 'running', session: '33333333-3333-4333-8333-333333333333', progress: null, result: null, revision: 3, updated_at: now, questions: [{ id: '77777777-7777-4777-8777-777777777777', text: '需要检查哪个版本？', answer: null as string | null }] };
    const item = { task, current: receipt, history: [receipt], pending_answer: null as any, synced_at: now };
    const ledger = { owner: task.sender, capacity: 128, used: 1, max_attempts: 8, incoming: [], outgoing: [item] };
    (window as any).confirmTaskReply = () => { receipt.questions[0].answer = item.pending_answer.text; item.pending_answer = null; receipt.revision++; };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: 'own-device', online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command === 'direct_peers') {
        (window as any).calls.push({ command, payload });
        const action = payload.command;
        if (action.operation === 'list') return { node_id: ledger.owner, routes: [{ node: 'remote-device', url: 'https://device.example.com', allow_private_http: false }], grants: [] };
        if (action.operation === 'agent_sessions') return { owner: ledger.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] };
        if (action.operation === 'tasks') {
          if (action.command.operation === 'answer') item.pending_answer = { question: action.command.question, text: action.command.text };
          return structuredClone(ledger);
        }
      }
      return previous(command, payload);
    };
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: '客户端页面' });
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.getByRole('region', { name: '协作任务', exact: true });
  await expect(panel.locator('.message-status')).toHaveText('等待回复');
  await panel.getByRole('textbox', { name: '回复执行方', exact: true }).fill('检查发布分支，完成后先报告测试证据。');
  await panel.getByRole('button', { name: '保存并回复', exact: true }).click();
  await expect(panel.locator('.task-answer-pending')).toContainText('回复已在本机排队');
  await nav.getByRole('button', { name: '连接设置', exact: true }).click();
  await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  await expect(panel.locator('.task-answer-pending')).toContainText('检查发布分支');
  await expect(panel.getByRole('textbox', { name: '回复执行方', exact: true })).toHaveCount(0);
  await page.evaluate(() => (window as any).confirmTaskReply());
  await panel.getByRole('button', { name: '刷新任务', exact: true }).click();
  await expect(panel.locator('.task-answer')).toContainText('目标已确认回复');
  await expect(panel.locator('.message-status')).toHaveText('Agent 已领取');
  await page.setViewportSize({ width: 375, height: 900 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const calls = await page.evaluate(() => (window as any).calls);
  const answers = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'answer');
  expect(answers).toHaveLength(1);
  expect(answers[0].payload.command.command).toEqual({ operation: 'answer', id: '11111111-1111-4111-8111-111111111111', dispatch: '22222222-2222-4222-8222-222222222222', question: '77777777-7777-4777-8777-777777777777', text: '检查发布分支，完成后先报告测试证据。' });
  expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
  expect(await page.evaluate(() => JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]))).not.toContain('检查发布分支');
  expect(errors).toEqual([]);
});

test('native collaboration runs retain their goal and use the same revision-fenced IPC commands', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke, now = Math.floor(Date.now() / 1000);
    const ledger = { owner: 'own-device', incoming: [], outgoing: [], capacity: 128, used: 0, max_attempts: 8, collaborations: [] as any[], collaboration_used: 0, collaboration_capacity: 32 };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: ledger.owner, online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push({ command, payload }); const action = payload.command;
      if (action.operation === 'list') return { node_id: ledger.owner, routes: [{ node: 'remote-device', url: 'https://device.example.com', allow_private_http: false }], grants: [] };
      if (action.operation === 'agent_sessions') return { owner: action.node || ledger.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] };
      if (action.operation === 'executions') return { owner: ledger.owner, executions: [], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 };
      if (action.operation === 'tasks') {
        if (action.command.operation === 'collaboration') {
          const update = action.command.command;
          if (update.operation === 'create') { ledger.collaborations.push({ scope: { id: update.id, owner: ledger.owner, name: update.name, goal: update.goal }, members: update.members, state: 'open', revision: 1, created_at: now, result: null, stats: { total: 0, active: 0, succeeded: 0, failed: 0, cancelled: 0, questions: 0, blocked: 0 } }); ledger.collaboration_used++; }
          if (update.operation === 'close') { ledger.collaborations[0].state = 'closed'; ledger.collaborations[0].result = update.result; ledger.collaborations[0].revision++; }
        }
        return structuredClone(ledger);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/'); const nav = page.getByRole('navigation', { name: '客户端页面' }); await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const panel = page.getByRole('region', { name: '多设备协作', exact: true }); await panel.getByRole('button', { name: '新建协作', exact: true }).click();
  await panel.getByLabel('协作名称', { exact: true }).fill('跨设备发布检查'); await panel.getByLabel('共同目标与完成条件', { exact: true }).fill('逐台检查并汇总证据。'); await panel.getByRole('checkbox', { name: 'remote-device', exact: true }).check(); await panel.getByRole('button', { name: '创建协作', exact: true }).click();
  await nav.getByRole('button', { name: '连接设置', exact: true }).click(); await nav.getByRole('button', { name: '协作任务', exact: true }).click();
  const card = panel.getByRole('article', { name: '协作 跨设备发布检查', exact: true }); await expect(card).toContainText('逐台检查并汇总证据。'); await card.getByRole('button', { name: '核对并结束协作', exact: true }).click(); await card.getByLabel('核对后的协作总结', { exact: true }).fill('没有派发任务，已核对并结束。'); await card.getByRole('button', { name: '确认结束协作', exact: true }).click(); await expect(card).toContainText('已结束');
  for (const width of [375, 1440]) { await page.setViewportSize({ width, height: 1000 }); expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true); }
  const calls = await page.evaluate(() => (window as any).calls); const updates = calls.filter((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'collaboration').map((call: any) => call.payload.command.command.command);
  expect(updates).toHaveLength(2); expect(updates[1]).toMatchObject({ operation: 'close', id: updates[0].id, revision: 1 }); expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});

test('native dependency delegations use shared controls and keep dependency IDs in Rust IPC', async ({ page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__, previous = internals.invoke, now = Math.floor(Date.now() / 1000);
    const runId = '11111111-1111-4111-8111-111111111111', parentId = '22222222-2222-4222-8222-222222222222';
    const scope = { id: runId, owner: 'own-device', name: '编译后测试', goal: '前置成功后再委托测试。' };
    const parent = { task: { id: parentId, sender: 'own-device', recipient: 'remote-device', collaboration: scope, title: '编译项目', description: '报告编译证据。', created_at: now, expires_at: now + 604800 }, current: { task: parentId, dispatch: '33333333-3333-4333-8333-333333333333', sender: 'own-device', recipient: 'remote-device', state: 'running', session: '44444444-4444-4444-8444-444444444444', progress: null, result: null, revision: 2, updated_at: now }, history: [] as any[] }; parent.history.push(parent.current);
    const ledger = { owner: 'own-device', incoming: [], outgoing: [parent] as any[], capacity: 128, used: 1, max_attempts: 8, collaborations: [{ scope, members: ['remote-device'], state: 'open', revision: 1, created_at: now, result: null, stats: { total: 1, active: 1, succeeded: 0, failed: 0, cancelled: 0, questions: 0, blocked: 0, waiting: 0 } }], collaboration_used: 1, collaboration_capacity: 32 };
    internals.invoke = async (command: string, payload: any) => {
      if (command === 'snapshot') return { settings: null, local: { id: ledger.owner, online: true }, status: null, autostart: true, error: null, quit_requested: false };
      if (command !== 'direct_peers') return previous(command, payload);
      (window as any).calls.push({ command, payload }); const action = payload.command;
      if (action.operation === 'list') return { node_id: ledger.owner, routes: [{ node: 'remote-device', url: 'https://device.example.com', allow_private_http: false }], grants: [] };
      if (action.operation === 'agent_sessions') return { owner: action.node || ledger.owner, observed_at: now, lease_secs: 300, capacity: 64, sessions: [] };
      if (action.operation === 'executions') return { owner: ledger.owner, executions: [], capacity: 128, used: 0, parallel: 8, log_limit: 1048576 };
      if (action.operation === 'tasks') {
        const update = action.command;
        if (update.operation === 'create') {
          const current = { ...parent.current, task: update.id, dispatch: update.dispatch, state: 'queued', session: null, revision: 0 };
          ledger.outgoing.push({ task: { ...parent.task, id: update.id, title: update.title, description: update.description }, current, history: [current], dependencies: { state: 'waiting', prerequisites: [{ id: parentId, title: parent.task.title, node: 'remote-device', current: parent.current }] } }); ledger.used++; ledger.collaborations[0].stats.waiting++;
        }
        return structuredClone(ledger);
      }
      return previous(command, payload);
    };
  });
  await page.goto('/'); await page.getByRole('navigation', { name: '客户端页面' }).getByRole('button', { name: '协作任务', exact: true }).click();
  await page.getByRole('article', { name: '协作 编译后测试', exact: true }).getByRole('button', { name: '添加委托', exact: true }).click();
  const form = page.getByRole('form', { name: '委托新任务', exact: true }); await form.getByText('前置任务（可选）', { exact: true }).click(); await form.getByRole('checkbox', { name: '等待 编译项目 (remote-device)', exact: true }).check();
  await form.getByRole('combobox', { name: '执行设备', exact: true }).selectOption('remote-device'); await form.getByRole('textbox', { name: '任务标题', exact: true }).fill('测试项目'); await form.getByRole('textbox', { name: '任务说明与完成条件', exact: true }).fill('核对编译完成后的版本并测试。'); await form.getByRole('button', { name: '保存并等待前置任务', exact: true }).click();
  const card = page.getByRole('article', { name: '任务 测试项目', exact: true }); await expect(card).toContainText('此任务尚未发出'); await expect(card.getByRole('button', { name: '取消等待任务', exact: true })).toBeVisible();
  const calls = await page.evaluate(() => (window as any).calls); const create = calls.find((call: any) => call.command === 'direct_peers' && call.payload.command.command?.operation === 'create').payload.command.command;
  expect(create).toMatchObject({ run_id: '11111111-1111-4111-8111-111111111111', depends_on: ['22222222-2222-4222-8222-222222222222'] }); expect(calls.some((call: any) => call.command === 'configure' || call.command === 'terminal_open')).toBe(false);
});
