import { useEffect, useId, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Box, Check, CircleCheck, Code, Copy, Download, HardDrive, LoaderCircle, Monitor, Play, Plus, RefreshCw, ShieldCheck, Square, Terminal, X } from 'lucide-react';
import '../devices/devices.css';
import './spaces.css';
import type { DirectClient } from '../devices/DirectDevicesView';
import { RemoteSpaces } from './RemoteSpaces';

type PullPolicy = 'always' | 'missing' | 'never';
type Provider = 'docker' | 'colima' | 'orbstack' | 'wsl';
type Registry = 'auto' | 'dockerhub' | 'ghcr' | 'aliyun';
type Desktop = 'xfce' | 'kde' | 'lxqt';
type Display = 'wayland' | 'x11';
type Images = { registries: { id: Exclude<Registry, 'auto'>; repository: string }[]; desktops: { id: Desktop; tag: string }[] };
type Space = { id: string; name: string; provider: Provider; image: string; registry?: Registry | null; desktop?: Desktop | null; display?: Display; pull_policy?: PullPolicy; node: string; phase: string; error: string | null; volume: string; ports: { peer?: number; desktop?: number; code?: number } };
type Catalog = { spaces: Space[]; platform: string; default_image: string; images?: Images; setup: { action: string; phase: string; error: string | null } | null };
type Preflight = { ready: boolean; detail: string; notice: string; containerized: boolean; engine: { name: string; version: string; os: string } | null; actions: string[] };
type Plan = { action: string; plan_id: string; expires_in_seconds: number; summary: string; commands: string; administrator: boolean; restart_may_be_required: boolean; requires_homebrew: boolean };
type ImageEdit = { id: string; image: string; registry: Registry | null; display: Display; pullPolicy: PullPolicy; error: string };
export type SpacesClient = (command: Record<string, unknown>) => Promise<any>;
const busyPhases = ['preparing', 'pulling', 'creating', 'starting', 'enrolling', 'stopping', 'removing'];
const providerName: Record<Provider, string> = { docker: 'Docker', colima: 'Colima', orbstack: 'OrbStack', wsl: 'WSL2 · Ubuntu' };
const desktopName: Record<Desktop, string> = { xfce: 'XFCE', kde: 'KDE Plasma', lxqt: 'LXQt' };
const registryIds: Registry[] = ['auto', 'dockerhub', 'ghcr', 'aliyun'];

export function SpacesView({ client, language, onOpenTerminal, onOpenApplication, onProjects, direct, onDevices }: { client: SpacesClient; direct: DirectClient; onDevices: () => void; language: string; onOpenTerminal: (node: string) => void; onOpenApplication: (id: string, application: 'desktop' | 'code') => Promise<void>; onProjects: () => void }) {
  const t = (zh: string, en: string) => language.startsWith('zh') ? zh : en;
  const [catalog, setCatalog] = useState<Catalog | null>(null), [error, setError] = useState(''), [loadError, setLoadError] = useState(''), [notice, setNotice] = useState('');
  const [pending, setPending] = useState(false), [wizard, setWizard] = useState(false), [step, setStep] = useState(0);
  const [opening, setOpening] = useState('');
  const [provider, setProvider] = useState<Provider>('docker'), [check, setCheck] = useState<Preflight | null>(null), [plan, setPlan] = useState<Plan | null>(null);
  const [name, setName] = useState(''), [image, setImage] = useState(''), [pullPolicy, setPullPolicy] = useState<PullPolicy>('always');
  const [registry, setRegistry] = useState<Registry>('auto'), [desktop, setDesktop] = useState<Desktop>('xfce'), [display, setDisplay] = useState<Display>('wayland'), [customImage, setCustomImage] = useState(false);
  const [latency, setLatency] = useState<Record<string, number | null> | null>(null);
  const [credentials, setCredentials] = useState<{ id: string; username: string; password: string } | null>(null), [copied, setCopied] = useState(false);
  const [imageEdit, setImageEdit] = useState<ImageEdit | null>(null);
  const imageEditTrigger = useRef<HTMLButtonElement | null>(null), imageEditError = useRef<HTMLParagraphElement>(null), retryCard = useRef<HTMLElement | null>(null);
  const active = useRef(false), loaded = useRef(false), panel = useRef<HTMLDivElement>(null), createButton = useRef<HTMLButtonElement>(null);
  const requesting = useRef(false), reviewPanel = useRef<HTMLDivElement>(null), reviewTrigger = useRef<HTMLButtonElement | null>(null), restoreReviewFocus = useRef(false);
  const errorPanel = useRef<HTMLParagraphElement>(null), setupStatus = useRef<HTMLDivElement>(null), focusSetup = useRef(false);
  const knownRoutes = useRef<string | null>(null);
  const id = useId();
  const operationRunning = !!catalog?.spaces.some(space => busyPhases.includes(space.phase)) || catalog?.setup?.phase === 'installing';
  const locked = pending || operationRunning;
  const phaseName = (phase: string) => ({ preparing:t('准备中', 'Preparing'), pulling:t('下载镜像', 'Downloading image'), creating:t('创建容器', 'Creating container'), starting:t('启动中', 'Starting'), enrolling:t('等待节点就绪', 'Waiting for node'), ready:t('可用', 'Ready'), stopped:t('已停止', 'Stopped'), stopping:t('停止中', 'Stopping'), removing:t('归档中', 'Archiving'), archived:t('已归档 · 数据保留', 'Archived · data retained'), interrupted:t('操作中断', 'Interrupted'), failed:t('需要处理', 'Needs attention') }[phase] || phase);
  function updateCatalog(value: Catalog) {
    setCatalog(value);
    const routes = value.spaces.filter(space => space.phase === 'ready').map(space => space.node).sort().join(',');
    if (knownRoutes.current !== routes) { knownRoutes.current = routes; window.dispatchEvent(new Event('cranibox:peers-changed')); }
  }
  async function refresh() {
    if (active.current) return;
    active.current = true;
    try {
      const value: Catalog = await client({ operation: 'list' }); updateCatalog(value); setLoadError('');
      if (!loaded.current) { loaded.current = true; setProvider(value.platform === 'windows' ? 'wsl' : value.platform === 'macos' ? 'colima' : 'docker'); setImage(value.default_image); }
    } catch (error) { setLoadError(String(error)); } finally { active.current = false; }
  }
  useEffect(() => {
    let disposed = false;
    async function poll() { if (!disposed && !document.hidden) await refresh(); }
    void poll(); const timer = setInterval(poll, operationRunning ? 2000 : 15000);
    document.addEventListener('visibilitychange', poll);
    return () => { disposed = true; clearInterval(timer); document.removeEventListener('visibilitychange', poll); };
  }, [client, operationRunning]);
  useEffect(() => { if (wizard) panel.current?.focus(); }, [wizard, step]);
  useEffect(() => {
    if (wizard && plan) reviewPanel.current?.focus();
    else if (restoreReviewFocus.current) { restoreReviewFocus.current = false; reviewTrigger.current?.focus(); }
  }, [wizard, plan]);
  useEffect(() => { if (error) errorPanel.current?.focus(); }, [error]);
  useEffect(() => { if (imageEdit?.error) imageEditError.current?.focus(); }, [imageEdit?.error]);
  useEffect(() => { if (!imageEdit && retryCard.current) { retryCard.current.focus(); retryCard.current = null; } }, [imageEdit]);
  useEffect(() => { if (focusSetup.current && catalog?.setup) { focusSetup.current = false; setupStatus.current?.focus(); } }, [catalog]);
  async function request(command: Record<string, unknown>, reportError: (message: string) => void = setError) {
    if (locked || requesting.current) return;
    requesting.current = true;
    setPending(true); setError(''); setNotice('');
    try { const value = await client(command); if (value?.spaces) updateCatalog(value); return value; }
    catch (error) { reportError(String(error)); }
    finally { requesting.current = false; setPending(false); }
  }
  async function install() {
    if (!plan || locked || requesting.current) return;
    const reviewed = plan;
    // A lost response can mean setup already started. Never offer the same confirmation again.
    setPlan(null); focusSetup.current = true;
    const value = await request({ operation: 'install', action: reviewed.action, plan_id: reviewed.plan_id });
    if (value) setCheck(null);
    else { focusSetup.current = false; void refresh(); }
  }
  async function preflight() {
    const result = await request({ operation: 'preflight', provider });
    if (result) { setCheck(result); setPlan(null); if (result.ready) { setStep(1); void probeRegistries(); } }
  }
  async function probeRegistries() {
    setLatency(null);
    try { const rows: { id: string; milliseconds: number | null }[] = await client({ operation: 'registries' }); setLatency(Object.fromEntries(rows.map(row => [row.id, row.milliseconds]))); }
    catch { setLatency({}); }
  }
  const registryName = (value: Registry) => ({ auto: t('自动（最快可达）', 'Automatic (fastest reachable)'), dockerhub: 'Docker Hub', ghcr: t('GitHub 镜像', 'GitHub Container Registry'), aliyun: t('阿里云镜像', 'Alibaba Cloud mirror') }[value]);
  const fastest = latency && Object.entries(latency).filter(([, ms]) => ms !== null).sort((a, b) => (a[1] as number) - (b[1] as number))[0]?.[0] as Registry | undefined;
  const registryLabel = (value: Registry) => {
    if (value === 'auto') return registryName(value) + (fastest ? ' · ' + registryName(fastest) : '');
    if (!latency) return registryName(value);
    const ms = latency[value];
    return registryName(value) + ' · ' + (ms === null || ms === undefined ? t('不可达', 'unreachable') : ms + ' ms');
  };
  const repository = (value: Registry) => catalog?.images?.registries.find(row => row.id === (value === 'auto' ? fastest || 'dockerhub' : value))?.repository;
  const publishedImage = repository(registry) ? `${repository(registry)}:ubuntu-${desktop}` : '';
  const displayName = (value: Display) => value === 'wayland' ? t('Wayland（支持 Computer Use）', 'Wayland (Computer Use)') : t('X11（兼容模式）', 'X11 (compatibility)');
  async function openApplication(space: Space, application: 'desktop' | 'code') {
    if (opening) return;
    setOpening(space.id + application); setError(''); setNotice('');
    try { await onOpenApplication(space.id, application); setNotice(t('已在浏览器中打开。关闭标签页不会停止空间。', 'Opened in your browser. Closing the tab does not stop the space.')); }
    catch (error) { setError(String(error)); }
    finally { setOpening(''); }
  }
  async function create() {
    const value = await request({ operation: 'create', name: name.trim(), provider, desktop, display, pull_policy: pullPolicy, ...(customImage ? { image } : { registry }) });
    if (value) { setWizard(false); setStep(0); setName(''); setNotice(t('空间正在后台创建，可以关闭此页面。', 'Your space is being created in the background. You can close this page.')); createButton.current?.focus(); }
  }
  async function act(space: Space, operation: 'resume' | 'stop' | 'remove') {
    if (operation === 'stop' && !window.confirm(t('停止此空间会中断其中的 Agent 和 Shell。数据会保留，继续？', 'Stopping this space interrupts its Agents and shells. Data is retained. Continue?'))) return;
    if (operation === 'remove' && !window.confirm(t('归档将停止并移除此容器，其中的进程会结束。数据卷和节点身份保留，之后可恢复。继续？', 'Archive stops and removes this container and ends its processes. Its data volume and node identity are retained for restoration. Continue?'))) return;
    const value = await request({ operation, id: space.id });
    if (value) { setCredentials(null); setNotice(t('操作已在后台开始。', 'Operation started in the background.')); }
  }
  async function access(space: Space) {
    const value = await request({ operation: 'credentials', id: space.id });
    if (value) { setCredentials({ id: space.id, ...value }); setCopied(false); }
  }
  async function retryWithImage(space: Space, card: HTMLElement) {
    if (!imageEdit || imageEdit.id !== space.id) return;
    setImageEdit(current => current && { ...current, error: '' });
    const source = imageEdit.registry ? { registry: imageEdit.registry } : { image: imageEdit.image };
    const value = await request({ operation: 'resume', id: space.id, ...source, display: imageEdit.display, pull_policy: imageEdit.pullPolicy }, message => setImageEdit(current => current?.id === space.id ? { ...current, error: message } : current));
    if (value) {
      retryCard.current = card; setImageEdit(null); setCredentials(null);
      setNotice(t('已保存镜像设置，正在后台重试。空间身份和数据卷保持不变。', 'Image settings saved. Retrying in the background with the same space identity and data volume.'));
    }
  }
  const policyName = (policy: PullPolicy) => ({ always: t('每次拉取', 'Always pull'), missing: t('缺少时拉取', 'Pull if missing'), never: t('仅使用本地镜像', 'Use local image only') }[policy]);
  const actions: Record<string, string> = { linux_docker:t('安装 Docker Engine', 'Install Docker Engine'), colima_install:t('安装 Colima 与 Docker CLI', 'Install Colima and Docker CLI'), colima_start:t('启动 Colima', 'Start Colima'), orbstack_install:t('安装 OrbStack', 'Install OrbStack'), orbstack_start:t('启动 OrbStack', 'Start OrbStack'), wsl_install:t('安装 WSL2 与 Ubuntu', 'Install WSL2 and Ubuntu'), wsl_docker:t('在 Ubuntu 中安装 Docker', 'Install Docker in Ubuntu') };
  const providers: Provider[] = catalog?.platform === 'macos' ? ['colima', 'orbstack', 'docker'] : catalog?.platform === 'windows' ? ['wsl', 'docker'] : ['docker'];
  function closeWizard() { setWizard(false); setPlan(null); createButton.current?.focus(); }
  return <section className="direct-devices virtual-spaces" aria-label={t('虚拟空间', 'Virtual spaces')}>
    <div className="direct-heading"><div><p className="spaces-eyebrow">CRANIBOX SPACES</p><h2>{t('给每个任务，一个独立空间', 'A separate space for every task')}</h2><p>{t('在这台管理设备上运行完整工作区，作为独立节点加入设备网络。', 'Run complete workspaces on this management device. Each joins your device network as its own node.')}</p></div><div className="direct-actions"><button aria-label={t('刷新空间', 'Refresh spaces')} disabled={pending} onClick={() => void refresh()}><RefreshCw size={16} aria-hidden="true" /></button><button ref={createButton} className="direct-primary" disabled={!catalog || locked} onClick={() => { setWizard(true); setStep(0); setCheck(null); setPlan(null); setError(''); }}><Plus size={17} aria-hidden="true" />{t('新建空间', 'New space')}</button></div></div>
    {(error || loadError) && <p ref={errorPanel} tabIndex={-1} className="direct-error" role="alert">{error || loadError}</p>}{notice && <p className="direct-notice" role="status">{notice}</p>}
    {!catalog && !loadError && <p role="status">{t('正在读取空间…', 'Loading spaces…')}</p>}
    {catalog?.setup && <div ref={setupStatus} tabIndex={-1} className="spaces-setup-status" role="status"><div><strong>{actions[catalog.setup.action] || catalog.setup.action}</strong><p>{catalog.setup.phase === 'installing' ? t('安装正在后台执行，请完成操作系统的授权提示。关闭页面不会取消安装。', 'Setup is running in the background. Complete any operating system approval prompts. Closing this page does not cancel setup.') : catalog.setup.phase === 'check_required' ? t('安装命令已结束。完成首次启动、重启或重新登录后，请重新检测环境。', 'The setup command finished. Complete first-run setup, restart or sign in again if needed, then recheck.') : catalog.setup.error || t('安装监控中断，系统安装程序可能仍在运行。请检查软件包管理器并重新检测环境。', 'Setup monitoring was interrupted. The system installer may still be running. Check the package manager and recheck the environment.')}</p></div>{catalog.setup.phase === 'installing' && <LoaderCircle className="spaces-spinner" size={20} aria-hidden="true" />}</div>}
    {wizard && <div ref={panel} tabIndex={-1} className="spaces-wizard" aria-label={t('创建空间向导', 'Create space wizard')}>
      <div className="direct-heading"><h3>{t('创建虚拟空间', 'Create a virtual space')}</h3><button aria-label={t('关闭向导', 'Close wizard')} onClick={closeWizard}><X size={17} aria-hidden="true" /></button></div>
      <ol className="spaces-steps">{[t('运行环境', 'Environment'), t('空间设置', 'Settings'), t('确认创建', 'Review')].map((title, index) => <li key={title} className={index <= step ? 'active' : ''} aria-current={index === step ? 'step' : undefined}><span>{index < step ? <Check size={14} aria-hidden="true" /> : index + 1}</span>{title}</li>)}</ol>
      {step === 0 && <div className="spaces-step-content"><h4>{t('选择这台设备的容器环境', 'Choose this device’s container runtime')}</h4><p className="spaces-muted">{t('检测和安装发生在运行 Cranibox 的设备上。', 'Detection and installation run on the device hosting Cranibox.')} {catalog?.platform === 'macos' ? t('推荐开源 Colima，也支持 OrbStack 和已安装的 Docker。', 'Open-source Colima is recommended; OrbStack and an existing Docker engine also work.') : catalog?.platform === 'windows' ? t('推荐 WSL2 + Ubuntu，也支持已有的 Docker Linux 引擎。', 'WSL2 + Ubuntu is recommended; an existing Docker Linux engine also works.') : t('使用本机 Docker Engine。容器中的管理节点需要可访问的宿主机 Docker socket。', 'Use local Docker Engine. A container manager needs access to its host’s Docker socket.')}</p>
        <fieldset className="spaces-providers"><legend>{t('容器运行环境', 'Container runtime')}</legend>{providers.map(value => <label key={value}><input type="radio" name={id + '-provider'} value={value} checked={provider === value} disabled={locked} onChange={() => { setProvider(value); setCheck(null); setPlan(null); }} /><span><strong>{providerName[value]}</strong>{value === 'colima' && <small>{t('开源 · 基于 Linux 虚拟机', 'Open source · Linux virtual machine')}</small>}{value === 'orbstack' && <small>{t('需接受 OrbStack 许可', 'Subject to OrbStack licensing')}</small>}</span></label>)}</fieldset>
        <button className="direct-primary" disabled={locked} onClick={() => void preflight()}>{pending ? <LoaderCircle className="spaces-spinner" size={17} aria-hidden="true" /> : <ShieldCheck size={17} aria-hidden="true" />}{pending ? t('正在检测…', 'Checking…') : t('检测环境', 'Check environment')}</button>
        {check && !check.ready && <div className="spaces-environment"><h4>{t('还需要准备环境', 'Environment needs setup')}</h4><p role="status">{check.detail}</p><p className="spaces-muted">{check.notice}</p><div className="direct-actions">{check.actions.map(action => <button key={action} disabled={locked || !!plan} onClick={async event => { reviewTrigger.current = event.currentTarget; const value = await request({ operation: 'plan', action }); if (value) setPlan(value); }}><Download size={16} aria-hidden="true" />{actions[action] || action}</button>)}</div>{!check.actions.length && <p>{t('请在宿主机安装并启动 Docker，检查 socket 权限和引擎连接后重新检测。', 'Install and start Docker on the host, check socket access and the engine connection, then recheck.')}</p>}</div>}
        {plan && <div ref={reviewPanel} tabIndex={-1} className="spaces-install-plan" role="region" aria-label={t('检查安装操作', 'Review setup action')}><h4>{t('检查安装操作', 'Review setup action')}</h4><p>{plan.summary}</p><p className="spaces-muted">{t('此确认十分钟内有效，仅能使用一次。执行后请重新检测环境，确认运行环境已就绪。', 'This confirmation is valid for ten minutes and can be used once. Recheck the environment after setup to confirm the runtime is ready.')}</p>{plan.requires_homebrew && <p>{t('需要先安装 Homebrew；若尚未安装，请从 brew.sh 完成其官方安装流程。', 'Homebrew must be installed first. If missing, complete its official installation at brew.sh.')}</p>}{plan.restart_may_be_required && <p>{t('可能需要重启 Windows，并首次打开 Ubuntu 创建用户。', 'Windows may need a restart. Open Ubuntu once to create your user.')}</p>}<pre tabIndex={0} aria-label={t('将执行的安装命令', 'Setup commands to execute')}>{plan.commands}</pre><div className="direct-actions"><button className="direct-primary" disabled={locked || !plan.plan_id} onClick={() => void install()}>{t('确认并执行安装', 'Confirm and run setup')}</button><button disabled={locked} onClick={() => { restoreReviewFocus.current = true; setPlan(null); }}>{t('取消', 'Cancel')}</button></div></div>}
      </div>}
      {step === 1 && <form onSubmit={event => { event.preventDefault(); setStep(2); }} className="spaces-step-content"><div className="spaces-engine"><CircleCheck size={20} aria-hidden="true" /><div><strong>{providerName[provider]} {t('已就绪', 'is ready')}</strong><p>{check?.engine?.name} · {check?.engine?.os} · {check?.engine?.version}</p>{check?.detail && <p>{check.detail}</p>}</div></div><label>{t('空间名称', 'Space name')}<input required maxLength={120} autoFocus value={name} onChange={event => setName(event.target.value)} placeholder={t('例如：日常开发、Agent 实验', 'e.g. Daily development')} /></label><label htmlFor={id + '-desktop'}>{t('桌面环境', 'Desktop')}</label><select id={id + '-desktop'} value={desktop} disabled={customImage} onChange={event => setDesktop(event.target.value as Desktop)}>{(['xfce', 'kde', 'lxqt'] as Desktop[]).map(value => <option key={value} value={value}>{desktopName[value]}</option>)}</select><label htmlFor={id + '-display'}>{t('显示协议', 'Display server')}</label><select id={id + '-display'} value={display} onChange={event => setDisplay(event.target.value as Display)}>{(['wayland', 'x11'] as Display[]).map(value => <option key={value} value={value}>{displayName(value)}</option>)}</select><p className="spaces-muted">{t('镜像同时包含 x86-64 与 ARM64，按 Docker 引擎的架构自动选择。', 'Images include x86-64 and ARM64; the Docker engine’s architecture is selected automatically.')}</p><details className="spaces-advanced"><summary>{t('高级设置：镜像源', 'Advanced: image source')}</summary><label htmlFor={id + '-registry'}>{t('镜像源', 'Registry')}</label><select id={id + '-registry'} value={registry} disabled={customImage} onChange={event => setRegistry(event.target.value as Registry)}>{registryIds.map(value => <option key={value} value={value}>{registryLabel(value)}</option>)}</select>{!customImage && publishedImage && <p className="spaces-muted"><code>{publishedImage}</code></p>}<label className="spaces-check"><input type="checkbox" checked={customImage} onChange={event => { setCustomImage(event.target.checked); if (event.target.checked && !image) setImage(publishedImage || catalog?.default_image || ''); }} />{t('使用自定义镜像', 'Use a custom image')}</label>{customImage && <label>{t('镜像地址', 'Image reference')}<input required maxLength={240} pattern="[A-Za-z0-9._/:@-]+" value={image} onChange={event => setImage(event.target.value)} /></label>}<label htmlFor={id + '-pull-policy'}>{t('镜像拉取策略', 'Image pull policy')}</label><select id={id + '-pull-policy'} value={pullPolicy} onChange={event => setPullPolicy(event.target.value as PullPolicy)}>{(['always', 'missing', 'never'] as PullPolicy[]).map(policy => <option key={policy} value={policy}>{policyName(policy)}</option>)}</select><p className="spaces-muted">{pullPolicy === 'never' ? t('使用 Docker 引擎上已有的兼容镜像，不访问镜像仓库；缺少镜像时会提示处理。适合离线环境或本地构建。', 'Use a compatible image already on the Docker engine, without contacting a registry. Missing images need to be built or loaded first. Suitable for offline use or local builds.') : t('使用兼容 Cranibox 的 Linux 镜像。策略也用于归档后的重建；恢复现有容器不会自动升级。', 'Use a compatible Cranibox Linux image. This policy also applies when restoring an archive; resuming an existing container does not upgrade it.')}</p></details><div className="direct-actions"><button type="button" onClick={() => setStep(0)}><ArrowLeft size={16} aria-hidden="true" />{t('上一步', 'Back')}</button><button className="direct-primary" type="submit" disabled={locked || !name.trim()}>{t('下一步', 'Next')}<ArrowRight size={16} aria-hidden="true" /></button></div></form>}
      {step === 2 && <div className="spaces-step-content"><h4>{name}</h4><dl className="spaces-review"><div><dt>{t('运行环境', 'Runtime')}</dt><dd>{providerName[provider]}</dd></div><div><dt>{t('资源上限', 'Resource limits')}</dt><dd>2 CPU · 4 GiB RAM</dd></div><div><dt>{t('数据存储', 'Storage')}</dt><dd>{t('独立持久数据卷 · /config', 'Dedicated persistent volume · /config')}</dd></div><div><dt>{t('桌面', 'Desktop')}</dt><dd>{customImage ? t('由自定义镜像决定', 'Set by the custom image') : desktopName[desktop]} · {displayName(display)}</dd></div><div><dt>{t('镜像', 'Image')}</dt><dd>{customImage ? <code>{image}</code> : <>{registryLabel(registry)}<br /><code>{publishedImage}</code></>}</dd></div><div><dt>{t('镜像拉取策略', 'Image pull policy')}</dt><dd>{policyName(pullPolicy)}</dd></div></dl><p className="spaces-muted">{t('将按所选策略准备镜像并启动容器，首次启动可能需要几分钟。空间以独立 NodeID 接入，授权当前管理设备使用其命令、终端和项目能力。桌面与编辑器端口仅发布到引擎宿主机的回环地址。', 'The image will be prepared using the selected policy, then started; first boot can take several minutes. The space gets its own NodeID and grants this manager command, terminal and project access. Desktop and editor ports are published only on the engine host’s loopback address.')}</p><div className="direct-actions"><button disabled={locked} onClick={() => setStep(1)}><ArrowLeft size={16} aria-hidden="true" />{t('上一步', 'Back')}</button><button className="direct-primary" disabled={locked} onClick={() => void create()}><Plus size={16} aria-hidden="true" />{pending ? t('提交中…', 'Submitting…') : t('创建并启动', 'Create and start')}</button></div></div>}
    </div>}
    {catalog && !catalog.spaces.length && !wizard && <div className="spaces-empty"><div className="spaces-empty-icon"><Box size={40} strokeWidth={1.5} aria-hidden="true" /></div><h3>{t('此设备尚未管理空间', 'No spaces managed on this device')}</h3><p>{t('为开发和 Agent 任务准备独立的 Linux 工作区。向导会先检测运行环境，再完成部署与节点接入。', 'Give development and Agent tasks their own Linux workspace. The wizard checks your runtime, deploys the container and enrolls its node.')}</p><div className="spaces-benefits"><span><HardDrive size={16} aria-hidden="true" />{t('数据独立保留', 'Persistent data')}</span><span><ShieldCheck size={16} aria-hidden="true" />{t('独立节点授权', 'Scoped node access')}</span><span><Terminal size={16} aria-hidden="true" />{t('统一项目与终端', 'Shared projects and terminals')}</span></div></div>}
    {catalog && catalog.spaces.length > 0 && <h3 className="spaces-local-heading">{t('此设备管理的空间', 'Spaces managed here')}</h3>}
    <div className="spaces-grid">{catalog?.spaces.map(space => <article key={space.id} className="space-card" aria-label={space.name} tabIndex={-1}><div className="space-card-head"><span className="space-icon"><Box size={23} aria-hidden="true" /></span><div><h3>{space.name}</h3><p>{providerName[space.provider]} · {space.node}</p></div></div><p className={'space-phase ' + space.phase} role="status">{busyPhases.includes(space.phase) && <LoaderCircle size={16} className="spaces-spinner" aria-hidden="true" />}{space.phase === 'ready' && <CircleCheck size={16} aria-hidden="true" />}{phaseName(space.phase)}</p>{space.error && <p className="direct-error">{space.error}</p>}{busyPhases.includes(space.phase) && <p className="spaces-muted">{space.phase === 'enrolling' ? t('容器已启动，正在等待工作区完成初始化并验证节点身份。', 'Container started. Waiting for workspace initialization and node identity verification.') : t('操作在后台继续，关闭页面不影响进度。', 'Work continues in the background when you close this page.')}</p>}<div className="space-card-actions">{space.phase === 'ready' ? <><button className="direct-primary" disabled={locked} onClick={() => onOpenTerminal(space.node)}><Terminal size={16} aria-hidden="true" />{t('终端', 'Terminal')}</button><button disabled={locked || !!opening || !space.ports.desktop} aria-busy={opening === space.id + 'desktop'} onClick={() => void openApplication(space, 'desktop')}>{opening === space.id + 'desktop' ? <LoaderCircle size={16} className="spaces-spinner" aria-hidden="true" /> : <Monitor size={16} aria-hidden="true" />}{t('桌面', 'Desktop')}</button><button disabled={locked || !!opening || !space.ports.code} aria-busy={opening === space.id + 'code'} onClick={() => void openApplication(space, 'code')}>{opening === space.id + 'code' ? <LoaderCircle size={16} className="spaces-spinner" aria-hidden="true" /> : <Code size={16} aria-hidden="true" />}{t('编辑器', 'Editor')}</button><button onClick={onProjects}>{t('项目', 'Projects')}</button><button disabled={locked} onClick={() => void act(space, 'stop')}><Square size={15} aria-hidden="true" />{t('停止', 'Stop')}</button></> : !busyPhases.includes(space.phase) && <button className="direct-primary" disabled={locked || imageEdit?.id === space.id} onClick={() => void act(space, 'resume')}><Play size={16} aria-hidden="true" />{space.phase === 'archived' ? t('恢复空间', 'Restore space') : t('启动 / 重试', 'Start / retry')}</button>}{!busyPhases.includes(space.phase) && space.phase !== 'archived' && <button disabled={locked} onClick={() => void act(space, 'remove')}>{t('归档', 'Archive')}</button>}{['failed', 'interrupted', 'archived'].includes(space.phase) && <button disabled={locked} aria-expanded={imageEdit?.id === space.id} aria-controls={id + '-image-' + space.id} onClick={event => { imageEditTrigger.current = event.currentTarget; setWizard(false); setError(''); setImageEdit({ id: space.id, image: space.image, registry: space.registry || null, display: space.display || 'wayland', pullPolicy: space.pull_policy || 'always', error: '' }); if (space.registry) void probeRegistries(); }}>{t('修改镜像', 'Change image')}</button>}</div>
      {imageEdit?.id === space.id && ['failed', 'interrupted', 'archived'].includes(space.phase) && <form id={id + '-image-' + space.id} className="space-image-editor spaces-step-content" aria-label={t('修改空间镜像', 'Change space image')} onSubmit={event => { event.preventDefault(); const card = event.currentTarget.closest('article'); if (card) void retryWithImage(space, card); }}>
        <h4>{t('修改镜像或显示协议并重试', 'Change image or display and retry')}</h4>
        <p id={id + '-image-help-' + space.id} className="spaces-muted">{t('使用兼容 Cranibox 的 Linux 镜像。保留此空间的身份、凭据和数据卷，不迁移其中的数据。如果容器已存在，请先归档再修改。', 'Use a compatible Cranibox Linux image. This keeps the space identity, credentials and data volume without migrating its contents. If a container exists, archive the space before changing its image.')}</p>
        <fieldset disabled={locked} aria-describedby={id + '-image-help-' + space.id}>
          <label htmlFor={id + '-image-registry-' + space.id}>{t('镜像源', 'Registry')}</label><select id={id + '-image-registry-' + space.id} value={imageEdit.registry || 'custom'} onChange={event => { const value = event.target.value; setImageEdit(current => current && { ...current, registry: value === 'custom' ? null : value as Registry }); }}>{registryIds.map(value => <option key={value} value={value}>{registryLabel(value)}</option>)}<option value="custom">{t('自定义镜像', 'Custom image')}</option></select>
          {!imageEdit.registry && <label>{t('镜像地址', 'Image reference')}<input autoFocus required maxLength={240} pattern="[A-Za-z0-9._/:@-]+" value={imageEdit.image} onChange={event => setImageEdit(current => current && { ...current, image: event.target.value })} /></label>}
          <label htmlFor={id + '-image-display-' + space.id}>{t('显示协议', 'Display server')}</label><select id={id + '-image-display-' + space.id} value={imageEdit.display} onChange={event => setImageEdit(current => current && { ...current, display: event.target.value as Display })}>{(['wayland', 'x11'] as Display[]).map(value => <option key={value} value={value}>{displayName(value)}</option>)}</select>
          <div className="space-image-policy"><label htmlFor={id + '-image-policy-' + space.id}>{t('镜像拉取策略', 'Image pull policy')}</label><select id={id + '-image-policy-' + space.id} value={imageEdit.pullPolicy} onChange={event => setImageEdit(current => current && { ...current, pullPolicy: event.target.value as PullPolicy })}>{(['always', 'missing', 'never'] as PullPolicy[]).map(policy => <option key={policy} value={policy}>{policyName(policy)}</option>)}</select></div>
        </fieldset>
        {imageEdit.error && <p ref={imageEditError} tabIndex={-1} className="direct-error" role="alert">{imageEdit.error}</p>}
        <div className="direct-actions"><button className="direct-primary" type="submit" disabled={locked}>{pending ? t('提交中…', 'Submitting…') : t('保存并重试', 'Save and retry')}</button><button type="button" disabled={locked} onClick={() => { setImageEdit(null); imageEditTrigger.current?.focus(); }}>{t('取消', 'Cancel')}</button></div>
      </form>}
      <details className="spaces-details"><summary>{t('数据与访问方式', 'Data and access')}</summary><p className="spaces-muted">{t('停止或归档保留此数据卷；恢复时继续使用相同身份和数据。', 'Stop and archive retain this volume. Restore reuses the same identity and data.')}</p><code>{space.volume}</code><p><code>{space.image}</code></p><p>{space.registry ? registryName(space.registry) + ' · ' : ''}{space.desktop ? desktopName[space.desktop] + ' · ' : ''}{displayName(space.display || 'wayland')}</p><p>{policyName(space.pull_policy || 'always')}</p>{space.ports.desktop && <p>{t('宿主机桌面', 'Desktop on engine host')} <code>https://127.0.0.1:{space.ports.desktop}</code></p>}{space.ports.code && <p>{t('宿主机编辑器', 'Editor on engine host')} <code>https://127.0.0.1:{space.ports.code}</code></p>}<p className="spaces-muted">{t('以上地址需在 Docker 引擎所在设备访问，不是当前浏览器所在设备。初次访问可能需要接受空间的自签名证书。', 'Open these addresses on the Docker engine’s device. They do not refer to a remote browser’s device. First access may require accepting the space’s self-signed certificate.')}</p><button disabled={locked} onClick={() => credentials?.id === space.id ? setCredentials(null) : void access(space)}>{credentials?.id === space.id ? t('隐藏凭据', 'Hide credentials') : t('查看登录凭据', 'View login credentials')}</button>{credentials?.id === space.id && <div className="space-credentials"><p>{t('用户名', 'Username')}: <code>{credentials.username}</code></p><label>{t('空间密码', 'Space password')}<input type="password" readOnly autoComplete="off" value={credentials.password} /></label><button onClick={async () => { try { await navigator.clipboard.writeText(credentials.password); setCopied(true); } catch { setError(t('无法复制，请选中密码手动复制。', 'Copy failed. Select the password and copy manually.')); } }}><Copy size={16} aria-hidden="true" />{copied ? t('已复制', 'Copied') : t('复制密码', 'Copy password')}</button></div>}</details></article>)}</div>
    <RemoteSpaces client={direct} language={language} phaseName={phaseName} onDevices={onDevices} />
  </section>;
}
