import { useId, useState } from 'react';
import { HelpCircle, Send } from 'lucide-react';

export type Question = { id: string; text: string; answer: string | null };
export type PendingAnswer = { question: string; text: string };

/** A reply is immutable once queued; only a target receipt confirms delivery. */
export function TaskQuestions({ questions, pending, canAnswer = false, busy = false, language, onAnswer }: { questions: Question[]; pending?: PendingAnswer | null; canAnswer?: boolean; busy?: boolean; language: string; onAnswer?: (question: string, text: string, clear: () => void) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  if (!questions.length) return null;
  return <div className="task-questions" aria-label={t('任务问答', 'Task questions')}>
    {questions.map((question, index) => <div className="task-question" key={question.id}>
      <h4><HelpCircle size={17} aria-hidden="true" />{t('执行方提问 ', 'Worker question ') + (index + 1)}</h4>
      <p>{question.text}</p>
      {question.answer !== null ? <div className="task-answer"><strong>{t('目标已确认回复', 'Reply confirmed by target')}</strong><p>{question.answer}</p></div> : pending?.question === question.id ? <div className="task-answer task-answer-pending"><strong>{t('回复已在本机排队', 'Reply queued on this device')}</strong><p>{pending.text}</p><small>{t('等待目标确认，关闭页面后仍会投递。', 'Awaiting target confirmation. Delivery continues after closing this page.')}</small></div> : canAnswer && onAnswer ? <ReplyForm key={question.id} busy={busy} language={language} onSubmit={(text, clear) => onAnswer(question.id, text, clear)} /> : <small>{t('尚未收到回复。任务结束或请求停止后，不再交付新回复。', 'No reply received. New replies are not delivered after completion or cancellation.')}</small>}
    </div>)}
  </div>;
}
function ReplyForm({ busy, language, onSubmit }: { busy: boolean; language: string; onSubmit: (text: string, clear: () => void) => void }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [text, setText] = useState(''), help = useId();
  const bytes = new TextEncoder().encode(text).length;
  return <form className="task-reply" onSubmit={event => { event.preventDefault(); if (text.trim() && bytes <= 4096 && !busy) onSubmit(text, () => setText('')); }}>
    <label>{t('回复执行方', 'Reply to worker')}<textarea rows={3} required disabled={busy} value={text} aria-describedby={help} aria-invalid={bytes > 4096 || undefined} onChange={event => setText(event.target.value)} /></label>
    <small id={help} className={bytes > 4096 ? 'direct-error' : 'direct-note'}>{bytes} / 4096 {t('字节 · 保存后不可修改，请确认内容。', 'bytes · Confirm the text; a saved reply cannot be edited.')}</small>
    <button type="submit" disabled={busy || !text.trim() || bytes > 4096}><Send size={16} aria-hidden="true" />{busy ? t('正在保存…', 'Saving…') : t('保存并回复', 'Save and reply')}</button>
  </form>;
}
