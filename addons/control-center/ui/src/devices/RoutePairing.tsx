import { useState, type FormEvent } from 'react';
import { BadgeCheck, ShieldAlert } from 'lucide-react';
import type { DirectCatalog, DirectClient, DirectRoute } from './DirectDevicesView';

// Caller side of pairing: the code the target asks for is shown here, and the owner can
// confirm the target's code. The target's strict grant is what enforces verification.
export function RoutePairing({ route, client, language, busy, run, onCatalog }: {
  route: DirectRoute; client: DirectClient; language: string; busy: boolean;
  run: (action: () => Promise<void>) => Promise<void>; onCatalog: (catalog: DirectCatalog) => void;
}) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [code, setCode] = useState(''), [error, setError] = useState('');
  const identity = route.identity;
  if (!identity) return <p className="direct-pairing">{t('首次连接后显示配对码', 'The pairing code appears after the first connection')}</p>;
  if (identity.verified) return <p className="direct-pairing direct-pairing-verified"><BadgeCheck size={15} aria-hidden="true" />{t('配对码已核对', 'Pairing code verified')} <code>{identity.pairing_code || '—'}</code></p>;
  function verify(event: FormEvent) {
    event.preventDefault();
    void run(async () => {
      try {
        const value = await client({ operation: 'verify', node: route.node, code: code.trim() }) as { catalog: DirectCatalog };
        setCode(''); setError(''); onCatalog(value.catalog);
      } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    });
  }
  return <form className="direct-pairing direct-pairing-pending" onSubmit={verify} aria-label={t('核对配对码 ', 'Verify pairing code ') + route.node}>
    <span><ShieldAlert size={15} aria-hidden="true" />{t('未核对：请在目标设备输入此配对码', 'Not verified: enter this code on the target device')}</span>
    <span>{t('本机显示的配对码', 'Code shown here')} <code>{identity.pairing_code || '—'}</code></span>
    <label>{t('输入对方设备显示的配对码', 'Enter the code shown on the other device')}<input inputMode="numeric" autoComplete="off" pattern="[0-9 -]{12,16}" required value={code} disabled={busy} onChange={event => setCode(event.target.value)} /></label>
    <button type="submit" disabled={busy}><BadgeCheck size={16} aria-hidden="true" />{t('核对并信任', 'Verify and trust')}</button>
    {error && <span className="direct-error" role="alert">{error}</span>}
  </form>;
}
