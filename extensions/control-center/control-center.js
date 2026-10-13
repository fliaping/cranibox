// Shared UI and actions; the host supplies editor or standalone Web integrations.
module.exports = function createControlCenter(vscode) {
const fs = require('fs/promises');
const path = require('path');
const { execFile } = require('child_process');

const VIEW_ID = 'agentWorkspace.quickView';
const PANEL_TYPE = 'agentWorkspace.controlCenter';
const OUTPUT = vscode.window.createOutputChannel('Agent Workspace');
const ONBOARDING_MARKER = path.join(process.env.AGENT_WORKSPACE_CONFIG_ROOT || '/config', '.local/state/agent-workspace/onboarding-v1.complete');
const SNAPSHOT_CACHE_KEY = 'agentWorkspace.cachedSnapshot.v1';
const VALID_NAME = /^[A-Za-z0-9_@][A-Za-z0-9_.@-]*$/;
const AGENT_IDS = new Set(['codex', 'claude-code', 'hermes', 'deepseek-harness', 'openclaw']);
const MCP_AGENT_IDS = new Set(['codex', 'claude-code', 'hermes', 'deepseek-harness']);

function isSnapshotHealthy(snapshot) {
  return Boolean(snapshot && snapshot.summary && snapshot.summary.healthy);
}

function nonce() {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  let value = '';
  for (let index = 0; index < 32; index += 1) {
    value += alphabet.charAt(Math.floor(Math.random() * alphabet.length));
  }
  return value;
}

function run(command, args, options = {}) {
  return new Promise((resolve) => {
    execFile(command, args, {
      timeout: options.timeout ?? 20000,
      maxBuffer: options.maxBuffer ?? 8 * 1024 * 1024,
      env: { ...process.env, HOME: process.env.HOME || '/config' }
    }, (error, stdout, stderr) => {
      resolve({
        ok: !error,
        code: error && typeof error.code === 'number' ? error.code : 0,
        stdout: stdout.toString(),
        stderr: stderr.toString(),
        error
      });
    });
  });
}

function strings() {
  const zh = vscode.env.language.toLowerCase().startsWith('zh');
  if (!zh) {
    return {
      overview: 'Overview', agents: 'Agents', resources: 'MCP & Skills', services: 'Services', browser: 'Browser', desktop: 'Desktop', network: 'Network', diagnostics: 'Diagnostics',
      title: 'Agent Workspace', subtitle: 'One place to understand and operate your remote Agent environment.',
      ready: 'Workspace ready', attention: 'Needs attention', refreshing: 'Refreshing', refresh: 'Refresh',
      openDesktop: 'Open desktop', openTerminal: 'Open terminal', openWorkspace: 'Open workspace',
      quickStart: 'Quick start', environment: 'Environment', installedAgents: 'Ready Agents', runningServices: 'Running services',
      secureMode: 'Docker boundary', capabilities: 'Capabilities', recentServices: 'Workspace services', viewAll: 'View all',
      install: 'Install', launch: 'Launch', openWebUi: 'Open Web UI', installed: 'Installed', optional: 'Optional', missing: 'Missing', selected: 'Selected',
      versionUnknown: 'Version unavailable', noAgents: 'No Agent installed yet', agentHelp: 'Install one Agent first; it can help configure everything else.',
      updateOpenClaw: 'Update Gateway', updateOpenClawWarning: 'Update OpenClaw Gateway now? The service will restart briefly. Workspace will verify the new version and restore the previous service if verification fails.',
      service: 'Service', kind: 'Kind', status: 'Status', actions: 'Actions', searchServices: 'Filter services…',
      start: 'Start', stop: 'Stop', restart: 'Restart', logs: 'Logs', noServices: 'No services found',
      access: 'Access', networkHelp: 'Choose public custom-domain access, private Tailnet access, or an SSH tunnel.', listeningPorts: 'Listening ports', proxyRoutes: 'Custom-domain routes', installProxy: 'Configure custom domain',
      addRoute: 'Add route', checkProxy: 'Check routing', remove: 'Remove', open: 'Open', noRoutes: 'No named routes configured.',
      command: 'Process', address: 'Address', doctor: 'Environment checks', revision: 'Application revision',
      onboardingTitle: 'Welcome to Agent Workspace', onboardingSubtitle: 'Your environment is running. Take a minute to learn the important entry points.',
      stepWelcome: 'Welcome', stepAgent: 'Agent', stepWorkspace: 'Workspace', stepDesktop: 'Desktop', stepOptional: 'Finish',
      back: 'Back', next: 'Continue', finish: 'Finish setup', skip: 'Skip guide',
      welcomeBody: 'The foundation is installed automatically. Control Center will stay available from the Agent Workspace icon.',
      agentBody: 'Start with one working Agent. Sign in from a terminal, then let it help with the rest of the environment.',
      workspaceBody: 'Projects and Agent context live in the durable workspace directory.',
      accessBody: 'Selkies provides the complete graphical workspace and opens inside code-server or in its own browser tab.',
      optionalBody: 'Choose only the access and resource capabilities you need. You can return to the Control Center at any time.',
      foundationReady: 'Foundation ready', foundationWorking: 'Foundation is still being prepared',
      signInHint: 'Launch the Agent and follow its normal sign-in flow.',
      dockerSafe: 'Safe default', dockerElevated: 'Elevated access', dockerMedium: 'Isolated daemon',
      dockerSocketLabel: 'Docker socket connected', dockerSocketDetail: 'Agents can control containers reachable through the mounted socket.',
      dockerIsolatedLabel: 'Isolated Docker available', dockerIsolatedDetail: 'Agents can create containers in this workspace Docker daemon.',
      dockerDisabledLabel: 'Docker disabled', dockerDisabledDetail: 'No Docker daemon is available inside this workspace.',
      copyTunnel: 'Copy SSH tunnel', localAddresses: 'Docker-host addresses', detail: 'Details',
      healthy: 'Healthy', failed: 'Failed', unknown: 'Unknown', noPorts: 'No listening ports detected.',
      installCapability: 'Install capability', updateSource: 'Update application source', showOutput: 'Show operation log',
      globalResources: 'Global resources', resourceHelp: 'Install once, then see exactly which Agents can use it.',
      mcpServers: 'MCP servers', skills: 'Skills', addMcp: 'Add MCP', addSkill: 'Add Skill', updateAll: 'Update all',
      globalManager: 'Global manager', nativeConfig: 'Native config', shared: 'Shared', agentOnly: 'Agent-only',
      configuredAgents: 'Configured Agents', noMcp: 'No MCP servers configured.', noSkills: 'No global Skills found.',
      installMcpm: 'Install MCPM', mcpmReady: 'MCPM global profiles ready', mcpmOptional: 'Optional global MCP package manager',
      searchResources: 'Filter MCP servers and Skills…', update: 'Update',
      language: 'Language', switchLanguage: 'Switch language',
      switchToChinese: 'Switch the complete code-server interface to Simplified Chinese?',
      switchToEnglish: 'Switch the complete code-server interface to English?',
      switchAndRestart: 'Switch & restart',
      languageRestarting: 'Language saved. code-server is restarting; this page will reconnect automatically.',
      customDomain: 'Custom domain', configureDomain: 'Configure domain', domainNotConfigured: 'Not configured',
      domainHelp: 'Use your own DNS name through the Caddy router. TLS and gateway authentication stay deployment-owned.',
      domainValidation: 'Enter a bare domain without scheme, path, or port.',
      domainMigration: (current, next) => `Change the managed domain from ${current} to ${next}? Existing named routes will be migrated.`,
      tailscale: 'Tailscale network', tailscaleHelp: 'Private tailnet access without NET_ADMIN or a public port.',
      installTailscale: 'Install Tailscale', connectTailscale: 'Connect tailnet', exposeTailnet: 'Share workspace',
      tailnetConnected: 'Tailnet connected', tailnetLogin: 'Sign-in required', openTailnet: 'Open tailnet URL',
      selkiesCapability: 'Selkies Desktop', desktopReady: 'Desktop runtime and editor integration are ready.',
      desktopUnavailable: 'Desktop integration or the image runtime is unavailable.', installDesktop: 'Install desktop integration',
      desktopHelp: 'Operate the graphical workspace independently from networking and services.',
      desktopRuntime: 'Image runtime', desktopIntegration: 'Editor integration', desktopSession: 'Desktop session',
      desktopAccess: 'Desktop access', desktopPath: 'code-server path', copyDesktopTunnel: 'Copy desktop SSH tunnel',
      computerUse: 'Desktop Computer Use', computerUseHelp: 'Agents can see and operate the same native desktop shown by Selkies.',
      computerUseReady: 'Computer Use ready', computerUsePaused: 'Input is paused', computerUseStopped: 'Bridge is stopped', computerUseBackendMissing: 'Selkies control backend unavailable',
      setupComputerUse: 'Set up Computer Use', emergencyStop: 'Emergency stop', resumeControl: 'Resume control',
      controlEndpoint: 'Control endpoint', screenResolution: 'Screen resolution', activeSessions: 'Active sessions', agentConnections: 'Agent connections',
      noActiveSessions: 'No active Agent control session.', computerUseBoundary: 'Full desktop boundary',
      computerUseBoundaryDetail: 'Trusted container Agents can view the complete screen and send mouse or keyboard input. The bridge is loopback-only; never publish the internal PixelFlux port.',
      browserFirstPolicy: 'Web tasks use the managed browser through CDP. Desktop Computer Use is reserved for native applications.',
      browserControl: 'Managed Agent browser', browserControlReady: 'Agents can operate the same persistent Chromium window used on the desktop.',
      browserControlHelp: 'The default desktop browser uses a managed profile and a loopback-only control endpoint. Agent sessions connect without repeated approval prompts.',
      browserRunning: 'Browser running', browserApproval: 'Control endpoint', browserAgents: 'Configured Agents',
      browserSecurity: 'Container Agents can read and operate every open tab and logged-in session in this managed browser profile.',
      enableBrowserControl: 'Set up managed browser', openManagedBrowser: 'Open managed browser',
      browserNeedsSetup: 'MCP setup required', browserNeedsApproval: 'Restart in managed mode', browserNotRunning: 'Managed browser is stopped',
      browserUnsupported: 'Chromium 136 or newer is required', browserApprovalHint: 'Managed browser control is local to the container and no longer asks for per-session approval.',
      browserPageHelp: 'Set up and inspect the browser shared by the desktop user and trusted Agents.',
      browserProfile: 'Managed profile', browserProfileName: 'Visible profile name', browserProfileNamePending: 'New name applies after restarting the managed browser', browserEndpoint: 'Local control endpoint', browserDefault: 'Default desktop browser',
      browserDefaultReady: 'Managed launcher active', browserDefaultMissing: 'Managed launcher not installed',
      browserMcpTitle: 'Agent connections', browserMcpHelp: 'Chrome DevTools MCP is configured globally for installed Agents.',
      browserConnected: 'Connected', browserNotConnected: 'Not configured', browserNoAgents: 'Install an Agent before configuring browser access.',
      browserMigration: 'Existing browser data', browserMigrationPending: 'Will be copied on first managed launch', browserMigrationComplete: 'Managed profile is ready', browserProfileOnFirstLaunch: 'A new managed profile will be created on first launch',
      browserBoundary: 'Access boundary', browserBoundaryDetail: 'Only processes inside this trusted single-user container can reach the loopback control endpoint.',
      browserSettings: 'Browser settings', applyBrowserSettings: 'Apply Agent configuration'
    };
  }
  return {
    overview: '总览', agents: 'Agents', resources: 'MCP 与 Skills', services: '服务', browser: '浏览器', desktop: '桌面', network: '网络', diagnostics: '诊断',
    title: 'Agent Workspace', subtitle: '在一个地方了解和管理你的远程 Agent 环境。',
    ready: '工作区已就绪', attention: '需要处理', refreshing: '正在刷新', refresh: '刷新',
    openDesktop: '打开桌面', openTerminal: '打开终端', openWorkspace: '打开工作区',
    quickStart: '快速开始', environment: '环境', installedAgents: '可用 Agent', runningServices: '运行中服务',
    secureMode: 'Docker 权限边界', capabilities: '功能组件', recentServices: '工作区服务', viewAll: '查看全部',
    install: '安装', launch: '启动', openWebUi: '打开 Web UI', installed: '已安装', optional: '可选', missing: '缺失', selected: '首选',
    versionUnknown: '未知版本', noAgents: '尚未安装 Agent', agentHelp: '先安装一个 Agent，它就可以继续协助配置其他能力。',
    updateOpenClaw: '更新 Gateway', updateOpenClawWarning: '现在更新 OpenClaw Gateway？服务会短暂重启；Workspace 会验证新版本，失败时恢复原服务。',
    service: '服务', kind: '类型', status: '状态', actions: '操作', searchServices: '筛选服务…',
    start: '启动', stop: '停止', restart: '重启', logs: '日志', noServices: '未发现服务',
    access: '访问方式', networkHelp: '按需选择公网自定义域名、私有 Tailnet 或 SSH 隧道。', listeningPorts: '监听端口', proxyRoutes: '自定义域名路由', installProxy: '配置自定义域名',
    addRoute: '添加路由', checkProxy: '检查路由', remove: '删除', open: '打开', noRoutes: '尚未配置命名路由。',
    command: '进程', address: '地址', doctor: '环境检查', revision: '应用版本',
    onboardingTitle: '欢迎使用 Agent Workspace', onboardingSubtitle: '环境已经运行，用一分钟了解最重要的入口。',
    stepWelcome: '欢迎', stepAgent: 'Agent', stepWorkspace: '工作区', stepDesktop: '桌面', stepOptional: '完成',
    back: '上一步', next: '继续', finish: '完成向导', skip: '跳过向导',
    welcomeBody: '基础能力会自动安装。以后随时可以从左侧 Agent Workspace 图标回到控制中心。',
    agentBody: '先保证一个 Agent 可用。在终端完成登录后，就可以让它继续配置其他环境能力。',
    workspaceBody: '项目与 Agent 上下文都放在持久化工作区目录中，容器重建后仍会保留。',
    accessBody: 'Selkies 提供完整图形工作区，可以在 code-server 内打开，也可以使用独立浏览器窗口。',
    optionalBody: '只开启需要的访问与资源能力；以后随时可以回到控制中心继续配置。',
    foundationReady: '基础环境已就绪', foundationWorking: '基础环境仍在准备',
    signInHint: '启动 Agent，并按照它的正常流程完成登录。',
    dockerSafe: '安全默认值', dockerElevated: '高权限访问', dockerMedium: '隔离 Docker',
    dockerSocketLabel: '已连接 Docker Socket', dockerSocketDetail: 'Agent 可以控制通过该 Socket 可达的容器，请把它视为容器外的高权限。',
    dockerIsolatedLabel: '可用的隔离 Docker', dockerIsolatedDetail: 'Agent 可以在工作区自己的 Docker 守护进程中创建容器。',
    dockerDisabledLabel: 'Docker 未启用', dockerDisabledDetail: '当前工作区内没有可用的 Docker 守护进程。',
    copyTunnel: '复制 SSH 隧道', localAddresses: 'Docker 主机地址', detail: '详情',
    healthy: '正常', failed: '失败', unknown: '未知', noPorts: '未检测到监听端口。',
    installCapability: '安装能力', updateSource: '更新应用源码', showOutput: '查看操作日志',
    globalResources: '全局资源', resourceHelp: '安装一次，并清楚看到每个 Agent 是否可用。',
    mcpServers: 'MCP 服务', skills: 'Skills', addMcp: '添加 MCP', addSkill: '添加 Skill', updateAll: '全部更新',
    globalManager: '全局管理器', nativeConfig: '原生配置', shared: '全局共享', agentOnly: 'Agent 专属',
    configuredAgents: '已配置 Agent', noMcp: '尚未配置 MCP 服务。', noSkills: '尚未发现全局 Skill。',
    installMcpm: '安装 MCPM', mcpmReady: 'MCPM 全局配置已就绪', mcpmOptional: '可选的 MCP 全局包管理器',
    searchResources: '筛选 MCP 与 Skill…', update: '更新',
    language: '语言', switchLanguage: '切换语言',
    switchToChinese: '将控制中心与整个 code-server 界面切换为简体中文吗？',
    switchToEnglish: '将控制中心与整个 code-server 界面切换为英语吗？',
    switchAndRestart: '切换并重启',
    languageRestarting: '语言设置已保存，code-server 正在重启，页面会自动重新连接。',
    customDomain: '自定义域名', configureDomain: '配置域名', domainNotConfigured: '尚未配置',
    domainHelp: '通过 Caddy 路由使用自己的 DNS 域名；TLS 与网关认证仍由部署侧负责。',
    domainValidation: '请输入不含协议、路径和端口的裸域名。',
    domainMigration: (current, next) => `要将托管域名从 ${current} 改为 ${next} 吗？现有命名路由将一并迁移。`,
    tailscale: 'Tailscale 自组网', tailscaleHelp: '无需 NET_ADMIN 或公网端口，通过私有 Tailnet 访问。',
    installTailscale: '安装 Tailscale', connectTailscale: '连接 Tailnet', exposeTailnet: '共享工作区',
    tailnetConnected: 'Tailnet 已连接', tailnetLogin: '需要登录', openTailnet: '打开 Tailnet 地址',
    selkiesCapability: 'Selkies Desktop', desktopReady: '桌面运行时与编辑器集成均已就绪。',
    desktopUnavailable: '桌面集成或镜像运行时不可用。', installDesktop: '安装桌面集成',
    desktopHelp: '独立管理图形工作区，不与网络和服务功能混在一起。',
    desktopRuntime: '镜像运行时', desktopIntegration: '编辑器集成', desktopSession: '桌面会话',
    desktopAccess: '桌面访问', desktopPath: 'code-server 路径', copyDesktopTunnel: '复制桌面 SSH 隧道',
    computerUse: '桌面 Computer Use', computerUseHelp: 'Agent 可以看到并操作 Selkies 正在展示的同一个原生桌面。',
    computerUseReady: 'Computer Use 已就绪', computerUsePaused: '输入已暂停', computerUseStopped: 'Bridge 已停止', computerUseBackendMissing: 'Selkies 控制后端不可用',
    setupComputerUse: '配置 Computer Use', emergencyStop: '紧急停止', resumeControl: '恢复控制',
    controlEndpoint: '控制端点', screenResolution: '屏幕分辨率', activeSessions: '活动控制会话', agentConnections: 'Agent 连接',
    noActiveSessions: '当前没有 Agent 控制会话。', computerUseBoundary: '完整桌面权限边界',
    computerUseBoundaryDetail: '可信容器内的 Agent 可以查看完整屏幕，并发送鼠标和键盘输入；bridge 仅监听回环地址，绝不能发布内部 PixelFlux 端口。',
    browserFirstPolicy: '网页任务默认使用托管浏览器 CDP；只有原生桌面应用使用 Computer Use。',
    browserControl: 'Agent 托管浏览器', browserControlReady: 'Agent 与桌面用户共同操作同一个持久化 Chromium 窗口。',
    browserControlHelp: '桌面默认浏览器使用托管用户目录和仅回环开放的控制端点，Agent 会话无需反复授权。',
    browserRunning: '浏览器运行状态', browserApproval: '控制端点', browserAgents: '已配置 Agent',
    browserSecurity: '容器内 Agent 可以读取并操作托管浏览器用户目录中的全部标签页和登录会话。',
    enableBrowserControl: '配置托管浏览器', openManagedBrowser: '打开托管浏览器',
    browserNeedsSetup: '需要配置 MCP', browserNeedsApproval: '需要以托管模式重启', browserNotRunning: '托管浏览器未运行',
    browserUnsupported: '需要 Chromium 136 或更高版本', browserApprovalHint: '托管浏览器控制仅在容器内部开放，不再要求每个 Agent 会话重复授权。',
    browserPageHelp: '配置并查看桌面用户与可信 Agent 共用的浏览器能力。',
    browserProfile: '托管用户目录', browserProfileName: '可见 Profile 名称', browserProfileNamePending: '重启托管浏览器后应用新名称', browserEndpoint: '本地控制端点', browserDefault: '桌面默认浏览器',
    browserDefaultReady: '托管启动器已生效', browserDefaultMissing: '托管启动器未安装',
    browserMcpTitle: 'Agent 连接', browserMcpHelp: '通过全局 Chrome DevTools MCP 为已安装 Agent 提供浏览器能力。',
    browserConnected: '已接入', browserNotConnected: '未配置', browserNoAgents: '请先安装一个 Agent，再配置浏览器能力。',
    browserMigration: '现有浏览器数据', browserMigrationPending: '首次托管启动时将自动复制', browserMigrationComplete: '托管用户目录已就绪', browserProfileOnFirstLaunch: '首次启动时将创建新的托管用户目录',
    browserBoundary: '访问边界', browserBoundaryDetail: '只有这个可信单用户容器内的进程可以访问回环控制端点。',
    browserSettings: '浏览器设置', applyBrowserSettings: '应用 Agent 配置'
  };
}

// code-server rewrites localhost URLs passed to openExternal into its own
// /proxy/<port>/ route, which speaks plain HTTP to the port. The default
// desktop URL (https://localhost:3001) is nginx's HTTPS listener, so that route
// returns "400 plain HTTP request was sent to HTTPS port". For loopback URLs,
// open Selkies' HTTP listener on 3000 through the same authenticated proxy.
async function desktopExternalUri(configured) {
  const uri = vscode.Uri.parse(configured || 'https://localhost:3001');
  const host = uri.authority.replace(/^.*@/, '').replace(/:\d+$/, '').replace(/^\[|\]$/g, '');
  if (['localhost', '127.0.0.1', '::1'].includes(host.toLowerCase())) {
    return vscode.env.asExternalUri(vscode.Uri.parse('http://localhost:3000/'));
  }
  return uri;
}

class ControlCenter {
  constructor(context) {
    this.context = context;
    this.panel = undefined;
    this.quickView = undefined;
    const cachedSnapshot = context.globalState.get(SNAPSHOT_CACHE_KEY);
    this.snapshot = cachedSnapshot && cachedSnapshot.schema_version === 1
      ? cachedSnapshot
      : undefined;
    this.refreshPromise = undefined;
    this.interval = undefined;
    this.forceWelcome = false;
    this.statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 45);
    this.statusBar.command = 'agentWorkspace.openControlCenter';
    this.statusBar.name = 'Agent Workspace';
    this.statusBar.text = '$(loading~spin) Agent Workspace';
    this.statusBar.tooltip = 'Open Agent Workspace Control Center';
    this.statusBar.show();
    context.subscriptions.push(this.statusBar, OUTPUT);
    if (this.snapshot) this.updateStatusBar();
  }

  config() {
    const cfg = vscode.workspace.getConfiguration('agentWorkspace');
    return {
      workspacectl: cfg.get('workspacectlPath', '/config/bin/workspacectl'),
      autoOpen: cfg.get('autoOpenOnFirstRun', true),
      refreshInterval: cfg.get('refreshInterval', 30),
      desktopUrl: cfg.get('desktopUrl', 'https://localhost:3001')
    };
  }

  async loadSnapshot() {
    const result = await run(this.config().workspacectl, ['status', '--json'], { timeout: 30000 });
    if (!result.ok) {
      throw new Error((result.stderr || result.stdout || 'workspacectl failed').trim());
    }
    return JSON.parse(result.stdout);
  }

  async refresh(showError = false) {
    if (this.refreshPromise) return this.refreshPromise;
    const startedAt = Date.now();
    this.broadcast({ type: 'refreshing' });
    OUTPUT.appendLine(`[refresh] started ${new Date(startedAt).toISOString()}`);
    this.refreshPromise = this.loadSnapshot()
      .then((snapshot) => {
        this.snapshot = snapshot;
        this.updateStatusBar();
        this.broadcast({ type: 'snapshot', data: snapshot, forceWelcome: this.forceWelcome });
        OUTPUT.appendLine(`[refresh] completed in ${Date.now() - startedAt} ms`);
        this.context.globalState.update(SNAPSHOT_CACHE_KEY, snapshot).then(
          undefined,
          (error) => OUTPUT.appendLine(`[refresh] cache write failed: ${error.message}`)
        );
        return snapshot;
      })
      .catch((error) => {
        OUTPUT.appendLine(`Refresh failed: ${error.message}`);
        this.statusBar.text = '$(warning) Agent Workspace';
        this.statusBar.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
        this.broadcast({ type: 'error', message: error.message });
        if (showError) vscode.window.showErrorMessage(`Agent Workspace: ${error.message}`);
        return undefined;
      })
      .finally(() => {
        this.refreshPromise = undefined;
      });
    return this.refreshPromise;
  }

  updateStatusBar() {
    const healthy = isSnapshotHealthy(this.snapshot);
    this.statusBar.text = healthy ? '$(check) Agent Workspace' : '$(warning) Agent Workspace';
    this.statusBar.backgroundColor = healthy
      ? undefined
      : new vscode.ThemeColor('statusBarItem.warningBackground');
    this.statusBar.tooltip = healthy
      ? 'Agent Workspace is ready — open Control Center'
      : 'Agent Workspace needs attention — open Control Center';
  }

  broadcast(message) {
    if (this.panel) this.panel.webview.postMessage(message);
    if (this.quickView) this.quickView.webview.postMessage(message);
  }

  attach(webview, kind) {
    webview.options = { enableScripts: true };
    webview.html = kind === 'quick' ? quickViewHtml(webview) : controlCenterHtml(webview, this.forceWelcome);
    webview.onDidReceiveMessage((message) => this.handleMessage(message, webview));
  }

  resolveWebviewView(view) {
    this.quickView = view;
    this.attach(view.webview, 'quick');
    if (typeof view.onDidDispose === 'function') {
      view.onDidDispose(() => {
        if (this.quickView === view) this.quickView = undefined;
      });
    }
  }

  open(forceWelcome = false) {
    this.forceWelcome = forceWelcome;
    if (this.panel) {
      this.panel.reveal(vscode.ViewColumn.Active, false);
      this.panel.webview.postMessage({ type: 'showWelcome', value: forceWelcome });
      this.refresh();
      return;
    }
    const panel = vscode.window.createWebviewPanel(
      PANEL_TYPE,
      'Agent Workspace',
      vscode.ViewColumn.Active,
      { enableScripts: true, retainContextWhenHidden: true }
    );
    this.panel = panel;
    this.attach(panel.webview, 'panel');
    panel.onDidDispose(() => {
      if (this.panel === panel) this.panel = undefined;
      this.stopInterval();
    });
    panel.onDidChangeViewState(() => {
      if (panel.visible) {
        this.startInterval();
        this.refresh();
      } else {
        this.stopInterval();
      }
    });
    this.startInterval();
    this.refresh();
  }

  startInterval() {
    this.stopInterval();
    const seconds = Number(this.config().refreshInterval);
    if (seconds > 0) this.interval = setInterval(() => this.refresh(), seconds * 1000);
  }

  stopInterval() {
    if (this.interval) clearInterval(this.interval);
    this.interval = undefined;
  }

  terminal(command = '') {
    const terminal = vscode.window.createTerminal({
      name: 'Agent Workspace',
      cwd: '/config/Workspace',
      env: { HOME: '/config' }
    });
    terminal.show();
    if (command) terminal.sendText(command, true);
  }

  async operation(title, args, timeout = 15 * 60 * 1000) {
    OUTPUT.show(true);
    OUTPUT.appendLine(`\n==> ${title}`);
    OUTPUT.appendLine(`$ workspacectl ${args.join(' ')}`);
    const result = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title, cancellable: false },
      () => run(this.config().workspacectl, args, { timeout, maxBuffer: 16 * 1024 * 1024 })
    );
    if (result.stdout.trim()) OUTPUT.appendLine(result.stdout.trim());
    if (result.stderr.trim()) OUTPUT.appendLine(result.stderr.trim());
    if (!result.ok) {
      vscode.window.showErrorMessage(`${title} failed. See Agent Workspace output.`);
      return false;
    }
    vscode.window.showInformationMessage(`${title} complete.`);
    await this.refresh();
    return true;
  }

  async completeOnboarding() {
    await fs.mkdir(path.dirname(ONBOARDING_MARKER), { recursive: true });
    await fs.writeFile(ONBOARDING_MARKER, `${new Date().toISOString()}\n`, { mode: 0o600 });
    this.forceWelcome = false;
    await this.refresh();
  }

  async openDesktop() {
    const commands = await vscode.commands.getCommands(true);
    if (commands.includes('selkiesDesktop.open')) {
      await vscode.commands.executeCommand('selkiesDesktop.open');
      return;
    }
    await vscode.env.openExternal(await desktopExternalUri(this.config().desktopUrl));
  }

  async setupBrowser() {
    const copy = strings();
    const ok = await this.operation(copy.enableBrowserControl, ['browser', 'setup'], 120000);
    if (!ok) return;
    vscode.window.showWarningMessage(copy.browserApprovalHint);
    await this.openDesktop();
  }

  async openManagedBrowser() {
    const copy = strings();
    const ok = await this.operation(copy.openManagedBrowser, ['browser', 'open'], 30000);
    if (!ok) return;
    vscode.window.showWarningMessage(copy.browserApprovalHint);
    await this.openDesktop();
  }

  async switchLanguage() {
    const copy = strings();
    const configured = this.snapshot && this.snapshot.locale && this.snapshot.locale.current;
    const current = configured || (vscode.env.language.toLowerCase().startsWith('zh') ? 'zh-cn' : 'en');
    const target = current === 'zh-cn' ? 'en' : 'zh-cn';
    const choice = await vscode.window.showWarningMessage(
      target === 'zh-cn' ? copy.switchToChinese : copy.switchToEnglish,
      { modal: true },
      copy.switchAndRestart
    );
    if (choice !== copy.switchAndRestart) return;

    OUTPUT.appendLine(`\n==> ${copy.switchLanguage}`);
    OUTPUT.appendLine(`$ workspacectl locale ${target} --restart`);
    const result = await run(this.config().workspacectl, ['locale', target, '--restart'], { timeout: 10000 });
    if (result.stdout.trim()) OUTPUT.appendLine(result.stdout.trim());
    if (result.stderr.trim()) OUTPUT.appendLine(result.stderr.trim());
    if (!result.ok) {
      OUTPUT.show(true);
      vscode.window.showErrorMessage(`${copy.switchLanguage} failed. See Agent Workspace output.`);
      return;
    }
    vscode.window.showInformationMessage(copy.languageRestarting);
  }

  async handleMessage(message, sender) {
    try {
      switch (message && message.type) {
        case 'ready':
          if (this.snapshot && sender) {
            await sender.postMessage({
              type: 'snapshot',
              data: this.snapshot,
              forceWelcome: this.forceWelcome
            });
          }
          this.refresh();
          break;
        case 'open':
          this.open(false);
          break;
        case 'refresh':
          await this.refresh(true);
          break;
        case 'terminal':
          this.terminal(typeof message.command === 'string' ? message.command : '');
          break;
        case 'openWorkspace':
          await vscode.commands.executeCommand('vscode.openFolder', vscode.Uri.file('/config/Workspace'));
          break;
        case 'openDesktop':
          await this.openDesktop();
          break;
        case 'switchLanguage':
          await this.switchLanguage();
          break;
        case 'installAgent':
          if (AGENT_IDS.has(message.id)) {
            await this.operation(`Install ${message.id}`, ['install', 'agent', message.id]);
          }
          break;
        case 'updateOpenClaw': {
          const copy = strings();
          const choice = await vscode.window.showWarningMessage(copy.updateOpenClawWarning, { modal: true }, copy.updateOpenClaw);
          if (choice === copy.updateOpenClaw) await this.operation(copy.updateOpenClaw, ['agent', 'update', 'openclaw'], 40 * 60 * 1000);
          break;
        }
        case 'installMcpm':
          await this.operation('Install MCPM', ['install', 'mcpm']);
          break;
        case 'installDesktop':
          await this.operation('Install Selkies Desktop integration', ['install', 'desktop']);
          break;
        case 'setupComputerUse':
          await this.operation(strings().setupComputerUse, ['desktop', 'setup'], 180000);
          break;
        case 'desktopControl':
          if (['start', 'stop', 'restart', 'resume'].includes(message.action)) {
            await this.operation(`${strings().computerUse}: ${message.action}`, ['desktop', message.action], 60000);
          }
          break;
        case 'desktopEmergencyStop': {
          const copy = strings();
          const choice = await vscode.window.showWarningMessage(
            `${copy.emergencyStop}: ${copy.computerUseBoundaryDetail}`,
            { modal: true },
            copy.emergencyStop
          );
          if (choice === copy.emergencyStop) {
            await this.operation(copy.emergencyStop, ['desktop', 'emergency-stop'], 30000);
          }
          break;
        }
        case 'setupBrowser':
          await this.setupBrowser();
          break;
        case 'openManagedBrowser':
          await this.openManagedBrowser();
          break;
        case 'installTailscale':
          await this.operation('Install Tailscale', ['install', 'tailscale']);
          break;
        case 'connectTailscale':
          this.terminal('workspacectl tailscale login');
          break;
        case 'serveTailscale':
          await this.operation('Share workspace inside the tailnet', ['tailscale', 'serve'], 60000);
          break;
        case 'configureDomain':
          await this.configureDomain();
          break;
        case 'addSkill':
          await this.addSkill();
          break;
        case 'updateSkills':
          await this.operation('Update global Skills', ['skill', 'update']);
          break;
        case 'updateSkill':
          if (VALID_NAME.test(message.id || '')) {
            await this.operation(`Update Skill ${message.id}`, ['skill', 'update', message.id]);
          }
          break;
        case 'removeSkill':
          await this.removeSkill(message.id);
          break;
        case 'addMcp':
          await this.addMcp();
          break;
        case 'removeMcp':
          await this.removeMcp(message.id, message.agents, message.managed === true);
          break;
        case 'serviceAction':
          if (VALID_NAME.test(message.id) && ['start', 'stop', 'restart'].includes(message.action)) {
            const scope = message.kind === 's6' ? 's6' : 'service';
            await this.operation(`${message.action} ${message.id}`, [scope, message.action, message.id], 60000);
          }
          break;
        case 'showLogs':
          if (VALID_NAME.test(message.id)) {
            const result = await run(this.config().workspacectl, ['logs', message.id, '200'], { timeout: 10000 });
            OUTPUT.appendLine(`\n== ${message.id} ==`);
            OUTPUT.appendLine((result.stdout || result.stderr || '(no log output)').trim());
            OUTPUT.show(true);
          }
          break;
        case 'installProxy':
          await this.configureDomain();
          break;
        case 'checkProxy':
          await this.operation(strings().checkProxy, ['proxy', 'check'], 60000);
          break;
        case 'addRoute':
          await this.addRoute();
          break;
        case 'removeRoute':
          await this.removeRoute(message.name, message.host);
          break;
        case 'openUrl':
          if (typeof message.url === 'string' && /^https?:\/\//.test(message.url)) {
            await vscode.env.openExternal(vscode.Uri.parse(message.url));
          }
          break;
        case 'copyTunnel':
          await vscode.env.clipboard.writeText('ssh -N -L 8443:127.0.0.1:8443 user@server');
          vscode.window.showInformationMessage('SSH tunnel command copied.');
          break;
        case 'copyDesktopTunnel':
          await vscode.env.clipboard.writeText('ssh -N -L 3001:127.0.0.1:3001 user@server');
          vscode.window.showInformationMessage('Desktop SSH tunnel command copied.');
          break;
        case 'completeOnboarding':
          await this.completeOnboarding();
          break;
        case 'showOutput':
          OUTPUT.show(true);
          break;
        case 'updateSource':
          await this.operation('Update Agent Workspace', ['update']);
          break;
        case 'settings':
          await vscode.commands.executeCommand('workbench.action.openSettings', '@ext:agent-workspace.control-center');
          break;
      }
    } catch (error) {
      OUTPUT.appendLine(`Action failed: ${error.stack || error.message}`);
      vscode.window.showErrorMessage(`Agent Workspace: ${error.message}`);
    }
  }

  async addSkill() {
    const source = await vscode.window.showInputBox({
      title: 'Agent Workspace: Install global Skill',
      prompt: 'Enter owner/repository, a Git URL, or a local Skill package path.',
      placeHolder: 'owner/agent-skills',
      validateInput: (value) => value.trim() && !value.trim().startsWith('-') ? undefined : 'Enter a valid Skill source.'
    });
    if (!source) return;
    await this.operation('Install global Skill', ['skill', 'add', source.trim()]);
  }

  async removeSkill(name) {
    if (!VALID_NAME.test(name || '')) return;
    const choice = await vscode.window.showWarningMessage(
      `Remove global Skill ${name} from supported Agents?`,
      { modal: true },
      'Remove'
    );
    if (choice === 'Remove') {
      await this.operation(`Remove Skill ${name}`, ['skill', 'remove', name]);
    }
  }

  async addMcp() {
    const name = await vscode.window.showInputBox({
      title: 'Agent Workspace: MCP server name',
      prompt: 'Use one stable name across all selected Agents.',
      validateInput: (value) => VALID_NAME.test(value) ? undefined : 'Use letters, numbers, dot, underscore, @ or dash.'
    });
    if (!name) return;
    const transport = await vscode.window.showQuickPick(
      [
        { label: 'Streamable HTTP', id: 'http', description: 'A remote http:// or https:// MCP endpoint.' },
        { label: 'STDIO command', id: 'stdio', description: 'A local command such as npx -y package-name.' }
      ],
      { title: 'Agent Workspace: MCP transport' }
    );
    if (!transport) return;
    const value = await vscode.window.showInputBox({
      title: transport.id === 'http' ? 'Agent Workspace: MCP URL' : 'Agent Workspace: MCP command',
      prompt: transport.id === 'http'
        ? 'Secrets stay in the Agent OAuth flow or environment; do not place tokens in the URL.'
        : 'Enter the executable and arguments. Environment secrets are inherited by the Agent.',
      placeHolder: transport.id === 'http' ? 'https://example.com/mcp' : 'npx -y @scope/mcp-server',
      validateInput: (input) => {
        if (transport.id === 'http') return /^https?:\/\/[^\s]+$/.test(input.trim()) ? undefined : 'Enter an http:// or https:// URL.';
        return input.trim() && !input.trim().startsWith('-') ? undefined : 'Enter a valid command line.';
      }
    });
    if (!value) return;
    const available = ((this.snapshot && this.snapshot.mcp && this.snapshot.mcp.agents) || [])
      .filter((agent) => agent.installed)
      .map((agent) => ({ label: agent.label, id: agent.id, picked: true, description: `${agent.configured} configured` }));
    if (!available.length) {
      vscode.window.showErrorMessage('Install at least one Agent before adding an MCP server.');
      return;
    }
    const selected = await vscode.window.showQuickPick(available, {
      title: 'Agent Workspace: Make MCP available to',
      canPickMany: true,
      placeHolder: 'Choose one or more installed Agents'
    });
    if (!selected || !selected.length) return;
    const operationArgs = ['mcp', 'add', name, '--transport', transport.id, '--agents', selected.map((item) => item.id).join(',')];
    operationArgs.push(transport.id === 'http' ? '--url' : '--command-line', value.trim());
    await this.operation(`Add MCP ${name}`, operationArgs);
  }

  async removeMcp(name, agentList, managed) {
    if (!VALID_NAME.test(name || '')) return;
    const agents = String(agentList || '').split(',').filter((id) => MCP_AGENT_IDS.has(id));
    if (!agents.length) return;
    const choice = await vscode.window.showWarningMessage(
      `Remove MCP ${name} from ${agents.length} Agent${agents.length === 1 ? '' : 's'}?`,
      { modal: true },
      'Remove'
    );
    if (choice !== 'Remove') return;
    const operationArgs = ['mcp', 'remove', name, '--agents', agents.join(',')];
    if (managed) operationArgs.push('--purge-global');
    await this.operation(`Remove MCP ${name}`, operationArgs);
  }

  async addRoute() {
    const name = await vscode.window.showInputBox({
      title: 'Agent Workspace: Route name',
      prompt: 'Short route label, for example api or dashboard.',
      validateInput: (value) => VALID_NAME.test(value) ? undefined : 'Use letters, numbers, dot, underscore, @ or dash.'
    });
    if (!name) return;
    const target = await vscode.window.showInputBox({
      title: 'Agent Workspace: Route target',
      prompt: 'Upstream host:port reachable from this container.',
      placeHolder: '127.0.0.1:3000',
      validateInput: (value) => /^[^:\s]+:\d{1,5}$/.test(value) ? undefined : 'Use host:port.'
    });
    if (!target) return;
    await this.operation(`Add route ${name}`, ['proxy', 'add', name.trim(), target.trim()], 60000);
  }

  async configureDomain() {
    const copy = strings();
    const current = this.snapshot && this.snapshot.network && this.snapshot.network.root_domain;
    const domain = await vscode.window.showInputBox({
      title: `Agent Workspace: ${copy.customDomain}`,
      prompt: copy.domainHelp,
      value: current || '',
      placeHolder: 'workspace.example.com',
      validateInput: (value) => {
        const normalized = value.trim().toLowerCase().replace(/\.$/, '');
        return /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$/.test(normalized)
          ? undefined
          : copy.domainValidation;
      }
    });
    if (!domain) return;
    const normalized = domain.trim().toLowerCase().replace(/\.$/, '');
    if (current && current !== normalized) {
      const choice = await vscode.window.showWarningMessage(
        copy.domainMigration(current, normalized),
        { modal: true },
        copy.configureDomain
      );
      if (choice !== copy.configureDomain) return;
    }
    await this.operation(`Configure domain ${normalized}`, ['network', 'domain', normalized]);
  }

  async removeRoute(name, host) {
    if (!VALID_NAME.test(name || '')) return;
    const choice = await vscode.window.showWarningMessage(
      `Remove route ${host || name}?`,
      { modal: true },
      'Remove'
    );
    if (choice === 'Remove') {
      await this.operation(`Remove route ${name}`, ['proxy', 'remove', name], 60000);
    }
  }
}

function quickViewHtml(webview) {
  const scriptNonce = nonce();
  const copy = strings();
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-${scriptNonce}';">
<style>
  *{box-sizing:border-box} body{margin:0;padding:14px;color:var(--vscode-foreground);font:13px var(--vscode-font-family);background:var(--vscode-sideBar-background)}
  .hero{padding:16px;border:1px solid var(--vscode-widget-border);border-radius:12px;background:linear-gradient(145deg,color-mix(in srgb,var(--vscode-button-background) 18%,transparent),transparent)}
  .mark{width:34px;height:34px;border-radius:10px;display:grid;place-items:center;background:var(--vscode-button-background);color:var(--vscode-button-foreground);font-weight:800;margin-bottom:12px}
  h2{font-size:16px;margin:0 0 5px}.muted{color:var(--vscode-descriptionForeground);line-height:1.5}.status{display:flex;align-items:center;gap:7px;margin:13px 0}.dot{width:8px;height:8px;border-radius:50%;background:var(--vscode-charts-green)}
  button{width:100%;border:0;border-radius:7px;padding:8px 10px;margin-top:8px;cursor:pointer;background:var(--vscode-button-background);color:var(--vscode-button-foreground);font:inherit;font-weight:600}
  button.secondary{background:var(--vscode-button-secondaryBackground);color:var(--vscode-button-secondaryForeground)}
  .facts{margin-top:14px;display:grid;gap:8px}.fact{display:flex;justify-content:space-between;padding-bottom:8px;border-bottom:1px solid var(--vscode-widget-border)}
</style></head><body>
<div class="hero"><div class="mark">AW</div><h2>${copy.title}</h2><div class="muted">${copy.subtitle}</div><div class="status"><span class="dot" id="dot"></span><span id="status">${copy.refreshing}</span></div><button data-action="open">${copy.overview}</button><button class="secondary" data-action="desktop">${copy.openDesktop}</button></div>
<div class="facts"><div class="fact"><span>${copy.installedAgents}</span><strong id="agents">—</strong></div><div class="fact"><span>${copy.runningServices}</span><strong id="services">—</strong></div><div class="fact"><span>${copy.secureMode}</span><strong id="docker">—</strong></div></div>
<script nonce="${scriptNonce}">const vscode=acquireVsCodeApi();document.addEventListener('click',e=>{const a=e.target.closest('[data-action]');if(!a)return;vscode.postMessage({type:a.dataset.action==='desktop'?'openDesktop':'open'});});window.addEventListener('message',e=>{if(e.data.type!=='snapshot')return;const d=e.data.data;document.getElementById('status').textContent=d.summary.healthy?'${copy.ready}':'${copy.attention}';document.getElementById('dot').style.background=d.summary.healthy?'var(--vscode-charts-green)':'var(--vscode-charts-orange)';document.getElementById('agents').textContent=d.summary.ready_agents;document.getElementById('services').textContent=d.summary.running_services+'/'+d.summary.service_count;document.getElementById('docker').textContent=d.workspace.docker.mode;});vscode.postMessage({type:'ready'});</script>
</body></html>`;
}

function controlCenterHtml(webview, forceWelcome) {
  const scriptNonce = nonce();
  const copy = strings();
  return `<!doctype html>
<html lang="${vscode.env.language}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline' ${webview.standalone ? "'self'" : ''}; script-src 'nonce-${scriptNonce}'; ${webview.standalone ? "connect-src 'self';" : ''}">
${webview.standalone ? '<title>Agent Workspace</title>' : ''}
<style>
  :root{--aw-accent:#8b7cf6;--aw-cyan:#45c9c2;--aw-good:#55c98f;--aw-warn:#e5ac58;--aw-bad:#ed6b75;--aw-radius:14px;--aw-border:color-mix(in srgb,var(--vscode-foreground) 13%,transparent);--aw-soft:color-mix(in srgb,var(--vscode-editor-background) 78%,var(--aw-accent) 22%)}
  *{box-sizing:border-box}html,body{margin:0;min-height:100%;background:var(--vscode-editor-background);color:var(--vscode-foreground);font:13px/1.5 var(--vscode-font-family)}button,input{font:inherit}.app{min-height:100vh;background:radial-gradient(circle at 82% -5%,color-mix(in srgb,var(--aw-accent) 16%,transparent),transparent 32%),radial-gradient(circle at 8% 100%,color-mix(in srgb,var(--aw-cyan) 10%,transparent),transparent 30%)}
  .topbar{height:70px;padding:0 26px;display:flex;align-items:center;gap:14px;border-bottom:1px solid var(--aw-border);background:color-mix(in srgb,var(--vscode-editor-background) 88%,transparent);position:sticky;top:0;z-index:5;backdrop-filter:blur(12px)}.brand{display:flex;align-items:center;gap:11px;min-width:0}.logo{width:38px;height:38px;border-radius:12px;display:grid;place-items:center;background:linear-gradient(135deg,var(--aw-accent),#5a9bf8);color:white;font-weight:800;box-shadow:0 8px 26px color-mix(in srgb,var(--aw-accent) 28%,transparent)}.brand h1{font-size:16px;margin:0}.brand p{margin:1px 0 0;color:var(--vscode-descriptionForeground);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.top-actions{margin-left:auto;display:flex;align-items:center;gap:9px}.pill{display:inline-flex;align-items:center;gap:7px;border:1px solid var(--aw-border);border-radius:999px;padding:5px 10px;background:var(--vscode-editorWidget-background)}.pill .dot{width:7px;height:7px;border-radius:50%;background:var(--aw-good)}.language-button{min-width:54px;padding-inline:10px;font-weight:700}
  .shell{display:grid;grid-template-columns:210px minmax(0,1fr);max-width:1440px;margin:0 auto;min-height:calc(100vh - 70px)}.nav{padding:24px 16px;border-right:1px solid var(--aw-border)}.nav-list{display:grid;gap:5px;position:sticky;top:94px}.nav button{border:0;background:transparent;color:var(--vscode-foreground);border-radius:9px;padding:9px 11px;text-align:left;cursor:pointer;display:flex;gap:10px;align-items:center}.nav button:hover{background:var(--vscode-list-hoverBackground)}.nav button.active{background:color-mix(in srgb,var(--aw-accent) 18%,transparent);color:color-mix(in srgb,var(--vscode-foreground) 86%,var(--aw-accent) 14%);font-weight:650}.nav .icon{width:19px;text-align:center}.nav-note{margin-top:22px;padding:12px;border:1px solid var(--aw-border);border-radius:11px;color:var(--vscode-descriptionForeground);font-size:12px;overflow-wrap:anywhere}
  main{padding:30px clamp(20px,4vw,52px) 60px;min-width:0}.page-head{display:flex;align-items:flex-end;justify-content:space-between;gap:16px;margin-bottom:22px}.page-head h2{font-size:25px;letter-spacing:-.02em;margin:0}.page-head p{margin:5px 0 0;color:var(--vscode-descriptionForeground);font-size:14px}.grid{display:grid;grid-template-columns:repeat(12,minmax(0,1fr));gap:15px}.card{border:1px solid var(--aw-border);border-radius:var(--aw-radius);background:color-mix(in srgb,var(--vscode-editorWidget-background) 86%,transparent);padding:18px;box-shadow:0 9px 30px color-mix(in srgb,#000 8%,transparent)}.span-12{grid-column:span 12}.span-8{grid-column:span 8}.span-6{grid-column:span 6}.span-4{grid-column:span 4}.span-3{grid-column:span 3}.card h3{font-size:14px;margin:0 0 4px}.muted{color:var(--vscode-descriptionForeground)}.metric{font-size:27px;letter-spacing:-.03em;font-weight:720;margin-top:11px}.metric small{font-size:12px;font-weight:500;color:var(--vscode-descriptionForeground)}
  .hero{padding:25px;background:linear-gradient(135deg,color-mix(in srgb,var(--aw-accent) 22%,var(--vscode-editorWidget-background)),color-mix(in srgb,var(--aw-cyan) 9%,var(--vscode-editorWidget-background)));overflow:hidden;position:relative}.hero:after{content:'';position:absolute;width:220px;height:220px;border-radius:50%;right:-80px;top:-120px;border:35px solid color-mix(in srgb,#fff 7%,transparent)}.hero h2{font-size:25px;margin:0 0 7px;letter-spacing:-.025em}.hero p{max-width:650px;margin:0;color:var(--vscode-descriptionForeground);font-size:14px}.hero-actions{display:flex;gap:9px;flex-wrap:wrap;margin-top:20px;position:relative;z-index:1}
  .button{border:1px solid transparent;border-radius:8px;padding:7px 12px;cursor:pointer;background:var(--vscode-button-background);color:var(--vscode-button-foreground);font-weight:620;display:inline-flex;align-items:center;justify-content:center;gap:7px;text-decoration:none}.button:hover{background:var(--vscode-button-hoverBackground)}.button.secondary{background:var(--vscode-button-secondaryBackground);color:var(--vscode-button-secondaryForeground)}.button.ghost{background:transparent;color:var(--vscode-foreground);border-color:var(--aw-border)}.button.danger{background:color-mix(in srgb,var(--aw-bad) 18%,transparent);color:var(--vscode-foreground);border-color:color-mix(in srgb,var(--aw-bad) 42%,transparent)}.button.small{padding:4px 8px;font-size:12px}.button:disabled{opacity:.48;cursor:not-allowed}.icon-button{width:32px;height:32px;padding:0;border-radius:9px}.section-title{display:flex;justify-content:space-between;align-items:center;margin-bottom:13px}.section-title h3{margin:0;font-size:15px}
  .agent-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:15px}.agent-card{position:relative;overflow:hidden}.agent-card:before{content:'';position:absolute;inset:0 auto 0 0;width:3px;background:var(--agent,var(--aw-accent))}.agent-top{display:flex;align-items:center;gap:11px}.agent-avatar{width:39px;height:39px;border-radius:12px;display:grid;place-items:center;background:color-mix(in srgb,var(--agent,var(--aw-accent)) 18%,transparent);color:var(--agent,var(--aw-accent));font-weight:800}.agent-name{font-size:15px;font-weight:680}.agent-actions{display:flex;gap:8px;margin-top:16px}.badge{display:inline-flex;border-radius:999px;padding:2px 8px;font-size:11px;font-weight:650;background:color-mix(in srgb,var(--vscode-foreground) 9%,transparent)}.badge.ready,.state-running{color:var(--aw-good)}.badge.optional{color:var(--vscode-descriptionForeground)}.badge.warning,.state-failed{color:var(--aw-warn)}
  .list{display:grid;gap:8px}.list-row{display:flex;align-items:center;gap:12px;padding:10px 0;border-bottom:1px solid var(--aw-border)}.list-row:last-child{border-bottom:0}.list-main{min-width:0;flex:1}.list-main strong{display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.list-main small{display:block;color:var(--vscode-descriptionForeground);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.status-dot{width:9px;height:9px;border-radius:50%;background:var(--vscode-descriptionForeground)}.status-dot.running,.status-dot.ready{background:var(--aw-good);box-shadow:0 0 0 4px color-mix(in srgb,var(--aw-good) 14%,transparent)}.status-dot.failed,.status-dot.missing{background:var(--aw-bad)}.status-dot.needs-attention{background:var(--aw-warn)}.status-dot.optional,.status-dot.inactive,.status-dot.stopped{background:var(--vscode-descriptionForeground)}
  .table-wrap{overflow:auto;border:1px solid var(--aw-border);border-radius:12px}table{width:100%;border-collapse:collapse;min-width:720px}th,td{text-align:left;padding:10px 12px;border-bottom:1px solid var(--aw-border)}th{font-size:11px;text-transform:uppercase;letter-spacing:.06em;color:var(--vscode-descriptionForeground);background:color-mix(in srgb,var(--vscode-editorWidget-background) 75%,transparent);position:sticky;top:0}tr:last-child td{border-bottom:0}.row-actions{display:flex;gap:5px;justify-content:flex-end}.filter{width:min(320px,100%);border:1px solid var(--aw-border);border-radius:8px;padding:7px 10px;background:var(--vscode-input-background);color:var(--vscode-input-foreground);outline:0}.filter:focus{border-color:var(--vscode-focusBorder)}
  .risk{display:flex;align-items:flex-start;gap:12px}.risk-icon{width:38px;height:38px;border-radius:12px;display:grid;place-items:center;background:color-mix(in srgb,var(--aw-good) 14%,transparent);color:var(--aw-good);font-weight:800}.risk.high .risk-icon{background:color-mix(in srgb,var(--aw-bad) 14%,transparent);color:var(--aw-bad)}.risk.medium .risk-icon{background:color-mix(in srgb,var(--aw-warn) 14%,transparent);color:var(--aw-warn)}.code{font:12px/1.55 var(--vscode-editor-font-family);background:var(--vscode-textCodeBlock-background);padding:9px 11px;border-radius:8px;overflow-wrap:anywhere;margin-top:9px}.empty{padding:28px;text-align:center;color:var(--vscode-descriptionForeground)}
  .wizard{max-width:980px;margin:0 auto}.wizard-shell{display:grid;grid-template-columns:190px minmax(0,1fr);padding:0;overflow:hidden;min-height:510px}.steps{padding:25px 18px;background:linear-gradient(180deg,color-mix(in srgb,var(--aw-accent) 18%,var(--vscode-editorWidget-background)),var(--vscode-editorWidget-background))}.steps h3{font-size:17px;margin:0 0 4px}.step-list{display:grid;gap:7px;margin-top:25px}.step{display:flex;align-items:center;gap:9px;padding:8px;border-radius:9px;color:var(--vscode-descriptionForeground)}.step.active{background:color-mix(in srgb,var(--aw-accent) 20%,transparent);color:var(--vscode-foreground);font-weight:650}.step.done .step-index{background:var(--aw-good);color:#10281e}.step-index{width:24px;height:24px;display:grid;place-items:center;border-radius:50%;border:1px solid var(--aw-border);font-size:11px}.wizard-content{padding:38px clamp(24px,5vw,58px);display:flex;flex-direction:column}.wizard-content h2{font-size:27px;margin:0 0 9px;letter-spacing:-.03em}.wizard-content>p{font-size:14px;color:var(--vscode-descriptionForeground);max-width:640px}.wizard-body{margin-top:24px;flex:1}.wizard-actions{display:flex;align-items:center;gap:9px;margin-top:26px}.wizard-actions .skip{margin-right:auto}
  .toast{position:fixed;right:20px;bottom:20px;padding:10px 14px;border-radius:9px;background:var(--vscode-notifications-background);border:1px solid var(--aw-border);box-shadow:0 8px 28px #0005;display:none;z-index:20}.loading{display:grid;place-items:center;min-height:55vh}.spinner{width:28px;height:28px;border:3px solid var(--aw-border);border-top-color:var(--aw-accent);border-radius:50%;animation:spin .8s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}
  @media(max-width:900px){.shell{grid-template-columns:1fr}.nav{border-right:0;border-bottom:1px solid var(--aw-border);padding:10px 16px}.nav-list{position:static;display:flex;overflow:auto}.nav button{white-space:nowrap}.nav-note{display:none}.span-8,.span-6,.span-4,.span-3{grid-column:span 12}.agent-grid{grid-template-columns:1fr}.wizard-shell{grid-template-columns:1fr}.steps{padding:18px}.step-list{grid-template-columns:repeat(5,1fr);margin-top:15px}.step{justify-content:center;padding:5px}.step span:last-child{display:none}.topbar{padding:0 16px}.brand p{display:none}}
  @media(min-width:901px){
    .app{height:100vh;height:100dvh;min-height:0;display:flex;flex-direction:column;overflow:hidden}
    .topbar{flex-shrink:0}
    .shell{flex:1;min-height:0;width:100%;overflow:hidden}
    .nav{display:flex;flex-direction:column;min-height:0;overflow:hidden}
    .nav-list{position:static;flex:1;min-height:0;overflow-y:auto;align-content:start}
    .nav-note{flex-shrink:0}
    main{min-height:0;overflow-y:auto;overscroll-behavior-y:contain}
  }
  @media(prefers-reduced-motion:reduce){*{animation:none!important;transition:none!important}}
</style>${webview.standalone ? '<link rel="stylesheet" href="/web.css">' : ''}</head><body><div class="app">
  <header class="topbar"><div class="brand"><div class="logo">AW</div><div><h1>${copy.title}</h1><p>${copy.subtitle}</p></div></div><div class="top-actions"><span class="pill"><span class="dot" id="health-dot"></span><span id="health-label">${copy.refreshing}</span></span><button class="button ghost language-button" id="language-toggle" data-action="switchLanguage" title="${copy.switchLanguage}">中 / EN</button><button class="button ghost icon-button" data-action="refresh" title="${copy.refresh}">↻</button></div></header>
  <div class="shell"><aside class="nav"><div class="nav-list" id="nav"></div><div class="nav-note"><strong>/config/Workspace</strong><br><span id="nav-revision"></span></div></aside><main id="content"><div class="loading"><div><div class="spinner"></div><p>${copy.refreshing}…</p></div></div></main></div>
</div><div class="toast" id="toast"></div>
${webview.standalone ? `<script nonce="${scriptNonce}">window.workspaceWeb=${JSON.stringify(webview.standalone).replace(/</g, '\u003c')};</script><script nonce="${scriptNonce}" src="/web.js"></script>` : ''}
<script nonce="${scriptNonce}">
const vscode=acquireVsCodeApi();const copy=${JSON.stringify(copy)};let data=null;let page='overview';let wizardStep=0;let forceWelcome=${forceWelcome ? 'true' : 'false'};let serviceFilter='';let resourceFilter='';
const esc=(value)=>String(value??'').replace(/[&<>"']/g,ch=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
const post=(type,extra={})=>vscode.postMessage({type,...extra});
const stateLabel=(value)=>({ready:copy.installed,missing:copy.missing,optional:copy.optional,'needs-attention':copy.attention,running:copy.ready,active:copy.ready,stopped:copy.optional,inactive:copy.optional,failed:copy.failed,unknown:copy.unknown}[value]||value);
const dot=(state)=>'<span class="status-dot '+esc(state)+'"></span>';
const dockerText=(docker)=>docker.mode==='socket'?{label:copy.dockerSocketLabel,detail:copy.dockerSocketDetail}:docker.mode==='isolated'?{label:copy.dockerIsolatedLabel,detail:copy.dockerIsolatedDetail}:{label:copy.dockerDisabledLabel,detail:copy.dockerDisabledDetail};
function toast(message){const el=document.getElementById('toast');el.textContent=message;el.style.display='block';setTimeout(()=>el.style.display='none',2600)}
function nav(){const items=[['overview','⌂',copy.overview],['agents','✦',copy.agents],['browser','◉',copy.browser],['resources','⬡',copy.resources],['services','◫',copy.services],['desktop','▰',copy.desktop],['network','⌁',copy.network],['diagnostics','◇',copy.diagnostics]];document.getElementById('nav').innerHTML=items.map(([id,icon,label])=>'<button class="'+(page===id?'active':'')+'" data-nav="'+id+'"><span class="icon">'+icon+'</span><span>'+label+'</span></button>').join('');}
function health(){if(!data)return;const ok=data.summary.healthy;document.getElementById('health-label').textContent=ok?copy.ready:copy.attention;document.getElementById('health-dot').style.background=ok?'var(--aw-good)':'var(--aw-warn)';document.getElementById('nav-revision').textContent=(data.revision?copy.revision+' '+data.revision:'');const language=document.getElementById('language-toggle');if(language&&data.locale){language.textContent=data.locale.current==='zh-cn'?'EN':'中文';language.title=copy.switchLanguage+' · '+data.locale.label;}}
function pageHead(title,body,actions=''){return '<div class="page-head"><div><h2>'+title+'</h2><p>'+body+'</p></div><div>'+actions+'</div></div>';}
function overview(){const agents=data.agents.filter(a=>a.installed);const services=data.services.filter(s=>s.status==='running');const docker=data.workspace.docker;const dockerCopy=dockerText(docker);const featured=data.services.filter(s=>s.kind==='systemd').slice(0,5);return pageHead(copy.overview,copy.subtitle)+
'<div class="grid"><section class="card hero span-12"><h2>'+esc(data.summary.healthy?copy.foundationReady:copy.attention)+'</h2><p>'+esc(data.workspace.root)+' · '+esc(data.access.hostname)+'</p><div class="hero-actions"><button class="button" data-action="terminal">⌁ '+copy.openTerminal+'</button><button class="button secondary" data-action="openWorkspace">▣ '+copy.openWorkspace+'</button><button class="button ghost" data-action="openDesktop">▰ '+copy.openDesktop+'</button></div></section>'+
'<section class="card span-4"><h3>'+copy.installedAgents+'</h3><div class="metric">'+agents.length+' <small>/ '+data.agents.length+'</small></div><div class="muted">'+(agents.map(a=>esc(a.label)).join(' · ')||copy.noAgents)+'</div></section>'+
'<section class="card span-4"><h3>'+copy.runningServices+'</h3><div class="metric">'+services.length+' <small>/ '+data.services.length+'</small></div><div class="muted">systemd user + s6</div></section>'+
'<section class="card span-4"><h3>'+copy.secureMode+'</h3><div class="metric" style="font-size:18px">'+esc(dockerCopy.label)+'</div><div class="muted">'+esc(dockerCopy.detail)+'</div></section>'+
'<section class="card span-6"><div class="section-title"><h3>'+copy.quickStart+'</h3></div><div class="list"><div class="list-row">'+dot(agents.length?'ready':'missing')+'<div class="list-main"><strong>'+copy.agents+'</strong><small>'+esc(agents.length?agents[0].version:copy.agentHelp)+'</small></div><button class="button ghost small" data-nav="agents">'+copy.viewAll+'</button></div><div class="list-row">'+dot(data.browser&&data.browser.state==='ready'?'ready':data.browser&&data.browser.state==='needs-restart'?'needs-attention':'optional')+'<div class="list-main"><strong>'+copy.browser+'</strong><small>'+esc(data.browser&&data.browser.state==='ready'?copy.browserControlReady:copy.browserNeedsSetup)+'</small></div><button class="button ghost small" data-nav="browser">'+copy.detail+'</button></div><div class="list-row">'+dot(data.skills.shared_count?'ready':'optional')+'<div class="list-main"><strong>'+copy.resources+'</strong><small>'+esc(data.mcp.count+' MCP · '+data.skills.shared_count+' Skills')+'</small></div><button class="button ghost small" data-nav="resources">'+copy.detail+'</button></div><div class="list-row">'+dot('ready')+'<div class="list-main"><strong>'+copy.openWorkspace+'</strong><small>'+esc(data.workspace.root)+'</small></div><button class="button ghost small" data-action="openWorkspace">'+copy.open+'</button></div><div class="list-row">'+dot(data.network.mode==='incomplete'?'needs-attention':data.network.installed?'ready':'optional')+'<div class="list-main"><strong>'+copy.proxyRoutes+'</strong><small>'+esc(data.network.installed?(data.network.root_domain||copy.installed):data.network.mode==='incomplete'?copy.attention:copy.optional)+'</small></div><button class="button ghost small" data-nav="network">'+copy.detail+'</button></div></div></section>'+
'<section class="card span-6"><div class="section-title"><h3>'+copy.recentServices+'</h3><button class="button ghost small" data-nav="services">'+copy.viewAll+'</button></div><div class="list">'+(featured.length?featured.map(serviceRow).join(''):'<div class="empty">'+copy.noServices+'</div>')+'</div></section></div>';}
function serviceRow(s){return '<div class="list-row">'+dot(s.status)+'<div class="list-main"><strong>'+esc(s.name)+'</strong><small>'+esc(s.kind+' · '+s.status+(s.pid?' · PID '+s.pid:''))+'</small></div></div>';}
function agentWebButton(a){if(!a.installed||(!a.web_url&&!a.web_path))return '';const label=esc(a.web_label||copy.openWebUi);return a.web_url?'<button class="button" data-action="openUrl" data-url="'+esc(a.web_url)+'">'+label+'</button>':'<button class="button" data-action="openPath" data-path="'+esc(a.web_path)+'">'+label+'</button>';}
function agentsPage(){return pageHead(copy.agents,copy.agentHelp)+ '<div class="agent-grid">'+data.agents.map(a=>{const color=a.accent==='amber'?'#dda65c':a.accent==='cyan'?'#45c9c2':a.accent==='green'?'#55c98f':a.accent==='rose'?'#e56b8a':'#8b7cf6';const primary=a.installed?(agentWebButton(a)||'<button class="button" data-action="launchAgent" data-command="'+esc(a.launch)+'">'+copy.launch+'</button>'):'<button class="button" data-action="installAgent" data-id="'+esc(a.id)+'">'+copy.install+'</button>';const endpoint=a.web_url||a.web_path||a.launch;return '<section class="card agent-card" style="--agent:'+color+'"><div class="agent-top"><div class="agent-avatar">'+esc(a.label.slice(0,2).toUpperCase())+'</div><div><div class="agent-name">'+esc(a.label)+'</div><span class="badge '+(a.installed?'ready':'optional')+'">'+(a.installed?copy.installed:copy.optional)+'</span> '+(a.selected?'<span class="badge">'+copy.selected+'</span>':'')+'</div></div><p class="muted">'+esc(a.version||copy.versionUnknown)+'</p><div class="code">'+esc(endpoint)+'</div><div class="agent-actions">'+primary+(a.id==='openclaw'&&a.installed?'<button class="button ghost" data-action="updateOpenClaw">'+copy.updateOpenClaw+'</button>':'')+'<button class="button ghost" data-action="terminal" data-command="'+esc(a.launch||'')+'">'+copy.openTerminal+'</button></div></section>';}).join('')+'</div>';}
function agentLabel(id){return ({codex:'Codex','claude-code':'Claude Code',hermes:'Hermes Agent','deepseek-harness':'DeepSeek Harness',openclaw:'OpenClaw'}[id]||id);}
function resourcesPage(){const needle=resourceFilter.toLowerCase();const servers=data.mcp.items.filter(item=>(item.name+' '+item.detail+' '+item.agents.map(a=>a.label).join(' ')).toLowerCase().includes(needle));const skills=data.skills.items.filter(item=>(item.name+' '+item.description+' '+item.agents.join(' ')).toLowerCase().includes(needle));const manager=data.mcp.manager;const managerAction=manager.installed?'<span class="badge ready">'+copy.mcpmReady+'</span>':'<button class="button ghost small" data-action="installMcpm">'+copy.installMcpm+'</button>';return pageHead(copy.resources,copy.resourceHelp,'<input id="resource-filter" class="filter" placeholder="'+copy.searchResources+'" value="'+esc(resourceFilter)+'">')+
'<div class="grid"><section class="card span-4"><h3>'+copy.mcpServers+'</h3><div class="metric">'+data.mcp.count+' <small>/ '+data.mcp.global_count+' '+copy.shared+'</small></div><div class="muted">Codex · Claude Code · Hermes · DeepSeek</div></section><section class="card span-4"><h3>'+copy.skills+'</h3><div class="metric">'+data.skills.shared_count+' <small>/ '+data.skills.count+'</small></div><div class="muted">'+esc(data.skills.root)+'</div></section><section class="card span-4"><div class="section-title"><h3>'+copy.globalManager+'</h3>'+managerAction+'</div><div class="metric" style="font-size:18px">'+(manager.installed?copy.mcpmReady:copy.mcpmOptional)+'</div><div class="muted">MCPM · npx skills</div></section>'+
'<section class="card span-12"><div class="section-title"><div><h3>'+copy.mcpServers+'</h3><span class="muted">'+copy.configuredAgents+'</span></div><button class="button" data-action="addMcp">＋ '+copy.addMcp+'</button></div><div class="list">'+(servers.length?servers.map(server=>'<div class="list-row">'+dot(server.enabled?'ready':'optional')+'<div class="list-main"><strong>'+esc(server.name)+' '+(server.managed?'<span class="badge ready">MCPM</span>':'<span class="badge optional">'+copy.nativeConfig+'</span>')+'</strong><small>'+esc(server.transport+' · '+server.detail)+'</small><div style="margin-top:5px">'+server.agents.map(agent=>'<span class="badge '+(agent.enabled?'ready':'optional')+'">'+esc(agent.label)+'</span>').join(' ')+'</div></div><button class="button danger small" data-action="removeMcp" data-id="'+esc(server.id)+'" data-agents="'+esc(server.agents.map(agent=>agent.id).join(','))+'" data-managed="'+(server.managed?'true':'false')+'">'+copy.remove+'</button></div>').join(''):'<div class="empty">'+copy.noMcp+'</div>')+'</div></section>'+
'<section class="card span-12"><div class="section-title"><div><h3>'+copy.skills+'</h3><span class="muted">npx skills · '+esc(data.skills.root)+'</span></div><div><button class="button ghost" data-action="updateSkills">'+copy.updateAll+'</button> <button class="button" data-action="addSkill">＋ '+copy.addSkill+'</button></div></div><div class="table-wrap"><table><thead><tr><th>Skill</th><th>'+copy.status+'</th><th>'+copy.configuredAgents+'</th><th style="text-align:right">'+copy.actions+'</th></tr></thead><tbody>'+(skills.length?skills.map(skill=>'<tr><td><strong>'+esc(skill.name)+'</strong><div class="muted">'+esc(skill.description||skill.path)+'</div></td><td><span class="badge '+(skill.shared?'ready':'optional')+'">'+(skill.shared?copy.shared:copy.agentOnly)+'</span></td><td>'+skill.agents.map(id=>'<span class="badge">'+esc(agentLabel(id))+'</span>').join(' ')+'</td><td><div class="row-actions">'+(skill.shared?'<button class="button ghost small" data-action="updateSkill" data-id="'+esc(skill.id)+'">'+copy.update+'</button>':'')+'<button class="button danger small" data-action="removeSkill" data-id="'+esc(skill.id)+'">'+copy.remove+'</button></div></td></tr>').join(''):'<tr><td colspan="4" class="empty">'+copy.noSkills+'</td></tr>')+'</tbody></table></div></section></div>';}
function servicesPage(){const needle=serviceFilter.toLowerCase();const rows=data.services.filter(s=>(s.name+' '+s.id+' '+s.kind).toLowerCase().includes(needle));return pageHead(copy.services,copy.recentServices,'<input id="service-filter" class="filter" placeholder="'+copy.searchServices+'" value="'+esc(serviceFilter)+'">')+'<div class="table-wrap"><table><thead><tr><th>'+copy.service+'</th><th>'+copy.kind+'</th><th>'+copy.status+'</th><th>PID</th><th style="text-align:right">'+copy.actions+'</th></tr></thead><tbody>'+(rows.length?rows.map(s=>'<tr><td>'+dot(s.status)+' <strong>'+esc(s.name)+'</strong><div class="muted">'+esc(s.id)+'</div></td><td>'+esc(s.kind+(s.scope?' · '+s.scope:''))+'</td><td class="state-'+esc(s.status)+'">'+esc(stateLabel(s.status))+'</td><td>'+esc(s.pid||'—')+'</td><td><div class="row-actions">'+(s.status==='running'?'<button class="button ghost small" data-action="service" data-op="stop" data-kind="'+esc(s.kind)+'" data-id="'+esc(s.id)+'">'+copy.stop+'</button>':'<button class="button ghost small" data-action="service" data-op="start" data-kind="'+esc(s.kind)+'" data-id="'+esc(s.id)+'">'+copy.start+'</button>')+'<button class="button ghost small" data-action="service" data-op="restart" data-kind="'+esc(s.kind)+'" data-id="'+esc(s.id)+'">'+copy.restart+'</button>'+(s.kind==='systemd'?'<button class="button ghost small" data-action="logs" data-id="'+esc(s.id)+'">'+copy.logs+'</button>':'')+'</div></td></tr>').join(''):'<tr><td colspan="5" class="empty">'+copy.noServices+'</td></tr>')+'</tbody></table></div>';}
function desktopPage(){
  const desktop=data.desktop||{installed:false,runtime_available:false,running:false,state:'missing',local_url:'https://localhost:3001',code_server_path:'/proxy/3000/',service:'svc-selkies'};
  const cu=desktop.computer_use||{installed:false,running:false,state:'missing',paused:false,backend_available:false,loopback_only:true,resolution:{width:0,height:0},active_sessions:[],mcp_agents:[],capabilities:[],bridge_url:'http://127.0.0.1:8765',native_url:'http://127.0.0.1:8764'};
  const ready=desktop.installed&&desktop.runtime_available;
  const actions=ready?'<button class="button" data-action="openDesktop">'+copy.openDesktop+'</button>':'<button class="button" data-action="installDesktop">'+copy.installDesktop+'</button>';
  const cuLabel=cu.paused?copy.computerUsePaused:cu.state==='ready'?copy.computerUseReady:cu.running?copy.computerUseBackendMissing:copy.computerUseStopped;
  const cuTone=cu.paused?'needs-attention':cu.state==='ready'?'ready':'optional';
  const resolution=cu.resolution&&cu.resolution.width?cu.resolution.width+' × '+cu.resolution.height:copy.unknown;
  const agentConnections=(cu.mcp_agents||[]).length?(cu.mcp_agents||[]).map(id=>'<span class="badge ready">'+esc(agentLabel(id))+'</span>').join(' '):'<span class="muted">'+copy.browserNoAgents+'</span>';
  const sessions=(cu.active_sessions||[]).length?(cu.active_sessions||[]).map(session=>'<div class="list-row">'+dot('ready')+'<div class="list-main"><strong>'+esc(session.id)+'</strong><small>'+esc(session.last_action+' · '+session.idle_seconds+'s')+'</small></div></div>').join(''):'<div class="empty">'+copy.noActiveSessions+'</div>';
  const controlActions=!cu.installed?'<button class="button" data-action="setupComputerUse">'+copy.setupComputerUse+'</button>':!cu.running?'<button class="button" data-action="desktopControl" data-op="start">'+copy.start+'</button>':cu.paused?'<button class="button" data-action="desktopControl" data-op="resume">'+copy.resumeControl+'</button>':'<button class="button danger" data-action="desktopEmergencyStop">'+copy.emergencyStop+'</button> <button class="button ghost" data-action="desktopControl" data-op="stop">'+copy.stop+'</button>';
  return pageHead(copy.desktop,copy.desktopHelp,actions)+'<div class="grid">'+
    '<section class="card hero span-12"><h2>'+copy.selkiesCapability+'</h2><p>'+(ready?copy.desktopReady:copy.desktopUnavailable)+'</p><div class="hero-actions">'+actions+(desktop.runtime_available?' <button class="button ghost" data-action="service" data-op="restart" data-kind="s6" data-id="'+esc(desktop.service)+'">'+copy.restart+'</button>':'')+'</div></section>'+
    '<section class="card span-4"><h3>'+copy.desktopRuntime+'</h3><div class="metric" style="font-size:18px">'+esc(desktop.runtime_available?copy.ready:copy.missing)+'</div><div class="muted">/run/service/'+esc(desktop.service)+'</div></section>'+
    '<section class="card span-4"><h3>'+copy.desktopIntegration+'</h3><div class="metric" style="font-size:18px">'+esc(desktop.installed?copy.installed:copy.missing)+'</div><div class="muted">agent-workspace.selkies-desktop</div></section>'+
    '<section class="card span-4"><h3>'+copy.desktopSession+'</h3><div class="metric" style="font-size:18px">'+esc(desktop.running?copy.ready:copy.optional)+'</div><div class="muted">'+esc(desktop.service)+'</div></section>'+
    '<section class="card hero span-12"><div class="section-title"><div><h2>'+copy.computerUse+'</h2><span class="muted">'+copy.computerUseHelp+'</span></div>'+dot(cuTone)+'</div><div class="metric" style="font-size:18px">'+esc(cuLabel)+'</div><p>'+copy.browserFirstPolicy+'</p><div class="hero-actions">'+controlActions+'</div></section>'+
    '<section class="card span-4"><h3>'+copy.controlEndpoint+'</h3><div class="metric" style="font-size:17px">'+esc(cu.loopback_only?copy.ready:copy.attention)+'</div><div class="muted">Bridge: '+esc(cu.bridge_url)+'</div><div class="muted">PixelFlux: '+esc(cu.native_bind||cu.native_url)+'</div></section>'+
    '<section class="card span-4"><h3>'+copy.screenResolution+'</h3><div class="metric" style="font-size:18px">'+esc(resolution)+'</div><div class="muted">Wayland · PixelFlux</div></section>'+
    '<section class="card span-4"><h3>'+copy.agentConnections+'</h3><div style="margin-top:16px">'+agentConnections+'</div><div class="muted" style="margin-top:9px">MCP · '+esc((cu.capabilities||[]).length)+' tools</div></section>'+
    '<section class="card span-8"><div class="section-title"><h3>'+copy.activeSessions+'</h3><span class="badge '+((cu.active_sessions||[]).length?'ready':'optional')+'">'+esc((cu.active_sessions||[]).length)+'</span></div><div class="list">'+sessions+'</div></section>'+
    '<section class="card span-4"><div class="risk high"><div class="risk-icon">!</div><div><h3>'+copy.computerUseBoundary+'</h3><p class="muted">'+copy.computerUseBoundaryDetail+'</p></div></div></section>'+
    '<section class="card span-12"><div class="section-title"><h3>'+copy.desktopAccess+'</h3></div><div class="list"><div class="list-row">'+dot(desktop.running?'ready':'optional')+'<div class="list-main"><strong>'+copy.desktopPath+'</strong><small>'+esc(desktop.code_server_path)+'</small></div><button class="button ghost small" data-action="openDesktop">'+copy.open+'</button></div><div class="list-row">'+dot(desktop.running?'ready':'optional')+'<div class="list-main"><strong>HTTPS</strong><small>'+esc(desktop.local_url)+'</small></div></div></div><div class="code">ssh -N -L 3001:127.0.0.1:3001 user@server</div><button class="button ghost small" style="margin-top:9px" data-action="copyDesktopTunnel">'+copy.copyDesktopTunnel+'</button></section></div>';
}
function browserPage(){
  const browser=data.browser||{installed:false,supported:false,state:'unsupported',running:false,remote_debugging:false,mcp_agents:[],mcp_configured:false,version:'',profile:'/config/.config/agent-browser',profile_name:'Agent Workspace (Managed)',desired_profile_name:'Agent Workspace (Managed)',profile_name_applied:false,legacy_profile:'/config/.config/chromium',debug_port:9222,managed_default:false,profile_exists:false,migration_pending:false};
  const installedAgents=data.agents.filter(agent=>agent.installed);
  const browserStatus=browser.state==='ready'?copy.ready:browser.state==='needs-setup'?copy.browserNeedsSetup:browser.state==='needs-restart'?copy.browserNeedsApproval:browser.state==='not-running'?copy.browserNotRunning:copy.browserUnsupported;
  const browserTone=browser.state==='ready'?'ready':browser.state==='needs-restart'?'needs-attention':'optional';
  const primary=browser.supported?(browser.mcp_configured?'<button class="button" data-action="openManagedBrowser">'+copy.openManagedBrowser+'</button>':'<button class="button" data-action="setupBrowser"'+(installedAgents.length?'':' disabled')+'>'+copy.enableBrowserControl+'</button>'):'';
  const agentRows=installedAgents.length?installedAgents.map(agent=>{const connected=browser.mcp_agents.includes(agent.id);return '<div class="list-row">'+dot(connected?'ready':'optional')+'<div class="list-main"><strong>'+esc(agent.label)+'</strong><small>'+esc(connected?copy.browserConnected:copy.browserNotConnected)+'</small></div><span class="badge '+(connected?'ready':'optional')+'">'+esc(connected?copy.browserConnected:copy.browserNotConnected)+'</span></div>';}).join(''):'<div class="empty">'+copy.browserNoAgents+'</div>';
  return pageHead(copy.browser,copy.browserPageHelp)+'<div class="grid">'+
    '<section class="card hero span-12"><div class="section-title"><div><h2>'+copy.browserControl+'</h2><span class="muted">'+esc(browser.version||copy.browserUnsupported)+'</span></div>'+dot(browserTone)+'</div><p>'+copy.browserControlHelp+'</p><div class="hero-actions">'+primary+(browser.supported&&browser.mcp_configured?'<button class="button ghost" data-action="setupBrowser"'+(installedAgents.length?'':' disabled')+'>'+copy.applyBrowserSettings+'</button>':'')+'<button class="button ghost" data-action="openDesktop">'+copy.openDesktop+'</button></div></section>'+
    '<section class="card span-4"><h3>'+copy.status+'</h3><div class="metric" style="font-size:18px">'+esc(browserStatus)+'</div><div class="muted">'+esc(browser.running?copy.browserRunning:copy.browserNotRunning)+'</div></section>'+
    '<section class="card span-4"><h3>'+copy.browserDefault+'</h3><div class="metric" style="font-size:18px">'+esc(browser.managed_default?copy.browserDefaultReady:copy.browserDefaultMissing)+'</div><div class="muted">chromium.desktop</div></section>'+
    '<section class="card span-4"><h3>'+copy.browserApproval+'</h3><div class="metric" style="font-size:18px">'+esc(browser.remote_debugging?copy.ready:copy.browserNotRunning)+'</div><div class="muted">127.0.0.1:'+esc(browser.debug_port||9222)+'</div></section>'+
    '<section class="card span-8"><div class="section-title"><div><h3>'+copy.browserMcpTitle+'</h3><span class="muted">'+copy.browserMcpHelp+'</span></div><button class="button ghost small" data-action="setupBrowser"'+(installedAgents.length&&browser.supported?'':' disabled')+'>'+copy.applyBrowserSettings+'</button></div><div class="list">'+agentRows+'</div></section>'+
    '<section class="card span-4"><div class="risk medium"><div class="risk-icon">!</div><div><h3>'+copy.browserBoundary+'</h3><p class="muted">'+copy.browserBoundaryDetail+'</p></div></div><p class="muted" style="color:var(--aw-warn);margin-top:18px">'+copy.browserSecurity+'</p></section>'+
    '<section class="card span-12"><div class="section-title"><h3>'+copy.browserSettings+'</h3><span class="badge '+(browser.managed_default?'ready':'warning')+'">'+esc(browser.managed_default?copy.browserDefaultReady:copy.browserDefaultMissing)+'</span></div><div class="list">'+
      '<div class="list-row">'+dot(browser.managed_default?'ready':'needs-attention')+'<div class="list-main"><strong>'+copy.browserDefault+'</strong><small>'+esc(browser.managed_default?copy.browserDefaultReady:copy.browserDefaultMissing)+'</small></div></div>'+
      '<div class="list-row">'+dot(browser.profile_name_applied?'ready':'needs-attention')+'<div class="list-main"><strong>'+copy.browserProfileName+'</strong><small>'+esc(browser.profile_name_applied?browser.profile_name:(browser.profile_name+' → '+browser.desired_profile_name+' · '+copy.browserProfileNamePending))+'</small></div><span class="badge '+(browser.profile_name_applied?'ready':'warning')+'">'+esc(browser.profile_name_applied?copy.ready:copy.attention)+'</span></div>'+
      '<div class="list-row">'+dot('ready')+'<div class="list-main"><strong>'+copy.browserProfile+'</strong><small>'+esc(browser.profile)+'</small></div></div>'+
      '<div class="list-row">'+dot(browser.remote_debugging?'ready':'optional')+'<div class="list-main"><strong>'+copy.browserEndpoint+'</strong><small>http://127.0.0.1:'+esc(browser.debug_port||9222)+'</small></div></div>'+
      '<div class="list-row">'+dot(browser.migration_pending?'needs-attention':browser.profile_exists?'ready':'optional')+'<div class="list-main"><strong>'+copy.browserMigration+'</strong><small>'+esc(browser.migration_pending?copy.browserMigrationPending:browser.profile_exists?copy.browserMigrationComplete:copy.browserProfileOnFirstLaunch)+'</small></div><span class="badge '+(browser.migration_pending?'warning':browser.profile_exists?'ready':'optional')+'">'+esc(browser.migration_pending?copy.attention:browser.profile_exists?copy.ready:copy.optional)+'</span></div></div></section></div>';
}
function networkPage(){
  const net=data.network;
  const tail=data.tailscale||{installed:false,state:'optional',serve_enabled:false,url:'',ips:[]};
  const routeActions=net.installed?'<button class="button" data-action="addRoute">＋ '+copy.addRoute+'</button> <button class="button ghost" data-action="checkProxy">'+copy.checkProxy+'</button>':'<button class="button" data-action="configureDomain">'+copy.configureDomain+'</button>';
  const tailStatus=tail.state==='connected'?copy.tailnetConnected:tail.installed?copy.tailnetLogin:copy.optional;
  const tailActions=!tail.installed?'<button class="button" data-action="installTailscale">'+copy.installTailscale+'</button>':tail.state!=='connected'?'<button class="button" data-action="connectTailscale">'+copy.connectTailscale+'</button>':'<button class="button ghost" data-action="serveTailscale">'+copy.exposeTailnet+'</button>'+(tail.serve_enabled&&tail.url?' <button class="button" data-action="openUrl" data-url="'+esc(tail.url)+'">'+copy.openTailnet+'</button>':'');
  return pageHead(copy.network,copy.networkHelp,routeActions)+'<div class="grid">'+
    '<section class="card span-6"><div class="section-title"><h3>'+copy.customDomain+'</h3>'+dot(net.mode==='incomplete'?'needs-attention':net.installed?'ready':'optional')+'</div><div class="metric" style="font-size:18px">'+esc(net.mode==='incomplete'?copy.attention:net.root_domain||copy.domainNotConfigured)+'</div><p class="muted">'+copy.domainHelp+'</p><button class="button ghost small" data-action="configureDomain">'+copy.configureDomain+'</button></section>'+
    '<section class="card span-6"><div class="section-title"><h3>'+copy.tailscale+'</h3>'+dot(tail.state==='connected'?'ready':'optional')+'</div><div class="metric" style="font-size:18px">'+esc(tailStatus)+'</div><p class="muted">'+esc(tail.dns_name||tail.ips.join(' · ')||copy.tailscaleHelp)+'</p><div class="hero-actions">'+tailActions+'</div></section>'+
    '<section class="card span-12"><div class="section-title"><h3>SSH / '+copy.access+'</h3></div><div class="list"><div class="list-row">'+dot('ready')+'<div class="list-main"><strong>code-server</strong><small>'+esc(data.access.code_server_local)+'</small></div></div></div><div class="code">ssh -N -L 8443:127.0.0.1:8443 user@server</div><button class="button ghost small" style="margin-top:9px" data-action="copyTunnel">'+copy.copyTunnel+'</button></section>'+
    '<section class="card span-12"><div class="section-title"><h3>'+copy.proxyRoutes+'</h3><span class="muted">'+esc(net.root_domain||copy.optional)+'</span></div><div class="list">'+(net.routes.length?net.routes.map(r=>'<div class="list-row">'+dot('ready')+'<div class="list-main"><strong>'+esc(r.host||r.name)+'</strong><small>'+esc(r.target)+'</small></div>'+(r.public_url?'<button class="button ghost small" data-action="openUrl" data-url="'+esc(r.public_url)+'">'+copy.open+'</button>':'')+'<button class="button danger small" data-action="removeRoute" data-name="'+esc(r.name)+'" data-host="'+esc(r.host)+'">'+copy.remove+'</button></div>').join(''):'<div class="empty">'+copy.noRoutes+'</div>')+'</div></section>'+
    '<section class="card span-12"><div class="section-title"><h3>'+copy.listeningPorts+'</h3></div><div class="table-wrap"><table><thead><tr><th>'+copy.command+'</th><th>PID</th><th>'+copy.address+'</th></tr></thead><tbody>'+(data.ports.length?data.ports.slice(0,40).map(p=>'<tr><td>'+esc(p.command)+'</td><td>'+esc(p.pid||'—')+'</td><td><code>'+esc(p.address)+'</code></td></tr>').join(''):'<tr><td colspan="3" class="empty">'+copy.noPorts+'</td></tr>')+'</tbody></table></div></section></div>';
}
function diagnosticsPage(){return pageHead(copy.diagnostics,copy.doctor,'<button class="button ghost" data-action="updateSource">'+copy.updateSource+'</button> <button class="button ghost" data-action="showOutput">'+copy.showOutput+'</button>')+'<div class="grid"><section class="card span-6"><div class="section-title"><h3>'+copy.capabilities+'</h3></div><div class="list">'+data.capabilities.map(c=>'<div class="list-row">'+dot(c.state)+'<div class="list-main"><strong>'+esc(c.label)+'</strong><small>'+esc(c.detail)+'</small></div><span class="badge '+(c.state==='ready'?'ready':c.state==='needs-attention'?'warning':'optional')+'">'+esc(stateLabel(c.state))+'</span></div>').join('')+'</div></section><section class="card span-6"><div class="section-title"><h3>'+copy.doctor+'</h3><span class="badge '+(data.doctor.healthy?'ready':'warning')+'">'+(data.doctor.healthy?copy.healthy:copy.attention)+'</span></div><div class="list">'+data.doctor.checks.map(c=>'<div class="list-row">'+dot(c.state)+'<div class="list-main"><strong>'+esc(c.id)+'</strong><small>'+esc(c.detail)+'</small></div></div>').join('')+'</div></section></div>';}
function wizard(){
  const labels=[copy.stepWelcome,copy.stepAgent,copy.stepWorkspace,copy.stepDesktop,copy.stepOptional];
  const tail=data.tailscale||{installed:false,state:'optional'};
  const desktop=data.desktop||{installed:false,runtime_available:false,running:false};
  const desktopReady=desktop.installed&&desktop.runtime_available;
  let body='';
  if(wizardStep===0)body='<h2>'+copy.onboardingTitle+'</h2><p>'+copy.onboardingSubtitle+'</p><div class="wizard-body"><section class="card hero"><h2>'+esc(data.bootstrap.foundation_ready?copy.foundationReady:copy.foundationWorking)+'</h2><p>'+copy.welcomeBody+'</p></section></div>';
  if(wizardStep===1)body='<h2>'+copy.stepAgent+'</h2><p>'+copy.agentBody+'</p><div class="wizard-body"><div class="agent-grid">'+data.agents.map(a=>'<section class="card agent-card"><strong>'+esc(a.label)+'</strong><p class="muted">'+esc(a.version||copy.versionUnknown)+'</p>'+(a.installed?(agentWebButton(a)||'<button class="button" data-action="launchAgent" data-command="'+esc(a.launch)+'">'+copy.launch+'</button>'):'<button class="button" data-action="installAgent" data-id="'+esc(a.id)+'">'+copy.install+'</button>')+'</section>').join('')+'</div></div>';
  if(wizardStep===2)body='<h2>'+copy.stepWorkspace+'</h2><p>'+copy.workspaceBody+'</p><div class="wizard-body"><section class="card"><h3>'+copy.openWorkspace+'</h3><div class="code">'+esc(data.workspace.root)+'</div><div class="hero-actions"><button class="button" data-action="openWorkspace">'+copy.openWorkspace+'</button><button class="button ghost" data-action="terminal">'+copy.openTerminal+'</button></div></section></div>';
  if(wizardStep===3)body='<h2>'+copy.stepDesktop+'</h2><p>'+copy.accessBody+'</p><div class="wizard-body grid"><section class="card span-6"><h3>'+copy.selkiesCapability+'</h3><div class="code">'+esc(data.access.desktop_local)+'</div><button class="button" style="margin-top:12px" data-action="'+(desktopReady?'openDesktop':'installDesktop')+'">'+(desktopReady?copy.openDesktop:copy.installDesktop)+'</button></section><section class="card span-6"><div class="section-title"><h3>'+copy.desktopSession+'</h3>'+dot(desktop.running?'ready':'optional')+'</div><div class="metric" style="font-size:18px">'+esc(desktop.running?copy.ready:copy.optional)+'</div><p class="muted">'+esc(desktop.service||'svc-selkies')+'</p><button class="button ghost" data-nav="desktop">'+copy.detail+'</button></section></div>';
  if(wizardStep===4)body='<h2>'+copy.stepOptional+'</h2><p>'+copy.optionalBody+'</p><div class="wizard-body"><section class="card"><div class="list">'+
    '<div class="list-row">'+dot(data.browser&&data.browser.state==='ready'?'ready':data.browser&&data.browser.state==='needs-restart'?'needs-attention':'optional')+'<div class="list-main"><strong>'+copy.browser+'</strong><small>'+esc(data.browser&&data.browser.state==='ready'?copy.browserControlReady:copy.browserNeedsSetup)+'</small></div><button class="button ghost small" data-nav="browser">'+copy.detail+'</button></div>'+
    '<div class="list-row">'+dot(data.skills.shared_count?'ready':'optional')+'<div class="list-main"><strong>'+copy.resources+'</strong><small>'+esc(data.mcp.count+' MCP · '+data.skills.shared_count+' Skills')+'</small></div><button class="button ghost small" data-nav="resources">'+copy.detail+'</button></div>'+
    '<div class="list-row">'+dot(data.network.mode==='incomplete'?'needs-attention':data.network.installed?'ready':'optional')+'<div class="list-main"><strong>'+copy.customDomain+'</strong><small>'+esc(data.network.mode==='incomplete'?copy.attention:data.network.root_domain||copy.optional)+'</small></div><button class="button ghost small" data-action="configureDomain">'+copy.configureDomain+'</button></div>'+
    '<div class="list-row">'+dot(tail.state==='connected'?'ready':'optional')+'<div class="list-main"><strong>'+copy.tailscale+'</strong><small>'+esc(tail.state==='connected'?copy.tailnetConnected:tail.installed?copy.tailnetLogin:copy.optional)+'</small></div>'+(tail.state==='connected'?'<button class="button ghost small" data-nav="network">'+copy.detail+'</button>':tail.installed?'<button class="button ghost small" data-action="connectTailscale">'+copy.connectTailscale+'</button>':'<button class="button ghost small" data-action="installTailscale">'+copy.install+'</button>')+'</div>'+
    '<div class="list-row">'+dot('ready')+'<div class="list-main"><strong>'+copy.diagnostics+'</strong><small>workspacectl doctor</small></div><button class="button ghost small" data-nav="diagnostics">'+copy.detail+'</button></div></div></section></div>';
  const steps='<div class="steps"><h3>'+copy.quickStart+'</h3><div class="step-list">'+labels.map((label,i)=>'<div class="step '+(i===wizardStep?'active':i<wizardStep?'done':'')+'"><span class="step-index">'+(i<wizardStep?'✓':i+1)+'</span><span>'+label+'</span></div>').join('')+'</div></div>';
  const actions='<div class="wizard-actions"><button class="button ghost skip" data-action="completeOnboarding">'+copy.skip+'</button>'+(wizardStep?'<button class="button ghost" data-wizard="back">'+copy.back+'</button>':'')+'<button class="button" data-wizard="'+(wizardStep===4?'finish':'next')+'">'+(wizardStep===4?copy.finish:copy.next)+'</button></div>';
  return '<div class="wizard"><section class="card wizard-shell">'+steps+'<div class="wizard-content">'+body+actions+'</div></section></div>';
}
function render(){nav();health();if(!data)return;const onboarding=!data.bootstrap.onboarding.complete||forceWelcome;if(onboarding&&page==='overview'){document.getElementById('content').innerHTML=wizard();return;}const pages={overview,agents:agentsPage,browser:browserPage,resources:resourcesPage,services:servicesPage,desktop:desktopPage,network:networkPage,diagnostics:diagnosticsPage};document.getElementById('content').innerHTML=pages[page]();const serviceInput=document.getElementById('service-filter');if(serviceInput){serviceInput.focus();serviceInput.setSelectionRange(serviceInput.value.length,serviceInput.value.length);serviceInput.addEventListener('input',e=>{serviceFilter=e.target.value;render();});}const resourceInput=document.getElementById('resource-filter');if(resourceInput){resourceInput.focus();resourceInput.setSelectionRange(resourceInput.value.length,resourceInput.value.length);resourceInput.addEventListener('input',e=>{resourceFilter=e.target.value;render();});}}
document.addEventListener('click',event=>{const navButton=event.target.closest('[data-nav]');if(navButton){page=navButton.dataset.nav;forceWelcome=false;render();return;}const wizardButton=event.target.closest('[data-wizard]');if(wizardButton){if(wizardButton.dataset.wizard==='back')wizardStep=Math.max(0,wizardStep-1);else if(wizardButton.dataset.wizard==='finish')post('completeOnboarding');else wizardStep=Math.min(4,wizardStep+1);render();return;}const button=event.target.closest('[data-action]');if(!button)return;const action=button.dataset.action;if(action==='refresh')post('refresh');else if(action==='terminal')post('terminal');else if(action==='openWorkspace')post('openWorkspace');else if(action==='openDesktop')post('openDesktop');else if(action==='openPath'&&/^\\/proxy\\/\\d+\\/$/.test(button.dataset.path||''))window.open(button.dataset.path,'_blank','noopener');else if(action==='launchAgent')post('terminal',{command:button.dataset.command});else if(action==='updateOpenClaw')post('updateOpenClaw');else if(action==='installAgent')post('installAgent',{id:button.dataset.id});else if(action==='installMcpm')post('installMcpm');else if(action==='addSkill')post('addSkill');else if(action==='updateSkills')post('updateSkills');else if(action==='updateSkill')post('updateSkill',{id:button.dataset.id});else if(action==='removeSkill')post('removeSkill',{id:button.dataset.id});else if(action==='addMcp')post('addMcp');else if(action==='removeMcp')post('removeMcp',{id:button.dataset.id,agents:button.dataset.agents,managed:button.dataset.managed==='true'});else if(action==='service')post('serviceAction',{id:button.dataset.id,kind:button.dataset.kind,action:button.dataset.op});else if(action==='logs')post('showLogs',{id:button.dataset.id});else if(action==='installProxy')post('installProxy');else if(action==='checkProxy')post('checkProxy');else if(action==='addRoute')post('addRoute');else if(action==='removeRoute')post('removeRoute',{name:button.dataset.name,host:button.dataset.host});else if(action==='openUrl')post('openUrl',{url:button.dataset.url});else if(action==='copyTunnel'){post('copyTunnel');toast(copy.copyTunnel);}else if(action==='copyDesktopTunnel'){post('copyDesktopTunnel');toast(copy.copyDesktopTunnel);}else if(action==='completeOnboarding')post('completeOnboarding');else if(action==='showOutput')post('showOutput');else if(action==='updateSource')post('updateSource');});
document.addEventListener('click',event=>{const button=event.target.closest('[data-action]');if(!button)return;const action=button.dataset.action;if(['configureDomain','installDesktop','setupBrowser','openManagedBrowser','installTailscale','connectTailscale','serveTailscale','setupComputerUse','desktopEmergencyStop'].includes(action))post(action);else if(action==='desktopControl')post('desktopControl',{action:button.dataset.op});});
document.getElementById('language-toggle').addEventListener('click',event=>{event.stopPropagation();post('switchLanguage');});
window.addEventListener('message',event=>{const message=event.data;if(message.type==='snapshot'){data=message.data;if(typeof message.forceWelcome==='boolean')forceWelcome=message.forceWelcome;render();}else if(message.type==='showWelcome'){forceWelcome=message.value;wizardStep=0;page='overview';render();}else if(message.type==='refreshing'){document.getElementById('health-label').textContent=copy.refreshing;}else if(message.type==='error'){document.getElementById('content').innerHTML='<div class="empty"><h2>'+copy.attention+'</h2><p>'+esc(message.message)+'</p><button class="button" data-action="refresh">'+copy.refresh+'</button></div>';}});post('ready');
</script></body></html>`;
}

function activate(context) {
  const controller = new ControlCenter(context);
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider(VIEW_ID, { resolveWebviewView: (view) => controller.resolveWebviewView(view) }),
    vscode.commands.registerCommand('agentWorkspace.openControlCenter', () => controller.open(false)),
    vscode.commands.registerCommand('agentWorkspace.refresh', () => controller.refresh(true)),
    vscode.commands.registerCommand('agentWorkspace.showWelcome', () => controller.open(true))
  );

  controller.refresh().then((snapshot) => {
    if (!snapshot) return;
    const autoOpen = controller.config().autoOpen;
    const incomplete = snapshot.bootstrap && snapshot.bootstrap.onboarding && !snapshot.bootstrap.onboarding.complete;
    if (autoOpen && incomplete) {
      setTimeout(() => controller.open(true), 900);
    }
  });
}

function deactivate() {}

return { activate, deactivate, ControlCenter, controlCenterHtml, quickViewHtml, isSnapshotHealthy, desktopExternalUri };
};
