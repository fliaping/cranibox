import { ArrowRight } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Modal } from '../overlays';
import { request, t } from '../workspace';

export function NodeIdentityDialog({ source, onCancel, onMigrated }: { source: string; onCancel: () => void; onMigrated: (id: string) => void }) {
  const [target, setTarget] = useState(''), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { input.current?.focus(); }, []);
  async function migrate(event: React.FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true); setError('');
    try {
      const result = await request('/api/nodes/identity', { id: source, target: target.trim() });
      onMigrated(result.id);
    } catch (problem) {
      setError(problem instanceof Error ? problem.message : String(problem));
      input.current?.focus();
    } finally { setBusy(false); }
  }
  return <Modal className="node-identity-dialog" title={t('迁移节点身份', 'Migrate node identity')} onCancel={() => { if (!busy) onCancel(); }}>
    <form onSubmit={event => void migrate(event)} aria-busy={busy}>
      <p className="muted" id="identity-help">{t('将旧网关注册改为这台设备的稳定 NodeID。请从对应客户端的“这台设备”复制 NodeID，确认属于同一台设备。', 'Move a gateway registration to this device’s persistent NodeID. Copy it from “This device” in the matching client and confirm it is the same device.')}</p>
      <label className="field"><span>{t('原节点 ID', 'Previous node ID')}</span><input readOnly value={source} /></label>
      <label className="field"><span>{t('目标 NodeID', 'Target NodeID')}</span><input ref={input} autoFocus required maxLength={64} pattern="[A-Za-z0-9_-]{1,64}" placeholder="node-…" autoComplete="off" spellCheck={false} value={target} onChange={event => setTarget(event.target.value)} aria-describedby={error ? 'identity-help identity-error' : 'identity-help'} aria-invalid={!!error} /></label>
      <p className="muted">{t('凭据、设备组和项目关联会保留；旧连接地址仍然有效。客户端会重连，终端连接需要恢复，节点上的 Shell 会继续运行。有未完成的网关任务时会拒绝迁移。', 'Credentials, device groups and project bindings are retained. Old connection paths remain valid. The client reconnects; terminal attachments need to reconnect while node Shells keep running. Finish gateway tasks before migrating.')}</p>
      {error && <p className="form-error" id="identity-error" role="alert">{error}</p>}
      <div className="modal-actions"><button type="button" className="button secondary" disabled={busy} onClick={onCancel}>{t('取消', 'Cancel')}</button><button type="submit" className="button primary" disabled={busy || !target.trim() || target.trim() === source}><ArrowRight size={16} aria-hidden="true" />{busy ? t('正在迁移…', 'Migrating…') : t('确认迁移', 'Migrate identity')}</button></div>
    </form>
  </Modal>;
}
