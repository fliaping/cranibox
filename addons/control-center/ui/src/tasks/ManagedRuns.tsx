import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { Archive, FileText, Monitor, Play, RefreshCw, Square, Terminal } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import './managed-runs.css';
import { ask } from '../confirm';

type Launch = { name: string; harness: string; program: string; args: string[]; cwd: string; stdin?: string };
type Adapter = 'codex' | 'claude_code';
type Access = 'read_only' | 'workspace_write' | 'edit_files';
type Prepared = { launch: Launch; adapter?: Adapter; access?: Access; available?: boolean };
export type ExecutionCommand = { operation: 'list' } | { operation: 'prepare_agent'; agent: { name: string; adapter: Adapter; access: Access; prompt: string; cwd: string } } | { operation: 'start'; id: string; launch: Launch } | { operation: 'get'; id: string; stdout_offset?: number; stderr_offset?: number } | { operation: 'stop' | 'archive'; id: string };
type State = 'queued' | 'starting' | 'running' | 'stopping' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted';
type Run = { id: string; session_id: string; name: string; harness: string; state: State; created_at: number; updated_at: number; pid: number | null; exit_code: number | null; error: string | null; stdout_truncated: boolean; stderr_truncated: boolean };
type Catalog = { owner: string; executions: Run[]; capacity: number; used: number; parallel: number; log_limit: number };
type Output = { text: string; next_offset: number };
type Detail = { execution: Run; launch: Launch; stdout: Output; stderr: Output };
const active = (state: State) => ['queued', 'starting', 'running', 'stopping'].includes(state);
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
function identifier() {
  const bytes = crypto.getRandomValues(new Uint8Array(16)); bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
  const hex = [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
  return [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join('-');
}

/** Changing the target remounts its observer, so late responses cannot cross devices. */
export function ManagedRuns({ client, language, paused, devices }: { client: DirectClient; language: string; paused: boolean; devices?: DirectCatalog }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [open, setOpen] = useState(false), [node, setNode] = useState(''), [locked, setLocked] = useState(false);
  const available = !node || devices?.routes.some(route => route.node === node);
  return <details className="managed-runs" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary><Terminal size={18} aria-hidden="true" />{t('设备后台执行', 'Device background runs')}<span>{t('关闭页面后继续运行', 'Continue after closing the page')}</span></summary>
    {open && <>
      <div className="managed-run-device">
        <Monitor size={20} aria-hidden="true" />
        <label>{t('后台执行设备', 'Background execution device')}<select value={node} disabled={locked || !devices} onChange={event => setNode(event.target.value)} aria-describedby="managed-run-device-help">
          <option value="">{t('本机', 'This device')}{devices?.node_id ? ` · ${devices.node_id}` : ''}</option>
          {devices?.routes.filter(route => route.node !== devices.node_id).map(route => <option key={route.node} value={route.node}>{route.node}</option>)}
          {!available && <option value={node}>{node} · {t('连接已移除', 'Connection removed')}</option>}
        </select></label>
        <p id="managed-run-device-help" className="direct-note">{node ? t('目标需授予执行权限。仅显示本机在当前授权下提交的运行；切换设备会清除未提交的表单。', 'Execution access is required. Only runs submitted by this device under its current grant are shown. Switching devices clears the unsubmitted form.') : t('Web 上的本机是运行 Workspace 服务的设备。选择其他设备可管理其授权的后台运行。', 'On the Web, this device runs the Workspace service. Select another device to manage authorized background runs.')}</p>
      </div>
      {available ? <DeviceRuns key={node || 'local'} client={client} language={language} paused={paused} node={node || undefined} onLock={setLocked} /> : <p className="direct-error" role="alert">{t('设备连接已移除。此页面不会因此停止已经启动的进程。', 'The device connection was removed. This does not stop processes already started.')}</p>}
    </>}
  </details>;
}

function DeviceRuns({ client, language, paused, node, onLock }: { client: DirectClient; language: string; paused: boolean; node?: string; onLock: (busy: boolean) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [catalog, setCatalog] = useState<Catalog>();
  const [name, setName] = useState(''), [harness, setHarness] = useState('custom-cli'), [program, setProgram] = useState(''), [cwd, setCwd] = useState(''), [args, setArgs] = useState('[]');
  const [adapter, setAdapter] = useState<Adapter | 'custom'>('codex'), [access, setAccess] = useState<Access>('read_only'), [prompt, setPrompt] = useState(''), [launchError, setLaunchError] = useState('');
  const [review, setReview] = useState<Prepared>(), [detail, setDetail] = useState<Detail>();
  const [busy, setBusy] = useState(false), [reading, setReading] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('');
  const alive = useRef(true), mutation = useRef(false), fetching = useRef(false), serial = useRef(0), selected = useRef<Detail | undefined>(undefined), draft = useRef<{ key: string; id: string } | undefined>(undefined);
  useEffect(() => { onLock(busy); return () => onLock(false); }, [busy, onLock]);
  useEffect(() => { alive.current = true; return () => { alive.current = false; serial.current++; }; }, []);
  const read = useCallback(async () => {
    if (fetching.current || mutation.current || document.hidden) return;
    fetching.current = true; const request = ++serial.current; setReading(true);
    try {
      const value = await client({ operation: 'executions', ...(node ? { node } : {}), command: { operation: 'list' } }) as Catalog;
      if (!value || !Array.isArray(value.executions)) throw new Error('Invalid execution catalog. Update the device service.');
      if (node && value.owner !== node) throw new Error('Execution device identity mismatch. Reconnect the intended device.');
      if (!alive.current || request !== serial.current) return;
      setCatalog(value); setError('');
      const previous = selected.current;
      if (previous) {
        const current = await client({ operation: 'executions', ...(node ? { node } : {}), command: { operation: 'get', id: previous.execution.id, stdout_offset: previous.stdout.next_offset, stderr_offset: previous.stderr.next_offset } }) as Detail;
        if (alive.current && request === serial.current && selected.current === previous) {
          const updated = { ...current, stdout: { ...current.stdout, text: previous.stdout.text + current.stdout.text }, stderr: { ...current.stderr, text: previous.stderr.text + current.stderr.text } };
          selected.current = updated; setDetail(updated);
        }
      }
    } catch (error) { if (alive.current && request === serial.current) { setError(errorText(error)); if (node) { setCatalog(undefined); selected.current = undefined; setDetail(undefined); } } }
    finally { fetching.current = false; if (alive.current) setReading(false); }
  }, [client, node]);
  useEffect(() => {
    void read(); const timer = !paused ? setInterval(() => void read(), 3000) : undefined;
    const visible = () => { if (!paused) void read(); };
    document.addEventListener('visibilitychange', visible);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', visible); };
  }, [read, paused]);
  async function inspect(id: string) {
    if (fetching.current || mutation.current) return;
    fetching.current = true;
    const request = ++serial.current; setReading(true); setError('');
    try {
      const value = await client({ operation: 'executions', ...(node ? { node } : {}), command: { operation: 'get', id } }) as Detail;
      if (alive.current && request === serial.current) { selected.current = value; setDetail(value); }
    } catch (error) { if (alive.current && request === serial.current) setError(errorText(error)); }
    finally { fetching.current = false; if (alive.current) setReading(false); }
  }
  async function run(command: ExecutionCommand) {
    if (mutation.current) return;
    serial.current++; mutation.current = true; setBusy(true); setError(''); setLaunchError(''); setNotice('');
    try {
      const value = await client({ operation: 'executions', ...(node ? { node } : {}), command }) as Catalog;
      if (!value || !Array.isArray(value.executions)) throw new Error('Invalid execution catalog. Update the device service.');
      if (node && value.owner !== node) throw new Error('Execution device identity mismatch. Reconnect the intended device.');
      if (!alive.current) return;
      setCatalog(value);
      if (command.operation === 'start') { setReview(undefined); draft.current = undefined; setNotice(t('已保存启动请求。节点服务会更新实际进程状态。', 'Launch request saved. The node service will report the actual process state.')); }
      if (command.operation === 'stop') setNotice(t('已请求停止，请等待进程退出状态。', 'Stop requested. Wait for the process exit state.'));
      if (command.operation === 'archive') { if (selected.current?.execution.id === command.id) { selected.current = undefined; setDetail(undefined); } setNotice(t('已归档并清除输出，运行 ID 保留用于去重。', 'Archived and output removed. The run ID remains for deduplication.')); }
    } catch (error) { if (alive.current) { if (command.operation === 'start') setLaunchError(errorText(error)); else setError(errorText(error)); } }
    finally { mutation.current = false; if (alive.current) setBusy(false); }
  }
  async function prepare(event: FormEvent) {
    event.preventDefault();
    if (mutation.current) return;
    serial.current++; mutation.current = true; setBusy(true); setError(''); setLaunchError(''); setNotice('');
    try {
      if (adapter === 'custom') {
        const parsed: unknown = JSON.parse(args);
        if (!Array.isArray(parsed) || !parsed.every(arg => typeof arg === 'string') || parsed.length > 128 || new TextEncoder().encode(parsed.join('')).length > 8192) throw new Error(t('参数需为 JSON 字符串数组，最多 128 项、8192 字节。', 'Arguments must be a JSON string array, up to 128 items and 8192 bytes.'));
        setReview({ launch: { name: name.trim(), harness: harness.trim(), program: program.trim(), cwd: cwd.trim(), args: parsed } });
      } else {
        if (new TextEncoder().encode(prompt).length > 16384) throw new Error(t('任务说明最多 16 KiB，请缩短说明或引用项目文件。', 'Task instructions are limited to 16 KiB. Shorten the instructions or refer to a project file.'));
        const value = await client({ operation: 'executions', ...(node ? { node } : {}), command: { operation: 'prepare_agent', agent: { name: name.trim(), adapter, access, prompt, cwd: cwd.trim() } } }) as Prepared;
        if (!value?.launch || typeof value.available !== 'boolean') throw new Error(t('节点尚不支持 Agent 启动，请更新节点服务。', 'Update the node service to enable Agent launches.'));
        if (alive.current) setReview(value);
      }
    } catch (error) { if (alive.current) setLaunchError(errorText(error)); }
    finally { mutation.current = false; if (alive.current) setBusy(false); }
  }
  function start() {
    if (!review || review.available === false) return;
    const key = JSON.stringify(review.launch);
    if (draft.current?.key !== key) draft.current = { key, id: identifier() };
    void run({ operation: 'start', id: draft.current.id, launch: review.launch });
  }
  const accessLabel = (value: Access) => ({ read_only: t('只读', 'Read only'), workspace_write: t('修改工作区', 'Write to workspace'), edit_files: t('读写文件工具', 'File editing tools') })[value];
  const accessHelp = (agent: Adapter, value: Access) => agent === 'codex'
    ? (value === 'read_only' ? t('使用 Codex 只读沙箱；无法交互确认的操作会被拒绝。', 'Uses the Codex read-only sandbox. Operations requiring interactive approval are denied.') : t('允许在 Codex 工作区沙箱内执行命令和修改文件；不启用跳过沙箱选项。', 'Allows commands and file changes within the Codex workspace sandbox. Sandbox bypass is disabled.'))
    : t('Claude 仅开放读取、查找工具；选择读写后增加 Edit、Write。不开放 Shell 或 MCP 工具。这是工具限制，不是系统沙箱，文件工具仍使用节点账户权限。', 'Claude exposes Read, Glob and Grep, adding Edit and Write for file editing. Shell and MCP tools are disabled. This is a tool restriction, not an OS sandbox; file tools use the node account’s permissions.');
  const status = (state: State) => ({ queued: t('等待节点启动', 'Awaiting node launch'), starting: t('正在启动', 'Starting'), running: t('进程运行中', 'Process running'), stopping: t('正在停止', 'Stopping'), succeeded: t('退出码 0', 'Exited with code 0'), failed: t('运行失败', 'Execution failed'), cancelled: t('已停止', 'Stopped'), interrupted: t('已中断，需检查', 'Interrupted · review needed') })[state];
  return <div className="managed-runs-content">
      <p className="direct-note">{t('用于支持非交互运行的 Agent CLI 或程序。节点持有进程、输出和退出结果；节点退出会结束运行，重启后保留中断记录。退出码不代表协作任务已完成，仍需执行方明确报告。', 'For Agent CLIs or programs that support noninteractive execution. The node owns processes, output and exit results. Stopping the node ends execution; restart retains interrupted records. An exit code does not complete a collaboration task; an explicit worker report is still required.')}</p>
      <div className="direct-actions"><button disabled={reading || busy} onClick={() => void read()}><RefreshCw size={16} aria-hidden="true" />{t('刷新后台执行', 'Refresh background runs')}</button>{catalog && <small>{catalog.owner} · {catalog.executions.filter(run => active(run.state)).length} / {catalog.parallel} {t('运行', 'active')}</small>}</div>
      {error && <p className="direct-error" role="alert">{error}{catalog && t(' · 当前显示上次记录。', ' · Showing the previous records.')}</p>}{notice && <p role="status">{notice}</p>}
      {!catalog && !error && <p role="status">{t('正在读取运行记录…', 'Reading execution records…')}</p>}
      {catalog && <div className="managed-run-list">{!catalog.executions.length && <p>{t('还没有后台运行。在下方检查并启动此设备上的 Agent 或程序。', 'No background runs yet. Review and launch an Agent or program on this device below.')}</p>}{[...catalog.executions].sort((a, b) => b.created_at - a.created_at).map(item => <article className="managed-run-card" key={item.id} aria-label={t('后台运行 ', 'Background run ') + item.name}>
        <div className="managed-run-heading"><div><strong>{item.name}</strong><small>{item.harness}</small></div><span>{status(item.state)}</span></div>
        <code>{item.id}</code><small>{t('Agent 会话：', 'Agent session: ')}{item.session_id}</small>
        {item.error && <p className="direct-error">{item.error}</p>}
        {item.exit_code !== null && <p>{t('进程退出码：', 'Process exit code: ')}{item.exit_code}</p>}
        <div className="direct-actions"><button disabled={busy || reading} onClick={() => void inspect(item.id)}><FileText size={16} aria-hidden="true" />{t('查看输出', 'View output')}</button>{active(item.state) ? <button disabled={busy || !!error || item.state === 'stopping'} onClick={async () => { if (await ask(t('停止此运行及其子进程？未保存的工作可能丢失。', 'Stop this run and its child processes? Unsaved work may be lost.'))) void run({ operation: 'stop', id: item.id }); }}><Square size={16} aria-hidden="true" />{t('停止运行', 'Stop run')}</button> : <button disabled={busy || !!error} onClick={async () => { if (await ask(t('归档此记录并删除保存的输出？', 'Archive this record and delete its saved output?'))) void run({ operation: 'archive', id: item.id }); }}><Archive size={16} aria-hidden="true" />{t('归档运行', 'Archive run')}</button>}</div>
      </article>)}</div>}
      {detail && <section className="managed-run-output" aria-label={t('运行输出', 'Execution output')}><div className="direct-actions"><h3>{detail.execution.name}</h3><button onClick={() => { serial.current++; selected.current = undefined; setDetail(undefined); }}>{t('关闭输出', 'Close output')}</button></div><p>{status(detail.execution.state)}</p><dl><dt>{t('程序', 'Program')}</dt><dd><code>{detail.launch.program}</code></dd><dt>{t('工作目录', 'Working directory')}</dt><dd><code>{detail.launch.cwd}</code></dd></dl>{(['stdout', 'stderr'] as const).map(stream => <div key={stream}><h4>{stream}</h4><pre tabIndex={0} aria-label={stream}>{detail[stream].text || t('暂无输出', 'No output yet')}</pre></div>)}{(detail.execution.stdout_truncated || detail.execution.stderr_truncated) && <p className="direct-note">{t('输出已达到每条流 1 MiB 上限，后续内容未保存。程序仍可继续运行。', 'Output reached the 1 MiB limit per stream. Further content is not saved; the program can continue running.')}</p>}</section>}
      <details className="managed-run-launch"><summary><Play size={16} aria-hidden="true" />{t('启动 Agent 或程序', 'Launch an Agent or program')}</summary>
        {review ? <div className="managed-run-review">
          <div><h3>{t('检查启动内容', 'Review launch')}</h3><p className="direct-note">{t('确认后才会在以下设备执行。', 'Execution begins on the following device only after confirmation.')}</p></div>
          <dl className="managed-run-facts">
            <div><dt>{t('执行设备', 'Execution device')}</dt><dd><code>{catalog?.owner}</code></dd></div>
            <div><dt>{t('运行名称', 'Run name')}</dt><dd>{review.launch.name}</dd></div>
            <div><dt>{t('Agent / 程序', 'Agent / program')}</dt><dd>{review.adapter === 'codex' ? 'Codex' : review.adapter === 'claude_code' ? 'Claude Code' : review.launch.harness}</dd></div>
            <div><dt>{t('工作目录', 'Working directory')}</dt><dd><code>{review.launch.cwd}</code></dd></div>
            {review.access && <div><dt>{t('执行权限', 'Execution access')}</dt><dd>{accessLabel(review.access)}</dd></div>}
          </dl>
          {review.launch.stdin !== undefined && <div><h4>{t('任务说明', 'Task instructions')}</h4><pre className="managed-run-prompt">{review.launch.stdin}</pre></div>}
          {review.adapter && review.access && <p className="direct-note">{accessHelp(review.adapter, review.access)}</p>}
          <details className="managed-run-command"><summary>{t('查看启动命令', 'View launch command')}</summary><dl><dt>{t('可执行程序', 'Executable')}</dt><dd><code>{review.launch.program}</code></dd><dt>{t('参数', 'Arguments')}</dt><dd><code>{JSON.stringify(review.launch.args)}</code></dd></dl></details>
          {review.available === false && <p className="direct-error" role="alert">{t('此设备的 PATH 中未找到该 CLI。请在节点账户下安装并登录，再返回编辑重新检查；也可选择自定义程序填写完整路径。', 'This CLI was not found on the device PATH. Install it and sign in with the node account, then return to edit and review again. You can also select Custom program and enter its full path.')}</p>}
          <p className="direct-note">{review.adapter ? t('使用节点账户已有的环境及 CLI 登录配置。任务与输出保存在此设备，任务文本不会放入进程参数。', 'Uses the node account’s environment and CLI login configuration. Instructions and output are stored on this device; instructions are not placed in process arguments.') : t('使用节点账户的环境与权限，启动内容及输出保存在此设备。请勿在参数中填写密钥。', 'Uses the node account’s environment and permissions. Launch details and output are stored on this device. Keep secrets out of arguments.')}</p>
          {launchError && <p className="direct-error" role="alert">{launchError}{draft.current && <><br />{t('请求 ID：', 'Request ID: ')}<code>{draft.current.id}</code><br />{t('请求可能已保存。请刷新记录确认，或使用相同内容再次确认；不会为此次重试创建新的运行 ID。', 'The request may already be saved. Refresh the records to check, or confirm the same launch again; this retry retains the run ID.')}</>}</p>}
          <div className="direct-actions"><button className="direct-primary" disabled={busy || !catalog || review.available === false} onClick={start}><Play size={16} aria-hidden="true" />{busy ? t('正在保存…', 'Saving…') : t('确认启动', 'Confirm launch')}</button><button disabled={busy} onClick={() => { setReview(undefined); setError(''); setLaunchError(''); }}>{t('返回编辑', 'Back to edit')}</button></div>
        </div> : <form className="direct-editor managed-run-form" onSubmit={prepare} aria-label={t('启动 Agent 或程序', 'Launch an Agent or program')} aria-busy={busy}>
          <div className="managed-run-fields">
            <label>{t('Agent / 程序', 'Agent / program')}<select value={adapter} disabled={busy} onChange={event => { setAdapter(event.target.value as Adapter | 'custom'); setAccess('read_only'); setLaunchError(''); }}><option value="codex">Codex</option><option value="claude_code">Claude Code</option><option value="custom">{t('自定义程序', 'Custom program')}</option></select></label>
            <label>{t('运行名称', 'Run name')}<input required maxLength={120} value={name} disabled={busy} onChange={event => setName(event.target.value)} /></label>
          </div>
          <label>{t('完整工作目录', 'Absolute working directory')}<input required value={cwd} disabled={busy} placeholder={t('节点上的项目绝对路径', 'Absolute project path on the node')} onChange={event => setCwd(event.target.value)} /></label>
          <p className="direct-note">{t('执行设备：', 'Execution device: ')}<code>{catalog?.owner || node || '…'}</code>{node ? t('。目录和 CLI 均使用目标设备上的配置。', '. Directories and CLI configuration belong to the target device.') : t('。Web 页面操作的是服务所在设备。', '. On the Web, this is the device running the service.')}</p>
          {adapter === 'custom' ? <>
            <label>{t('Agent 类型', 'Agent type')}<input required value={harness} disabled={busy} onChange={event => setHarness(event.target.value)} /></label>
            <label>{t('可执行程序', 'Executable')}<input required value={program} disabled={busy} onChange={event => setProgram(event.target.value)} /></label>
            <label>{t('参数（JSON 字符串数组）', 'Arguments (JSON string array)')}<textarea rows={3} required value={args} disabled={busy} onChange={event => setArgs(event.target.value)} /></label>
            <p className="direct-note">{t('程序须支持非交互运行，使用节点账户权限。请勿在参数中填写密钥。', 'The program must support noninteractive execution and uses the node account’s permissions. Keep secrets out of arguments.')}</p>
          </> : <>
            <label>{t('任务说明', 'Task instructions')}<textarea rows={5} required value={prompt} disabled={busy} aria-describedby="managed-run-prompt-limit" placeholder={t('描述希望 Agent 完成的工作、约束和验收方式…', 'Describe the work, constraints and how to verify the result…')} onChange={event => setPrompt(event.target.value)} /></label>
            <small id="managed-run-prompt-limit" className="direct-note">{new TextEncoder().encode(prompt).length.toLocaleString()} / 16,384 {t('字节 · 通过标准输入传递，请勿填写密钥', 'bytes · sent through stdin; do not include secrets')}</small>
            <label>{t('执行权限', 'Execution access')}<select value={access} disabled={busy} aria-describedby="managed-run-access-help" onChange={event => setAccess(event.target.value as Access)}><option value="read_only">{accessLabel('read_only')}</option>{adapter === 'codex' ? <option value="workspace_write">{accessLabel('workspace_write')}</option> : <option value="edit_files">{accessLabel('edit_files')}</option>}</select></label>
            <p id="managed-run-access-help" className="direct-note">{accessHelp(adapter, access)}</p>
            <p className="direct-note">{t('请先在此设备安装并登录所选 CLI。检查步骤不会启动 Agent。', 'Install and sign in to the selected CLI on this device first. Reviewing does not start an Agent.')}</p>
          </>}
          {launchError && <p className="direct-error" role="alert">{launchError}</p>}
          <button type="submit" disabled={busy || !name.trim() || !cwd.trim() || (adapter === 'custom' ? !harness.trim() || !program.trim() : !prompt.trim())}>{busy ? t('正在检查…', 'Checking…') : t('检查启动内容', 'Review launch')}</button>
        </form>}
      </details>
    </div>;
}
