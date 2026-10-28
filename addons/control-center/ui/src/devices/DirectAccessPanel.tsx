import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { BadgeCheck, Copy, KeyRound, Plus, ShieldAlert, ShieldCheck, Trash2, X } from 'lucide-react';
import type { DirectCatalog, DirectClient } from './DirectDevicesView';
import { DirectListenerPanel } from './DirectListenerPanel';
import { BrowserSharingPanel } from './BrowserSharingPanel';
import { ComputerSharingPanel } from './ComputerSharingPanel';
import { ProjectCopyRootPanel } from './ProjectCopyRootPanel';
import { ask } from '../confirm';

type Issued = { node: string; token: string };
export function DirectAccessPanel({ client, catalog, language, busy, run, onCatalog }: {
  client: DirectClient; catalog: DirectCatalog; language: string; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>; onCatalog: (catalog: DirectCatalog) => void;
}) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [editor, setEditor] = useState(false), [node, setNode] = useState('');
  const [exec, setExec] = useState(false), [terminals, setTerminals] = useState(false);
  const [messages, setMessages] = useState(false), [spaces, setSpaces] = useState(false);
  const [activity, setActivity] = useState(false);
  const [computer, setComputer] = useState(false), [browser, setBrowser] = useState(false);
  const [projects, setProjects] = useState(false), [desktop, setDesktop] = useState(false);
  const [allowUnverified, setAllowUnverified] = useState(false), [codes, setCodes] = useState<Record<string, string>>({}), [verifyError, setVerifyError] = useState<Record<string, string>>({});
  const [issued, setIssued] = useState<Issued>(), [copied, setCopied] = useState(false), [copyError, setCopyError] = useState('');
  const dialog = useRef<HTMLDialogElement>(null), tokenField = useRef<HTMLTextAreaElement>(null), firstField = useRef<HTMLInputElement>(null);
  const trigger = useRef<HTMLButtonElement>(null), mounted = useRef(true), issuance = useRef(0), titleId = useId(), tokenId = useId();
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; issuance.current++; }; }, []);
  useEffect(() => { if (editor) firstField.current?.focus(); }, [editor]);
  useEffect(() => {
    if (issued) dialog.current?.showModal();
    else dialog.current?.close();
  }, [issued]);
  function dismiss() {
    issuance.current++; setIssued(undefined); setCopied(false); setCopyError('');
    requestAnimationFrame(() => trigger.current?.focus());
  }
  async function grant(event: FormEvent) {
    event.preventDefault();
    await run(async () => {
      const value = await client({ operation: 'grant', node: node.trim(), require_verified: !allowUnverified, permissions: { exec, terminals, projects, desktop, computer, browser, messages, activity, spaces } }) as Issued & { catalog: DirectCatalog };
      if (!mounted.current) return;
      onCatalog(value.catalog); setEditor(false); setNode(''); setExec(false); setTerminals(false); setProjects(false); setDesktop(false); setComputer(false); setBrowser(false); setMessages(false); setActivity(false); setSpaces(false); setAllowUnverified(false);
      setCopied(false); setCopyError(''); issuance.current++; setIssued({ node: value.node, token: value.token });
    });
  }
  async function copy() {
    if (!issued) return;
    const current = issuance.current;
    try {
      await navigator.clipboard.writeText(issued.token);
      if (mounted.current && current === issuance.current) { setCopied(true); setCopyError(''); }
    } catch {
      if (mounted.current && current === issuance.current) {
        tokenField.current?.focus(); tokenField.current?.select();
        setCopyError(t('无法自动复制。Token 已选中，请手动复制。', 'Automatic copy unavailable. The token is selected for manual copying.'));
      }
    }
  }
  type GrantView = NonNullable<DirectCatalog['grants']>[number];
  function verify(event: FormEvent, grant: GrantView) {
    event.preventDefault();
    const code = (codes[grant.node] || '').trim();
    void run(async () => {
      try {
        const value = await client({ operation: 'verify', node: grant.node, code }) as { catalog: DirectCatalog };
        if (!mounted.current) return;
        setCodes(current => ({ ...current, [grant.node]: '' })); setVerifyError(current => ({ ...current, [grant.node]: '' })); onCatalog(value.catalog);
      } catch (error) {
        if (mounted.current) setVerifyError(current => ({ ...current, [grant.node]: error instanceof Error ? error.message : String(error) }));
      }
    });
  }
  // Pairing state: a key pinned on first use is not verified until the owner enters the
  // code shown on the other device. Strict grants refuse everything until then.
  function pairing(grant: GrantView) {
    if (grant.peer?.verified) return <p className="direct-pairing direct-pairing-verified"><BadgeCheck size={15} aria-hidden="true" />{t('配对码已核对', 'Pairing code verified')}</p>;
    if (!grant.peer) return <p className={'direct-pairing' + (grant.require_verified ? ' direct-pairing-pending' : '')}>{grant.require_verified ? <><ShieldAlert size={15} aria-hidden="true" />{t('等待对方首次连接，之后核对配对码', 'Waiting for the first connection, then verify the pairing code')}</> : t('未要求核对配对码（兼容模式）', 'Pairing verification not required (compatibility mode)')}</p>;
    return <form className="direct-pairing direct-pairing-pending" onSubmit={event => verify(event, grant)} aria-label={t('核对配对码 ', 'Verify pairing code ') + grant.node}>
      <span><ShieldAlert size={15} aria-hidden="true" />{grant.require_verified ? t('未核对：所有能力已暂停', 'Not verified: all capabilities are paused') : t('未核对（兼容模式仍允许访问）', 'Not verified (compatibility mode still allows access)')}</span>
      <span>{t('本机显示的配对码', 'Code shown here')} <code>{grant.peer.pairing_code || '—'}</code></span>
      <label>{t('输入对方设备显示的配对码', 'Enter the code shown on the other device')}<input inputMode="numeric" autoComplete="off" pattern="[0-9 -]{12,16}" required value={codes[grant.node] || ''} disabled={busy} onChange={event => setCodes(current => ({ ...current, [grant.node]: event.target.value }))} /></label>
      <button type="submit" disabled={busy}><BadgeCheck size={16} aria-hidden="true" />{t('核对并信任', 'Verify and trust')}</button>
      {verifyError[grant.node] && <span className="direct-error" role="alert">{verifyError[grant.node]}</span>}
    </form>;
  }
  return <section className="direct-access" aria-label={t('入站授权', 'Incoming access')}>
    <div className="direct-heading"><div><h3><ShieldCheck size={18} aria-hidden="true" />{t('入站授权', 'Incoming access')}</h3><p>{t('决定哪些设备可以访问此接入端。出站连接和入站授权分别管理。', 'Choose which devices may access this endpoint. Incoming grants are managed separately from outbound connections.')}</p></div><button ref={trigger} disabled={busy} onClick={() => { setEditor(value => !value); setNode(''); setExec(false); setTerminals(false); setProjects(false); setDesktop(false); setComputer(false); setBrowser(false); setMessages(false); setActivity(false); setSpaces(false); }}>{editor ? <X size={16} aria-hidden="true" /> : <Plus size={16} aria-hidden="true" />}{editor ? t('收起授权表单', 'Close grant form') : t('授权设备', 'Grant device access')}</button></div>
    <DirectListenerPanel client={client} catalog={catalog} language={language} busy={busy} run={run} onCatalog={onCatalog} />
    <ProjectCopyRootPanel client={client} catalog={catalog} language={language} busy={busy} run={run} onCatalog={onCatalog} />
    <ComputerSharingPanel client={client} catalog={catalog} language={language} busy={busy} run={run} onCatalog={onCatalog} />
    <BrowserSharingPanel client={client} catalog={catalog} language={language} busy={busy} run={run} onCatalog={onCatalog} />
    {editor && <form className="direct-grant-form" onSubmit={grant}>
      <label>{t('被授权设备 ID', 'Caller node ID')}<input ref={firstField} required pattern="[A-Za-z0-9_-]{1,64}" maxLength={64} value={node} disabled={busy} onChange={event => setNode(event.target.value)} /><span className="direct-note">{t('填写对方“当前接入端的直连身份”，不是 Workspace 中继别名。', 'Use the other device’s direct identity, rather than its Workspace relay alias.')}</span></label>
      <fieldset disabled={busy}><legend>{t('允许的能力', 'Allowed capabilities')}</legend><label className="direct-check"><input type="checkbox" checked={exec} onChange={event => setExec(event.target.checked)} />{t('执行命令', 'Execute commands')}</label><label className="direct-check"><input type="checkbox" checked={terminals} onChange={event => setTerminals(event.target.checked)} />{t('管理和接入终端会话', 'Manage and attach terminal sessions')}</label><p className="direct-note">{t('这些能力以节点进程的系统账户运行。终端权限可执行任意 Shell 命令，并访问该账户的所有终端会话。', 'These capabilities run as the node’s OS account. Terminal access permits arbitrary shell commands and access to all terminal sessions owned by that account.')}</p></fieldset>
      <label className="direct-check"><input type="checkbox" checked={projects} disabled={busy} onChange={event => setProjects(event.target.checked)} />{t('读取和管理此设备的项目目录', 'Read and manage this device’s project catalog')}</label><p className="direct-note">{t('可查看全部项目路径、会话关联并修改目录记录；不会授予文件读写或 Shell 权限。', 'Allows all project paths, session links and catalog changes on this device. File access and shell execution require separate capabilities.')}</p>
      <label className="direct-check"><input type="checkbox" checked={spaces} disabled={busy} onChange={event => setSpaces(event.target.checked)} />{t('查看此设备管理的虚拟空间目录', 'View virtual spaces managed by this device')}</label><p className="direct-note">{t('只共享空间名称、节点 ID、运行环境和管理目录中的状态。启动、停止、登录凭据和空间节点访问仍由各自的授权控制。', 'Shares space names, node IDs, runtime and recorded status. Starting, stopping, login credentials and access to the space’s node remain separately controlled.')}</p>
      <label className="direct-check"><input type="checkbox" checked={desktop} disabled={busy} onChange={event => setDesktop(event.target.checked)} />{t('查看和控制此设备的桌面', 'View and control this device’s desktop')}</label><p className="direct-note">{t('可读取完整屏幕并发送键盘、鼠标输入。需要在“远程桌面”中开启本机 VNC 共享。', 'Allows screen capture, keyboard and mouse input. Enable local VNC sharing in Remote desktop first.')}</p>
      <label className="direct-check"><input type="checkbox" checked={computer} disabled={busy} onChange={event => setComputer(event.target.checked)} />{t('允许 Agent 桌面操作', 'Allow Agent Computer Use')}</label><p className="direct-note">{t('可截图、读取窗口及发送键鼠输入。需先配置上方的本机桥接；输入必须取得短期控制权。此授权独立于桌面查看器。', 'Allows screenshots, window inspection and keyboard/mouse input through the local bridge above. Input requires a short control lease. This grant is separate from desktop viewer access.')}</p>
      <label className="direct-check"><input type="checkbox" checked={browser} disabled={busy} onChange={event => setBrowser(event.target.checked)} />{t('允许 Agent 浏览器操作', 'Allow Agent Browser Use')}</label><p className="direct-note">{t('可读取和操作共享浏览器的所有标签页、登录会话和网页数据，并执行页面脚本。需单独配置本机浏览器；不授予 Shell 或桌面权限。', 'Allows all tabs, signed-in sessions, page data and page scripts in the shared browser. Configure the local browser separately. This grants no shell or desktop access.')}</p>
      <label className="direct-check"><input type="checkbox" checked={messages} disabled={busy} onChange={event => setMessages(event.target.checked)} />{t('设备消息与协作任务', 'Device messages and collaboration tasks')}</label><p className="direct-note">{t('可查看本机 Agent 公开登记的名称、会话 ID 和状态，投递纯文本消息和任务请求，并查询自己投递的回执、进度与结果。目标 Agent 需主动领取任务；此权限不授予命令执行能力，也不能读取其他发送方的数据。', 'Allows reading public Agent session labels, IDs and reported status, sending plain text messages and tasks, and checking your own submissions. A target Agent must explicitly claim tasks. This grants no command execution or access to other senders’ data.')}</p>
      <label className="direct-check"><input type="checkbox" checked={activity} disabled={busy} onChange={event => setActivity(event.target.checked)} />{t('只读协作记录（包含其他发送方）', 'Read collaboration records, including other senders')}</label><p className="direct-note">{t('可读取此设备未归档的任务说明、进度、问题和回复、结果、消息正文及协作 Run。仅用于统一视图；不会授予投递、修改或命令执行能力。', 'Shares this device’s unarchived task descriptions, progress, questions and answers, results, message bodies and collaboration Runs. Read access only; delivery, changes and command execution require separate grants.')}</p>
      <label className="direct-check"><input type="checkbox" checked={allowUnverified} disabled={busy} onChange={event => setAllowUnverified(event.target.checked)} />{t('允许在核对配对码之前使用（不推荐）', 'Allow before the pairing code is verified (not recommended)')}</label><p className="direct-note">{t('默认情况下，对方首次连接后需在下方核对两台设备显示的 12 位配对码，核对前拒绝所有能力与明文请求。仅为旧版节点或网关勾选此项。', 'By default, after the device first connects you compare the 12-digit pairing code both devices show (below); until then every capability and plaintext request is refused. Check this only for older nodes or gateways.')}</p>
      <button className="direct-primary" type="submit" disabled={busy || !exec && !terminals && !projects && !desktop && !computer && !browser && !messages && !activity && !spaces}><KeyRound size={16} aria-hidden="true" />{busy ? t('正在创建…', 'Creating…') : t('创建授权 Token', 'Create access token')}</button>
    </form>}
    <div className="direct-grant-list" aria-label={t('已授权设备', 'Granted devices')}>
      {(catalog.grants || []).map(grant => <div key={grant.node} className="direct-grant-row"><div><strong>{grant.node}</strong><p className="direct-note">{[grant.permissions.exec && t('执行命令', 'Execute commands'), grant.permissions.terminals && t('终端会话', 'Terminal sessions'), grant.permissions.activity && t('只读协作记录', 'Read collaboration records'), grant.permissions.spaces && t('空间目录', 'Space catalog'), grant.permissions.projects && t('项目目录', 'Project catalog'), grant.permissions.messages && t('设备消息与协作任务', 'Device messages and collaboration tasks'), grant.permissions.browser && t('浏览器操作', 'Browser Use'), grant.permissions.desktop && t('桌面控制', 'Desktop control'), grant.permissions.computer && t('Agent 桌面操作', 'Agent Computer Use')].filter(Boolean).join(' · ')}</p>{pairing(grant)}</div><button disabled={busy} aria-label={t('撤销授权 ', 'Revoke access ') + grant.node} onClick={async () => {
        if (await ask(t('撤销该设备的入站授权？其运行中的直连命令、终端和桌面连接会关闭，后台 Shell 会话仍保留。', 'Revoke this device’s incoming access? Its running direct commands, terminal and desktop attachments will close. Background shells remain running.'))) void run(async () => {
          const value = await client({ operation: 'revoke', node: grant.node }) as DirectCatalog;
          if (mounted.current) onCatalog(value);
        });
      }}><Trash2 size={16} aria-hidden="true" />{t('撤销', 'Revoke')}</button></div>)}
      {!catalog.grants?.length && <p className="direct-note">{t('尚未授权其他设备访问此接入端。', 'No devices have incoming access to this endpoint.')}</p>}
    </div>
    <dialog ref={dialog} className="direct-token-dialog" aria-labelledby={titleId} onCancel={event => { event.preventDefault(); dismiss(); }} onClose={() => { if (issued) dismiss(); }}>
      {issued && <><div className="direct-heading"><h3 id={titleId}>{t('保存授权 Token', 'Save access token')}</h3><button autoFocus aria-label={t('关闭 Token 窗口', 'Close token window')} onClick={dismiss}><X size={18} aria-hidden="true" /></button></div><p>{t('此 Token 仅在创建时展示。将它填入被授权设备的“添加直连设备”表单，目标 ID 使用：', 'This token is shown only when created. Enter it in the granted device’s Add direct device form, using this target ID:')}</p><code>{catalog.node_id}</code><p className="direct-note">{t('被授权设备：', 'Granted device: ')}{issued.node}</p><label htmlFor={tokenId}>{t('新授权 Token', 'New access token')}</label><textarea id={tokenId} ref={tokenField} readOnly value={issued.token} rows={3} spellCheck={false} autoComplete="off" /><p className="direct-note">{t('关闭窗口后无法再次查看；如果丢失，请撤销授权后重新创建。', 'You cannot view it again after closing this window. If lost, revoke access and create a new grant.')}</p>{copyError && <p className="direct-error" role="alert">{copyError}</p>}<div className="direct-actions"><button onClick={() => void copy()}><Copy size={16} aria-hidden="true" />{copied ? t('已复制', 'Copied') : t('复制 Token', 'Copy token')}</button><button className="direct-primary" onClick={dismiss}>{t('已保存，关闭', 'Saved, close')}</button></div></>}
    </dialog>
  </section>;
}
