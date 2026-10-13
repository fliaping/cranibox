import { Download,KeyRound,Plus,Server,Terminal } from 'lucide-react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Button,Card,Empty,Facts,Filter,PageHead,Row,Status } from '../components';
import type { RemoteNode } from '../types';
import { request,t,useWorkspace } from '../workspace';
import { useTerminalWindow } from '../terminal/TerminalWindowContext';

export function Nodes() {
  const { snapshot, refresh } = useWorkspace();
  const { openTerminal } = useTerminalWindow();
  const remote = snapshot?.remote_nodes;
  const [search, setSearch] = useState(''), [id, setId] = useState('');
  const [secret, setSecret] = useState<{ id: string; token: string } | null>(null);
  const [server, setServer] = useState(() => location.hostname === 'localhost' || location.hostname === '127.0.0.1' ? 'ws://' + location.host : 'wss://' + location.host);
  const [target, setTarget] = useState(''), [program, setProgram] = useState(''), [args, setArgs] = useState('[]'), [cwd, setCwd] = useState(''), [timeout, setTimeoutSeconds] = useState('60');
  const [groups, setGroups] = useState<Record<string, string>>({});
  const [error, setError] = useState(''), [busy, setBusy] = useState(false), [job, setJob] = useState('');
  async function submit(action: string, body: unknown, complete?: (value: any) => void) {
    if (busy) return;
    setBusy(true); setError('');
    try { const value = await request('/api/nodes/' + action, body); complete?.(value); refresh(); }
    catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); }
    finally { setBusy(false); }
  }
  const enroll = (node?: RemoteNode) => {
    if (secret && !confirm(t('确认已保存上一个节点的凭据？继续将关闭其展示。', 'Have you saved the previous node credential? Continuing closes its display.'))) return;
    if (node && !confirm(t('轮换凭据会断开这个节点并停止其任务，继续？', 'Rotating credentials disconnects this node and stops its jobs. Continue?'))) return;
    void submit('enroll', { id: node?.id || id, rotate: !!node }, value => { setSecret(value); setId(''); });
  };
  function downloadToken() {
    if (!secret) return;
    const url = URL.createObjectURL(new Blob([secret.token + '\n'], { type: 'text/plain' }));
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = secret.id + '.token'; anchor.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  let serverUrl = '';
  try {
    const parsed = new URL(server);
    if (!['ws:', 'wss:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.pathname !== '/' || parsed.search || parsed.hash) throw Error('Invalid gateway origin');
    serverUrl = parsed.origin;
  } catch { /* Keep the input editable; show validation beside the command. */ }
  const nodes = (remote?.nodes || []).filter(node => [node.id, node.name, node.os, node.arch, node.mesh_group].join(' ').toLowerCase().includes(search.toLowerCase()));
  return <><PageHead title={t('远程节点', 'Remote nodes')} description={t('接入 Windows、macOS 和 Linux，让 Agent 在对应机器执行任务。', 'Connect Windows, macOS and Linux machines for Agent tasks.')} action={<Link className="button secondary" to="/history">{t('任务历史', 'Operation history')}</Link>} />
    {!remote?.available && <Card title={t('节点网关', 'Node gateway')}><p className="muted">{remote?.installed ? t('节点网关暂时无法连接，请在服务页检查网关服务。', 'The node gateway is unavailable. Check the gateway service on the Services page.') : t('内置节点网关尚未初始化，请修复网关服务。', 'The built-in node gateway has not been initialized. Repair the gateway service.')}</p>{!remote?.installed && <Button action="installRuntime" tone="primary" icon={Plus}>{t('修复节点网关', 'Repair node gateway')}</Button>}<Link className="text-link" to="/services">{t('查看服务', 'View services')}</Link></Card>}
    {error && <p className="form-error" role="alert">{error}</p>}
    {secret && <Card title={t('保存节点凭据', 'Save node credential')} action={<Button local onClick={() => setSecret(null)}>{t('关闭', 'Close')}</Button>}><p className="muted">{t('凭据仅展示这一次。下载后私下放到节点机器，限制文件访问权限。', 'This credential is shown once. Transfer it privately to the node and restrict file permissions.')}</p><label className="field"><span>{t('节点凭据', 'Node credential')}</span><input type="password" readOnly value={secret.token} autoComplete="off" /></label><div className="actions"><Button local icon={Download} onClick={downloadToken}>{t('下载 Token 文件', 'Download token file')}</Button></div><label className="field"><span>{t('节点连接地址', 'Node server URL')}</span><input value={server} onChange={event => setServer(event.target.value)} /></label><p className="muted">{t('该地址必须指向支持 WebSocket 的节点网关；远程访问使用 wss://。', 'Use the WebSocket node gateway address; remote connections require wss://.')}</p>{serverUrl ? <code className="code-block">cranibox-node --server '{serverUrl}' --id {secret.id} --token-file ./{secret.id}.token</code> : <p className="form-error" role="alert">{t('请输入 ws:// 或 wss:// 网关地址，不包含路径、凭据或查询参数。', 'Enter a ws:// or wss:// gateway origin without a path, credentials or query.')}</p>}<p className="muted">{t('Windows 使用 .\\cranibox-node.exe；macOS/Linux 使用 ./cranibox-node。', 'Use .\\cranibox-node.exe on Windows and ./cranibox-node on macOS/Linux.')}</p></Card>}
    <div className="grid two"><Card className="node-list" title={t('节点列表', 'Nodes')} action={<span className="tag">{remote?.nodes.length || 0}</span>}><p className="muted">{t('将可信设备加入同一设备组，允许各设备上的 Agent 通过客户端 MCP 相互访问；留空保持隔离。', 'Group trusted devices to let their Agents access each other through the client MCP. Leave blank to keep a device isolated.')}</p><Filter value={search} onChange={setSearch} label={t('搜索节点', 'Search nodes')} />{nodes.map(node => <Row key={node.id} icon={Server} title={node.name || node.id} detail={[node.id, node.os, node.arch, node.mesh_group && t('设备组：', 'Group: ') + node.mesh_group].filter(Boolean).join(' · ')}><div className="actions"><label className="field"><span>{t('设备组', 'Device group')}</span><input aria-label={t('设备组 ' , 'Device group ') + node.id} value={groups[node.id] ?? node.mesh_group ?? ''} maxLength={64} placeholder={t('未分组', 'Isolated')} onChange={event => setGroups(previous => ({ ...previous, [node.id]: event.target.value }))} /></label><Button local disabled={busy || (groups[node.id] ?? node.mesh_group ?? '') === (node.mesh_group ?? '')} onClick={() => { const group = groups[node.id] ?? ''; if (group && !/^[A-Za-z0-9_-]{1,64}$/.test(group)) { setError(t('设备组只能使用字母、数字、下划线或短横线。', 'Device groups use letters, numbers, underscores or hyphens.')); return; } if (confirm(t('同组设备上的 Agent 可以相互执行命令。确认修改设备组？', 'Agents on devices in the same group can run commands on each other. Change this group?'))) void submit('group', { id: node.id, group }); }}>{t('保存组', 'Save group')}</Button><Button local icon={Terminal} disabled={!node.online || !node.terminal_sessions} onClick={() => openTerminal({ node: node.id, create: true })}>{t("终端", "Terminal")}</Button><Status state={node.online ? 'connected' : 'stopped'} label={node.online ? t('在线', 'Online') : t('离线', 'Offline')} /><Button local icon={KeyRound} disabled={busy} label={t('轮换凭据 ', 'Rotate credential ') + node.id} onClick={() => enroll(node)} /><Button local tone="danger-ghost" disabled={busy} onClick={() => { if (confirm(t('撤销节点凭据并停止其任务？', 'Revoke this node credential and stop its jobs?'))) void submit('revoke', { id: node.id }); }}>{t('撤销', 'Revoke')}</Button></div></Row>)}{!nodes.length && <Empty title={search ? t('没有匹配的节点', 'No matching nodes') : t('尚未注册节点', 'No nodes registered')} />}<form className="node-form" onSubmit={event => { event.preventDefault(); enroll(); }}><label className="field"><span>{t('新节点 ID', 'New node ID')}</span><input value={id} maxLength={64} required pattern="[A-Za-z0-9_-]+" onChange={event => setId(event.target.value)} placeholder="my-laptop" /></label><Button local type="submit" icon={Plus} disabled={busy || !remote?.available}>{t('注册节点', 'Register node')}</Button></form></Card>
    <Card title={t('执行任务', 'Run a task')}><p className="muted">{t('程序和参数独立传递；需要 Shell 时显式填写对应程序。', 'Program and arguments are passed separately. Specify a shell explicitly when needed.')}</p><form className="node-form" onSubmit={event => { event.preventDefault(); setError(''); try { const values = JSON.parse(args); if (!Array.isArray(values) || !values.every(value => typeof value === 'string')) throw Error(t('参数必须是字符串 JSON 数组。', 'Arguments must be a JSON array of strings.')); void submit('exec', { id: target, request: { program, args: values, cwd: cwd || null, env: {}, timeout_secs: Number(timeout) } }, value => setJob(value.id)); } catch (problem) { setError(String(problem)); } }}>
      <label className="field"><span>{t('目标节点', 'Target node')}</span><select required value={target} onChange={event => setTarget(event.target.value)}><option value="">{t('选择在线节点', 'Choose an online node')}</option>{(remote?.nodes || []).filter(node => node.online).map(node => <option value={node.id} key={node.id}>{node.name || node.id}</option>)}</select></label>
      <label className="field"><span>{t('程序', 'Program')}</span><input required value={program} onChange={event => setProgram(event.target.value)} placeholder="git" /></label>
      <label className="field"><span>{t('参数（JSON 数组）', 'Arguments (JSON array)')}</span><textarea rows={2} required value={args} onChange={event => setArgs(event.target.value)} placeholder={'["status", "--short"]'} /></label>
      <label className="field"><span>{t('工作目录（可选）', 'Working directory (optional)')}</span><input value={cwd} onChange={event => setCwd(event.target.value)} /></label>
      <label className="field"><span>{t('超时（秒）', 'Timeout (seconds)')}</span><input type="number" min={1} max={3600} required value={timeout} onChange={event => setTimeoutSeconds(event.target.value)} /></label>
      <Button local type="submit" icon={Terminal} tone="primary" disabled={busy || !remote?.available || !target}>{busy ? t('正在提交…', 'Submitting…') : t('执行任务', 'Run task')}</Button>
    </form>{job && <p role="status"><Link className="text-link" to={'/history?kind=remote&id=' + encodeURIComponent(job)}>{t('任务已提交，查看输出', 'Task submitted. View output')}</Link></p>}<Facts items={[[t('Agent 接入', 'Agent access'), <code>workspacectl nodes --json</code>], ['MCP', <code>workspace-runtime mcp</code>]]} /></Card></div></>;
}
