import { useEffect, useState } from 'react';
import { inShell } from './app/shell';
import { Sheet } from './app/ui';
import { t } from './workspace';

type Question = { message: string; answer: (ok: boolean) => void };
let ask_: ((question: Question) => void) | undefined;

/** `window.confirm` for every page. The desktop app's webview has no native
 *  dialog on macOS (WKWebView answers "cancel" without showing anything), so
 *  inside the app the question is asked in the page instead. */
export function ask(message: string): Promise<boolean> {
  if (!inShell()) return Promise.resolve(window.confirm(message));
  return new Promise(answer => ask_ ? ask_({ message, answer }) : answer(false));
}

export function ConfirmHost() {
  const [question, setQuestion] = useState<Question | null>(null);
  useEffect(() => {
    ask_ = next => setQuestion(current => { current?.answer(false); return next; });
    return () => { ask_ = undefined; };
  }, []);
  if (!question) return null;
  const done = (ok: boolean) => { question.answer(ok); setQuestion(null); };
  return <Sheet title={t('请确认', 'Please confirm')} t={t} onClose={() => done(false)}>
    <p className="cx-confirm-message">{question.message}</p>
    <div className="cx-actions"><button className="cx-btn" onClick={() => done(false)}>{t('取消', 'Cancel')}</button><button className="cx-btn primary" autoFocus onClick={() => done(true)}>{t('确定', 'OK')}</button></div>
  </Sheet>;
}
