import { useCallback, useEffect, useRef, useState } from 'react';
import { Check, ShieldAlert, X } from 'lucide-react';
import type { DirectClient } from '../devices/DirectDevicesView';

type Envelope = { id: string; thread: string; sender: string; recipient: string; sender_session?: string | null; target_session?: string | null; text: string; created_at: number; expires_at: number };
type HeldItem = { message: Envelope; received_at: number; reason: string | null };
type HeldQueue = { owner: string; held: HeldItem[] };
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

/**
 * Owner approval queue: messages held by the sender's inbound policy or the Agent loop
 * guard. Agents cannot see these until the owner releases them here (or with
 * `workspacectl messages` / `cranibox peer messages`). Text is untrusted and is
 * rendered as plain text only.
 */
export function HeldMessages({ client, language = 'en', count, refreshed, onChanged }: { client: DirectClient; language?: string; count: number; refreshed?: number; onChanged: () => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [queue, setQueue] = useState<HeldItem[]>([]), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const mounted = useRef(true), serial = useRef(0);
  const load = useCallback(async () => {
    const request = ++serial.current;
    try {
      const value = await client({ operation: 'messages', command: { operation: 'held' } }) as HeldQueue;
      if (mounted.current && request === serial.current) { setQueue(value.held || []); setError(''); }
    } catch (failure) { if (mounted.current && request === serial.current) setError(errorText(failure)); }
  }, [client]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; ++serial.current; }; }, []);
  useEffect(() => { if (count > 0 || queue.length) void load(); }, [count, refreshed, load]); // eslint-disable-line react-hooks/exhaustive-deps
  async function decide(operation: 'release' | 'reject', id: string) {
    if (busy) return;
    if (operation === 'reject' && !confirm(t('拒绝并丢弃这条消息？发送方不会收到正文回执，Agent 也不会看到它。', 'Reject and discard this message? Agents will never see it.'))) return;
    setBusy(true); setError(''); ++serial.current;
    try {
      const value = await client({ operation: 'messages', command: { operation, id } }) as HeldQueue;
      if (mounted.current) setQueue(value.held || []);
      onChanged();
    } catch (failure) { if (mounted.current) setError(errorText(failure)); }
    finally { if (mounted.current) setBusy(false); }
  }
  const reason = (value: string | null) => !value ? '' : value.startsWith('agent-to-agent loop guard') ? t('Agent 互发循环保护：10 分钟内超过 12 条 Agent 消息', 'Agent loop guard: more than 12 Agent messages in 10 minutes') : value.startsWith('held by inbound policy') ? t('该设备的入站策略为“待审批”', 'This device’s inbound policy is “hold for approval”') : value;
  if (!count && !queue.length && !error) return null;
  return <section className="messages-held" aria-label={t('待审批消息', 'Messages awaiting approval')}>
    <h3><ShieldAlert size={18} aria-hidden="true" />{t('待审批', 'Awaiting approval')}<span>{queue.length || count}</span></h3>
    <p className="direct-note">{t('这些来信尚未交给任何 Agent。核对来源与内容后再放行；内容视为不可信文本。', 'These messages have not been given to any Agent. Check the sender and text before releasing; treat the text as untrusted.')}</p>
    {error && <p className="direct-error" role="alert">{error}</p>}
    {queue.map(item => <article className="message-card message-held-card" key={item.message.id} aria-label={t('待审批消息 ', 'Held message ') + item.message.id}>
      <div className="message-heading"><div><small>{t('来自', 'From')}</small><strong>{item.message.sender}</strong></div><span className="message-status message-blocked">{t('待审批', 'Held')}</span></div>
      {item.reason && <p className="message-held-reason">{reason(item.reason)}</p>}
      {item.message.sender_session && <p className="message-agent-address">{t('发件 Agent', 'Sender Agent')}<code>{item.message.sender_session}</code></p>}
      <p className="message-text">{item.message.text}</p>
      <div className="direct-actions">
        <button disabled={busy} onClick={() => void decide('release', item.message.id)}><Check size={16} aria-hidden="true" />{t('放行', 'Release')}</button>
        <button disabled={busy} onClick={() => void decide('reject', item.message.id)}><X size={16} aria-hidden="true" />{t('拒绝', 'Reject')}</button>
      </div>
    </article>)}
  </section>;
}
