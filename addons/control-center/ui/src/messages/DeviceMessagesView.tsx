import { useCallback, useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { Archive, CheckCheck, Inbox, MessageSquare, RefreshCw, Reply, Send, ShieldCheck, X } from 'lucide-react';
import type { DirectCatalog, DirectClient } from '../devices/DirectDevicesView';
import { AgentSessions } from '../tasks/AgentSessions';
import { HeldMessages } from './HeldMessages';
import '../devices/devices.css';
import './messages.css';

export type MessageCommand = { operation: 'list'; session?: string } | { operation: 'send'; id: string; node: string; text: string; thread?: string; in_reply_to?: string; sender_session?: string; target_session?: string } | { operation: 'acknowledge' | 'retry' | 'archive' | 'release' | 'reject'; id: string } | { operation: 'held' };
type Envelope = { id: string; thread: string; in_reply_to: string | null; sender: string; recipient: string; sender_session?: string | null; target_session?: string | null; text: string; created_at: number; expires_at: number };
type Item = { message: Envelope; status: string; attempts?: number; error?: string | null };
type Mailbox = { owner: string; incoming: Item[]; outgoing: Item[]; capacity: number; used: number; max_text_bytes: number; held?: number };
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
function identifier() {
  const bytes = crypto.getRandomValues(new Uint8Array(16)); bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
  const hex = [...bytes].map(byte => byte.toString(16).padStart(2, '0')).join('');
  return [hex.slice(0, 8), hex.slice(8, 12), hex.slice(12, 16), hex.slice(16, 20), hex.slice(20)].join('-');
}

/** A local durable mailbox, shared by the browser and the node's native shell. */
export function DeviceMessagesView({ client, language = 'en', onDevices }: { client: DirectClient; language?: string; onDevices?: () => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [catalog, setCatalog] = useState<DirectCatalog>(), [mailbox, setMailbox] = useState<Mailbox>();
  const [readError, setReadError] = useState(''), [formError, setFormError] = useState(''), [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false), [reading, setReading] = useState(false), [paused, setPaused] = useState(false), [updated, setUpdated] = useState<number>();
  const [node, setNode] = useState(''), [text, setText] = useState(''), [reply, setReply] = useState<Envelope>(), [box, setBox] = useState<'incoming' | 'outgoing'>('incoming');
  const [targetSession, setTargetSession] = useState(''), [targetValid, setTargetValid] = useState(true);
  const mounted = useRef(true), mutation = useRef(false), active = useRef(false), serial = useRef(0), draft = useRef<{ key: string; id: string }>(undefined), field = useRef<HTMLTextAreaElement>(null), focusAfterSend = useRef(false), help = useId();
  const routes = (catalog?.routes || []).filter(route => route.node !== mailbox?.owner);
  const replySession = reply ? (reply.sender === mailbox?.owner ? reply.target_session : reply.sender_session) : undefined;
  const bytes = new TextEncoder().encode(text).length, maxBytes = mailbox?.max_text_bytes || 8192;
  const load = useCallback(async () => {
    if (active.current || mutation.current || document.hidden) return;
    active.current = true; const request = ++serial.current; setReading(true);
    try {
      const [configuration, messages] = await Promise.all([client({ operation: 'list' }) as Promise<DirectCatalog>, client({ operation: 'messages', command: { operation: 'list' } }) as Promise<Mailbox>]);
      if (mounted.current && request === serial.current) { setCatalog(configuration); setMailbox(messages); setReadError(''); setUpdated(Date.now()); }
    } catch (error) { if (mounted.current && request === serial.current) setReadError(errorText(error)); }
    finally { active.current = false; if (mounted.current) setReading(false); }
  }, [client]);
  useEffect(() => {
    mounted.current = true; void load();
    return () => { mounted.current = false; ++serial.current; };
  }, [load]);
  useEffect(() => {
    if (!busy && focusAfterSend.current) { focusAfterSend.current = false; field.current?.focus(); }
  }, [busy]);
  useEffect(() => {
    if (paused) return;
    const tick = () => void load(), timer = setInterval(tick, 3000);
    document.addEventListener('visibilitychange', tick); window.addEventListener('cranibox:peers-changed', tick);
    return () => { clearInterval(timer); document.removeEventListener('visibilitychange', tick); window.removeEventListener('cranibox:peers-changed', tick); };
  }, [load, paused]);
  async function run(command: MessageCommand, after?: () => void) {
    if (mutation.current) return;
    mutation.current = true; ++serial.current; setBusy(true); setFormError(''); setNotice('');
    try {
      const value = await client({ operation: 'messages', command }) as Mailbox;
      if (!mounted.current) return;
      setMailbox(value); setUpdated(Date.now()); setReadError(''); after?.();
    } catch (error) { if (mounted.current) { setFormError(errorText(error)); if (command.operation === 'send') focusAfterSend.current = true; } }
    finally { mutation.current = false; if (mounted.current) setBusy(false); }
  }
  async function send(event: FormEvent) {
    event.preventDefault();
    if (!text.trim() || bytes > maxBytes || !targetValid || !routes.some(route => route.node === node)) return;
    const key = JSON.stringify([node, targetSession, text, reply?.thread, reply?.id]);
    if (draft.current?.key !== key) draft.current = { key, id: identifier() };
    await run({ operation: 'send', id: draft.current.id, node, text, ...(targetSession ? { target_session: targetSession } : {}), ...(reply ? { thread: reply.thread, in_reply_to: reply.id } : {}) }, () => {
      setText(''); setReply(undefined); draft.current = undefined; setBox('outgoing');
      setNotice(t('消息已保存在本机发件箱。节点服务会投递，并在断线后重试。', 'Saved in this device’s outbox. The node service delivers it and retries after an outage.'));
    });
  }
  function beginReply(message: Envelope) {
    const target = message.sender === mailbox?.owner ? message.recipient : message.sender;
    if (text && !confirm(t('替换当前草稿并回复这条消息？', 'Replace the current draft and reply to this message?'))) return;
    setNode(target); const session = (message.sender === mailbox?.owner ? message.target_session : message.sender_session) || ''; setTargetSession(session); setTargetValid(current => session === targetSession && target === node ? current : !session); setReply(message); setText(''); setFormError(''); setNotice(''); field.current?.focus();
  }
  const status = (value: string) => ({ queued: t('本机排队', 'Queued locally'), received: t('目标已收到', 'Received by device'), acknowledged: t('已确认收件', 'Acknowledged'), blocked: t('需要处理', 'Needs attention'), expired: t('已过期', 'Expired') }[value] || value);
  const items = [...(mailbox?.[box] || [])].sort((a, b) => b.message.created_at - a.message.created_at || a.message.id.localeCompare(b.message.id));
  return <section className="direct-devices device-messages" aria-label={t('设备消息', 'Device messages')}>
    <div className="direct-heading"><div><h2><MessageSquare size={22} aria-hidden="true" />{t('设备消息', 'Device messages')}</h2><p>{t('给另一台设备留言、回复和确认收件。消息保存在设备上，关闭页面后继续投递。', 'Send, reply and acknowledge across devices. Messages are saved on the device and delivery continues after closing this page.')}</p></div><div className="direct-actions"><button aria-pressed={paused} onClick={() => setPaused(value => !value)}>{paused ? t('恢复自动刷新', 'Resume updates') : t('暂停自动刷新', 'Pause updates')}</button><button disabled={reading || busy} onClick={() => void load()}><RefreshCw size={16} aria-hidden="true" />{t('刷新消息', 'Refresh messages')}</button></div></div>
    {readError && <p className="direct-error" role="alert">{readError}{mailbox && t(' · 显示上次读取的记录。', ' · Showing the last saved view.')}</p>}
    {!mailbox ? <p role="status">{readError ? t('无法读取本机消息。请检查设备服务，然后刷新重试。', 'Messages could not be read. Check the device service, then refresh to retry.') : t('正在读取本机消息…', 'Reading this device’s messages…')}</p> : <>
      <div className="messages-identity"><span>{t('本机身份', 'This device')}<code>{mailbox.owner}</code></span><span>{t('存储占用', 'Stored records')}<strong>{mailbox.used} / {mailbox.capacity}</strong></span><span>{t('最近读取', 'Last read')}<time>{updated ? new Date(updated).toLocaleTimeString(language) : '—'}</time></span></div>
      <div className="messages-layout"><div className="messages-compose"><form className="direct-editor" onSubmit={send} aria-label={t('撰写设备消息', 'Compose device message')}>
        <h3>{t('新消息', 'New message')}</h3>
        <label>{t('目标设备', 'Target device')}<select required disabled={busy || !!reply} value={node} onChange={event => { setNode(event.target.value); setTargetSession(''); setTargetValid(true); }}><option value="">{t('选择已连接设备', 'Choose a connected device')}</option>{routes.map(route => <option key={route.node} value={route.node}>{route.node}</option>)}</select></label>
        {!routes.length && <p className="direct-note">{t('先在设备网络中添加直连、EasyTier 或 Tailscale 连接。', 'First add a direct, EasyTier or Tailscale connection in Device network.')}</p>}
        {node && <AgentSessions key={node} client={client} node={node} language={language} paused={paused} selected={targetSession} onSelect={setTargetSession} onValidChange={setTargetValid} disabled={busy || !!replySession} purpose="message" />}
        {reply && <div className="messages-reply"><span>{t('回复消息', 'Replying to')}<code>{reply.id}</code></span><button type="button" disabled={busy} aria-label={t('取消回复', 'Cancel reply')} onClick={() => { setReply(undefined); setTargetSession(''); setTargetValid(true); field.current?.focus(); }}><X size={16} aria-hidden="true" /></button></div>}
        <label>{t('消息内容', 'Message text')}<textarea ref={field} rows={6} value={text} disabled={busy} aria-describedby={help} aria-invalid={bytes > maxBytes || undefined} onChange={event => setText(event.target.value)} /></label>
        <p id={help} className={bytes > maxBytes ? 'direct-error' : 'direct-note'}>{bytes} / {maxBytes} {t('UTF-8 字节 · 纯文本', 'UTF-8 bytes · Plain text')}</p>
        {formError && <p className="direct-error" role="alert">{formError}</p>}
        <button className="direct-primary" type="submit" disabled={busy || !text.trim() || bytes > maxBytes || !targetValid || !routes.some(route => route.node === node)}><Send size={16} aria-hidden="true" />{busy ? t('正在保存…', 'Saving…') : t('保存并发送', 'Save and send')}</button>
      </form><aside className="messages-guidance"><ShieldCheck size={20} aria-hidden="true" /><div><h3>{t('消息权限单独授权', 'Separate message access')}</h3><p>{t('目标需授予本机“收发设备消息”权限；回复还需要反向连接和授权。收到消息不会运行命令或输入到终端。', 'The target must grant this device message access. Replies need a reverse connection and grant. Receiving a message does not execute a command or type into a terminal.')}</p>{onDevices && <button onClick={onDevices}>{t('配置设备与授权', 'Configure devices and access')}</button>}<p>{t('自动投递和收件查询最长持续 7 天。每台设备最多保存 256 条收发记录。', 'Delivery and receipt checks continue for up to 7 days. Each device stores up to 256 incoming and outgoing records.')}</p><p>{t('收件确认仅表示用户明确确认了消息，不代表 Agent 已完成任务。', 'Acknowledgement means the recipient explicitly confirmed receipt. It does not mean an Agent completed a task.')}</p></div></aside></div>
      <div className="messages-feed"><HeldMessages client={client} language={language} count={mailbox.held || 0} refreshed={updated} onChanged={() => void load()} /><div className="messages-tabs" role="group" aria-label={t('消息方向', 'Message direction')}><button aria-pressed={box === 'incoming'} onClick={() => setBox('incoming')}><Inbox size={17} aria-hidden="true" />{t('收件箱', 'Inbox')}<span>{mailbox.incoming.length}</span></button><button aria-pressed={box === 'outgoing'} onClick={() => setBox('outgoing')}><Send size={17} aria-hidden="true" />{t('发件箱', 'Outbox')}<span>{mailbox.outgoing.length}</span></button></div>
        {notice && <p className="messages-notice" role="status">{notice}</p>}
        {items.length === 0 && <div className="direct-empty"><Inbox size={28} aria-hidden="true" /><h3>{box === 'incoming' ? t('暂无来信', 'No incoming messages') : t('暂无发出的消息', 'No outgoing messages')}</h3><p>{t('消息不会存入浏览器。重新打开页面，会从本机服务读取记录。', 'Messages are stored by the device service. Reopen this page to read saved records.')}</p></div>}
        {items.map(item => {
          const message = item.message, target = box === 'incoming' ? message.sender : message.recipient, canReply = routes.some(route => route.node === target);
          return <article className="message-card" key={message.id} aria-label={t('消息 ', 'Message ') + message.id}>
            <div className="message-heading"><div><small>{box === 'incoming' ? t('来自', 'From') : t('发送至', 'To')}</small><strong>{target}</strong></div><span className={'message-status message-' + item.status}>{box === 'incoming' && item.status === 'received' ? t('待确认收件', 'Awaiting acknowledgement') : status(item.status)}</span></div>
            {message.target_session && <p className="message-agent-address">{t('收件 Agent', 'Recipient Agent')}<code>{message.target_session}</code></p>}
            <p className="message-text">{message.text}</p>
            <details><summary>{t('消息详情与会话', 'Message and thread details')}</summary><dl>{[[t('消息 ID', 'Message ID'), message.id], [t('会话 ID', 'Thread ID'), message.thread], [t('回复于', 'In reply to'), message.in_reply_to || '—'], [t('发件 Agent', 'Sender Agent'), message.sender_session || '—'], [t('收件 Agent', 'Recipient Agent'), message.target_session || t('设备收件箱', 'Device inbox')], [t('发送时间', 'Sent'), new Date(message.created_at * 1000).toLocaleString(language)], [t('投递截止', 'Delivery deadline'), new Date(message.expires_at * 1000).toLocaleString(language)]].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl></details>
            {item.error && <p className="direct-error">{item.error}</p>}
            <div className="direct-actions">{box === 'incoming' && item.status !== 'acknowledged' && <button disabled={busy} onClick={() => void run({ operation: 'acknowledge', id: message.id })}><CheckCheck size={16} aria-hidden="true" />{t('确认收件', 'Acknowledge receipt')}</button>}<button disabled={busy || !canReply} title={!canReply ? t('需要先配置反向连接和消息授权', 'Configure a reverse connection and message grant first') : undefined} onClick={() => beginReply(message)}><Reply size={16} aria-hidden="true" />{t('回复', 'Reply')}</button>{box === 'outgoing' && item.status !== 'acknowledged' && message.expires_at * 1000 > Date.now() && <button disabled={busy} onClick={() => void run({ operation: 'retry', id: message.id })}><RefreshCw size={16} aria-hidden="true" />{t('重试原消息', 'Retry same message')}</button>}<button disabled={busy} onClick={() => {
              if (confirm(box === 'outgoing' ? t('删除本机发件记录并停止后续重试？目标已收到或正在传输的消息无法撤回。', 'Delete this local outgoing record and stop future retries? Messages already received or in transit cannot be recalled.') : t('隐藏这条来信？去重和收件回执会保留到投递截止。', 'Hide this incoming message? Deduplication and receipts remain until its delivery deadline.'))) void run({ operation: 'archive', id: message.id });
            }}><Archive size={16} aria-hidden="true" />{box === 'incoming' ? t('归档', 'Archive') : t('删除记录', 'Delete record')}</button></div>
            {box === 'incoming' && !canReply && <p className="direct-note">{t('尚未配置到发送方的连接。配置后可回复。', 'No connection to the sender is configured. Add it before replying.')}</p>}
          </article>;
        })}
        {mailbox.used > mailbox.incoming.length + mailbox.outgoing.length && <p className="direct-note">{t('存储占用包含用于防止重复投递的已归档来信，过期后会在下次保存时清理。', 'Stored records include archived incoming messages retained for deduplication. Expired archived entries are reclaimed on the next save.')}</p>}
      </div></div>
    </>}
  </section>;
}
