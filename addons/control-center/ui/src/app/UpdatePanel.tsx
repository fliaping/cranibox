import { Download } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { NodeUpdate, UpdateAccepted, UpdateAdopted, UpdateChannel, UpdatePhase, UpdateRequest, UpdateStatus } from "../types";
import { inShell, shellCall } from "./shell";
import { useApp } from "./store";
import { Alert, Card, Pill, Sheet } from "./ui";

const channels: UpdateChannel[] = ["latest", "next", "dev"];
/** An owner's rollback; an automatic recovery from an unhealthy version has another spec. */
const reverted = (status: UpdateStatus | null) => status?.phase === "rolled_back" && status.spec === "rollback";
const failed = (status: UpdateStatus | null) => status?.phase === "failed" || (status?.phase === "rolled_back" && !reverted(status));
/** Set while a new version runs but is not yet confirmed healthy; not a blocker to report. */
const inProgress = (reason: string) => reason.includes("already in progress");
const confirmed = (data: NodeUpdate, target: string | null) => !!target && data.runtime.version === target && data.runtime.state === "confirmed";
const message = (problem: unknown) => problem instanceof Error ? problem.message : String(problem);
type Watch = { id: string; target: string | null; phase: UpdatePhase; started: number };
const isLauncherReason = (reason: string) => /not started by the cranibox launcher|cranibox update launch/i.test(reason);
function renderUpdateReason(reason: string, t: (zh: string, en: string) => string) {
  if (isLauncherReason(reason)) {
    return <>
      <span>{t("当前进程未通过启动器运行，暂不支持自动就地重启更新。", "Started outside the Cranibox launcher — automatic in-place restart is unavailable.")}</span>
      <details className="cx-inline-details" style={{ marginTop: 4 }}>
        <summary>{t("如何通过启动器运行", "How to start with the launcher")}</summary>
        <small className="cx-muted">{t("使用启动器命令运行进程即可启用自动更新：", "Start the process with the launcher command:")} <code>cranibox update launch</code></small>
      </details>
      <span className="cx-sr"> {reason}</span>
    </>;
  }
  return reason;
}


/** Remount between peers so an old request cannot update the new panel. */
export function UpdatePanel({ node }: { node: string }) {
  return <DeviceUpdates key={node} node={node} />;
}
function DeviceUpdates({ node }: { node: string }) {
  const { api, t } = useApp();
  const [data, setData] = useState<NodeUpdate | null>(null), [status, setStatus] = useState<UpdateStatus | null>(null);
  const [channel, setChannel] = useState<UpdateChannel>("latest"), [watch, setWatch] = useState<Watch | null>(null);
  const [busy, setBusy] = useState(false), [error, setError] = useState(""), [capabilityOff, setCapabilityOff] = useState(false);
  const [restarting, setRestarting] = useState(false), [updated, setUpdated] = useState(false), [timedOut, setTimedOut] = useState(false);
  const [rollback, setRollback] = useState(false), [restartNeeded, setRestartNeeded] = useState(false);
  const alive = useRef(false);
  const path = "/update?node=" + encodeURIComponent(node);
  const begin = useCallback((id: string, target: string | null, phase: UpdatePhase) => {
    setUpdated(false); setTimedOut(false); setRestarting(false);
    setWatch({ id, target, phase, started: Date.now() });
  }, []);
  const load = useCallback(async (check = false, interactive = false) => {
    setBusy(true); setError("");
    try {
      const value = await api.call<NodeUpdate>("GET", path + (check ? "&check=1" : ""), undefined, { background: !interactive });
      if (!alive.current) return;
      setCapabilityOff(false);
      setData(value); setStatus(value.status); setTimedOut(false);
      const last = value.status;
      const finished = !!last && (last.phase === "done" || reverted(last)) && confirmed(value, last.to);
      setUpdated(finished);
      if (last && !failed(last) && !finished) begin(last.id, last.to, last.phase);
    } catch (problem) {
      if (!alive.current) return;
      const code = problem && typeof problem === "object" && "code" in problem ? String((problem as { code?: unknown }).code) : "";
      const msg = message(problem);
      if (!interactive && (code === "capability_off" || msg === "capability_off")) {
        setCapabilityOff(true);
      } else {
        setError(msg);
      }
    }
    finally { if (alive.current) setBusy(false); }
  }, [api, path, begin]);
  useEffect(() => {
    alive.current = true;
    void load(false, false);
    return () => { alive.current = false; };
  }, [load]);

  useEffect(() => {
    if (!watch) return;
    let stopped = false, timer: ReturnType<typeof setTimeout>;
    let target = watch.target, phase = watch.phase;
    const finish = () => { stopped = true; clearTimeout(timer); clearTimeout(deadline); setWatch(null); setRestarting(false); };
    // A separate deadline also covers a request that never answers during a restart.
    const deadline = setTimeout(() => { finish(); setTimedOut(true); }, Math.max(0, 300000 - (Date.now() - watch.started)));
    async function poll() {
      let current: UpdateStatus | null = null, pollError = "";
      try { current = await api.call<UpdateStatus>("GET", "/update/" + encodeURIComponent(watch!.id) + "?node=" + encodeURIComponent(node)); }
      catch (problem) { pollError = message(problem); }
      if (stopped) return;
      if (current) {
        phase = current.phase; target = current.to || target;
        setStatus(current); setError("");
        if (failed(current)) { finish(); return; }
      }
      // Activation may restart the process before its status response reaches us.
      // Consult the overall endpoint even if the per-update record disappeared.
      // A rollback restarts into the previous version the same way.
      if (phase === "activated" || phase === "done" || phase === "rolled_back" || !current) {
        setRestarting(true);
        try {
          const value = await api.call<NodeUpdate>("GET", path);
          if (stopped) return;
          setData(previous => ({ ...value, available: previous?.available, available_error: previous?.available_error }));
          const last = value.status;
          if (last?.id === watch!.id) {
            setStatus(last); phase = last.phase; target = last.to || target;
            if (failed(last)) { setError(""); finish(); return; }
          }
          if (confirmed(value, target)) { setError(""); setUpdated(true); finish(); return; }
        } catch (problem) { pollError = message(problem); }
        if (stopped) return;
        const activating = phase === "activated" || phase === "done" || phase === "rolled_back";
        setRestarting(activating);
        setError(activating ? "" : pollError);
      }
      if (!stopped) timer = setTimeout(() => void poll(), 2000);
    }
    timer = setTimeout(() => void poll(), 2000);
    return () => { stopped = true; clearTimeout(timer); clearTimeout(deadline); };
  }, [api, node, path, watch]);

  async function act(action: UpdateRequest["action"]) {
    setBusy(true); setError(""); setTimedOut(false); setUpdated(false);
    try {
      if (action === "apply") {
        const body: UpdateRequest = { node, action, spec: channel };
        const result = await api.call<UpdateAccepted>("POST", "/update", body);
        if (!alive.current) return;
        setStatus(null);
        begin(result.id, data?.available?.[channel] || null, "resolving");
      } else if (action === "rollback") {
        const result = await api.call<UpdateStatus>("POST", "/update", { node, action } satisfies UpdateRequest);
        if (!alive.current) return;
        setRollback(false); setStatus(result);
        if (!failed(result)) begin(result.id, result.to, result.phase);
      } else {
        const result = await api.call<UpdateAdopted>("POST", "/update", { node, action } satisfies UpdateRequest);
        if (!alive.current) return;
        if (result.needs_restart) {
          setRestartNeeded(true);
          if (inShell()) { await shellCall("restart"); if (alive.current) setRestarting(true); }
        } else await load();
      }
    } catch (problem) { if (alive.current) setError(message(problem)); }
    finally { if (alive.current) setBusy(false); }
  }
  const phases: Record<UpdatePhase, string> = {
    resolving: t("正在解析版本…", "Resolving version…"), downloading: t("正在下载…", "Downloading…"),
    verifying: t("正在验证…", "Verifying…"), staged: t("已准备就绪", "Staged"),
    activated: t("已激活", "Activated"), done: t("已完成", "Done"),
    failed: t("更新失败", "Update failed"), rolled_back: t("已回滚", "Rolled back"),
  };
  const locked = busy || !!watch || restartNeeded;
  const version = data?.available?.[channel];
  const progress = status?.total && status.total > 0 ? Math.max(0, Math.min(100, status.downloaded / status.total * 100)) : undefined;
  return <Card label={t("更新", "Updates")} className="cx-update-panel cx-stack">
    <div className="cx-card-head"><span className="cx-card-icon"><Download size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("更新", "Updates")}</h3></div></div>
    {!data && busy && <p role="status" className="cx-muted">{t("正在读取更新状态…", "Loading update status…")}</p>}
    {error && <Alert action={!locked && <button className="cx-btn small" onClick={() => void load(false, true)}>{t("重试", "Retry")}</button>}>{error}</Alert>}
    {!error && !data && capabilityOff && <p className="cx-muted cx-small">{t("目标设备未开启远程更新权限。如需远程检查或安装更新，请先在对方设备的访问权限中开启「远程更新」。", "Remote updates are turned off on this device. Enable Remote updates in its access settings to check or install updates remotely.")}</p>}
    {timedOut && <Alert>{t("5 分钟内未能确认更新。请检查设备是否在线，然后重试读取状态。", "The update could not be confirmed within 5 minutes. Check that the device is online, then retry loading its status.")} <button className="cx-btn small" onClick={() => void load(false, true)}>{t("重试", "Retry")}</button></Alert>}
    {data && <>
      <dl className="cx-facts">
        <div><dt>{t("运行版本", "Running version")}</dt><dd>{data.version}</dd></div>
        <div><dt>{t("平台", "Platform")}</dt><dd>{data.runtime.os} / {data.runtime.arch}</dd></div>
        <div><dt>{t("状态", "State")}</dt><dd>{data.runtime.state === "pending" ? t("等待确认", "Awaiting confirmation") : data.runtime.state === "confirmed" ? t("已确认", "Confirmed") : "—"}</dd></div>
      </dl>
      {!data.runtime.managed && <p className="cx-muted">{node === "self"
        ? t("远程更新需要由 Cranibox 管理版本并在更新后重启设备进程。请先在这台设备上启用远程更新，再重启 Cranibox 使其生效。", "Remote updates require Cranibox to manage versions and restart the device process after updating. Enable remote updates on this device, then restart Cranibox to activate them.")
        : t("远程更新需要由 Cranibox 管理版本。请在那台设备上启用远程更新并重启 Cranibox。", "Remote updates require Cranibox to manage versions. Enable remote updates on that device itself and restart Cranibox.")}</p>}
      {data.runtime.reasons.some(inProgress) && !watch && <p role="status" className="cx-muted cx-small">{data.runtime.state === "pending"
        ? t("新版本已启动，稳定运行约 20 秒后自动确认；确认前不能开始新的更新。", "The new version is running and is confirmed after about 20 seconds of stable operation. A new update can start after that.")
        : t("正在更新，完成前不能开始新的更新。", "An update is in progress. A new update can start after it finishes.")}</p>}
      {!data.runtime.can_update && data.runtime.reasons.some(reason => !inProgress(reason)) && (() => {
        const blockers = data.runtime.reasons.filter(reason => !inProgress(reason));
        return blockers.length === 1 && isLauncherReason(blockers[0])
          ? <Alert tone="info">{renderUpdateReason(blockers[0], t)}</Alert>
          : <Alert tone="warn">{t("这台设备目前无法自行更新。", "This device cannot update itself right now.")}<ul>{blockers.map((reason, index) => <li key={index}>{renderUpdateReason(reason, t)}</li>)}</ul></Alert>;
      })()}
      {data.available_error && <Alert tone="info">{data.available_error}</Alert>}
      {data.available && <label className="cx-field">{t("更新频道", "Update channel")}<select value={channel} disabled={locked} onChange={event => setChannel(event.target.value as UpdateChannel)}>
        {channels.map(item => <option key={item} value={item}>{item} · {data.available?.[item] || t("暂无可用版本", "No version available")}</option>)}
      </select></label>}
      <div className="cx-actions">
        <button className="cx-btn" disabled={locked} onClick={() => void load(true, true)}>{t("检查更新", "Check for updates")}</button>
        {data.available && <button className="cx-btn primary" disabled={locked || !data.runtime.can_update || !version} onClick={() => void act("apply")}>{version ? t("更新到 " + version, "Update to " + version) : t("暂无可用版本", "No version available")}</button>}
        {data.runtime.managed && <button className="cx-btn" disabled={locked} onClick={() => setRollback(true)}>{t("回滚到上一版本", "Roll back")}</button>}
        {node === "self" && !data.runtime.managed && <button className="cx-btn primary" disabled={locked} onClick={() => void act("adopt")}>{t("启用远程更新", "Enable remote updates")}</button>}
      </div>
    </>}
    {(status || watch || updated) && <div className="cx-stack" aria-live="polite">
      <div><Pill tone={failed(status) ? "bad" : updated ? "good" : "info"}>{updated ? (reverted(status) ? t("已回滚", "Rolled back") : t("已更新", "Updated")) : reverted(status) ? t("正在回滚", "Rolling back") : phases[status?.phase || "resolving"]}</Pill>{status && <span className="cx-muted cx-small"> {status.from || "—"} → {status.to || status.spec}</span>}</div>
      {status?.phase === "downloading" && <progress className="cx-update-progress" aria-label={t("下载进度", "Download progress")} max={100} value={progress} />}
      {failed(status) && <Alert>{status?.phase === "rolled_back" && <p>{t("新版本没有通过健康检查，已自动恢复。", "The new version was not healthy and was restored automatically.")}</p>}{status?.error || (status?.phase === "failed" ? t("更新失败，请重试。", "The update failed. Please try again.") : "")}</Alert>}
    </div>}
    {restarting && !restartNeeded && <p role="status">{reverted(status)
      ? t("正在重启到上一版本…", "Restarting into the previous version…")
      : t("正在重启，等待设备确认新版本…", "Restarting… waiting for the device to confirm the new version.")}</p>}
    {restartNeeded && <Alert tone="info">{t("已启用远程更新。", "Remote updates are enabled. ")}{inShell() && !error
      ? t("正在重启 Cranibox…", "Restarting Cranibox…")
      : <>{t("请重启设备进程：", "Restart the device process with: ")}<code>cranibox update launch</code></>}</Alert>}
    {rollback && <Sheet title={t("回滚到上一版本？", "Roll back to the previous version?")} t={t} onClose={() => { if (!busy) setRollback(false); }}>
      <p>{t("设备将切换到上一版本并重启，连接会暂时中断。", "The device will switch to the previous version and restart. Its connection will be interrupted briefly.")}</p>
      {error && <Alert>{error}</Alert>}
      <div className="cx-actions"><button className="cx-btn" disabled={busy} onClick={() => setRollback(false)}>{t("取消", "Cancel")}</button><button className="cx-btn danger" disabled={busy} onClick={() => void act("rollback")}>{t("确认回滚", "Confirm rollback")}</button></div>
    </Sheet>}
  </Card>;
}
