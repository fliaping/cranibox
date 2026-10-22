import { CircleCheck, MessageCircleQuestion } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import type { TaskSummary } from "../personal/api";
import { useApp } from "./store";
import { ApprovalCard, Card, Empty, Section } from "./ui";

export function Approvals() {
  const { t, api, approvals, busy, run, deviceName } = useApp();
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  useEffect(() => {
    let active = true;
    const load = () => api.call<TaskSummary[]>("GET", "/tasks").then(items => { if (active) setTasks(items); }).catch(() => { /* Tasks are optional here. */ });
    void load();
    const timer = setInterval(() => { if (!document.hidden) void load(); }, 10000);
    return () => { active = false; clearInterval(timer); };
  }, [api]);
  const questions = tasks.flatMap(task => (task.questions || []).filter(question => question.answer === null).map(question => ({ task, question })));
  return <>
    {approvals.length ? <div className="cx-stack">{approvals.map(card => <ApprovalCard key={card.id} card={card} t={t} busy={busy} deviceName={deviceName}
      onDecide={decision => void run(() => api.call("POST", "/approvals/" + encodeURIComponent(card.id), { decision }))} />)}</div>
      : <Empty icon={CircleCheck} title={t("都处理好了", "You’re all caught up")}>{t("配对新设备、Agent 使用工具、操作屏幕等需要你决定的事，会出现在这里。", "Pairing a new device, an agent using a tool or controlling a screen: anything that needs your decision appears here.")}</Empty>}
    {questions.length > 0 && <Section title={t("任务里等你回答的问题", "Questions from your tasks")} actions={<Link className="cx-btn small" to="/tasks">{t("打开任务", "Open tasks")}</Link>}>
      <div className="cx-stack">{questions.map(({ task, question }) => <Card key={task.id + question.id}><div className="cx-card-head"><span className="cx-card-icon"><MessageCircleQuestion size={19} aria-hidden="true" /></span><div className="cx-grow"><h3>{task.title}</h3><p className="cx-untrusted">{question.text}</p></div></div></Card>)}</div>
    </Section>}
  </>;
}
