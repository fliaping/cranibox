import { Copy, LogOut, MonitorSmartphone, PlugZap, SquareTerminal } from "lucide-react";
import { useEffect, useState } from "react";
import { shellCall, type CommandLine, type ShellStatus } from "./shell";
import { useApp } from "./store";
import { Alert, Card, Sheet, Switch } from "./ui";

/** Autostart, the command line, agent MCP access and quit: only inside the desktop app. */
export function DesktopAppSettings({ section }: { section?: "app" | "agents" }) {
  const { t } = useApp();
  const [status, setStatus] = useState<ShellStatus | null>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false);
  const [cli, setCli] = useState<CommandLine | null>(null);
  const [runs, setRuns] = useState(false), [config, setConfig] = useState(""), [copied, setCopied] = useState(false), [quit, setQuit] = useState(false);
  async function step(action: () => Promise<void>) {
    setBusy(true); setError("");
    try { await action(); } catch (problem) { setError(problem instanceof Error ? problem.message : String(problem)); } finally { setBusy(false); }
  }
  useEffect(() => {
    void step(async () => setStatus(await shellCall<ShellStatus>("status")));
    shellCall<CommandLine>("command_line").then(value => setCli(typeof value?.program === "string" ? value : null), () => setCli(null));
  }, []);
  return <>
    {section !== "agents" && <Card label={t("桌面应用", "Desktop app")}>
      <div className="cx-card-head"><span className="cx-card-icon"><MonitorSmartphone size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("桌面应用", "Desktop app")}</h3>
        <p className="cx-muted cx-small">{status ? "Cranibox " + status.version + " · " + status.platform : t("正在读取…", "Loading…")}</p></div></div>
      <div className="cx-toggle-list"><Switch label={t("登录时自动启动", "Start at login")} detail={t("开机后在后台保持这台设备在线。", "Keeps this device online in the background after you sign in.")} checked={!!status?.autostart} disabled={busy || !status} onChange={enabled => void step(async () => setStatus(await shellCall<ShellStatus>("autostart", { enabled })))} /></div>
      {cli && <div className="cx-command-line" style={{ marginTop: 12 }}>
        <p className="cx-small"><SquareTerminal size={15} aria-hidden="true" /> <strong>{t("命令行", "Command line")}</strong> · {cli.ready
          ? t("在终端里直接运行 cranibox，与窗口控制的是同一台设备。", "Run cranibox in a terminal to control this same device.")
          : cli.command ? t("PATH 中的 cranibox 是另一个程序：", "The cranibox on PATH is another program: ") + cli.command
          : t("cranibox 还不在 PATH 中。", "cranibox is not on PATH yet.")}</p>
        {!cli.ready && (cli.installable
          ? <div className="cx-actions"><button className="cx-btn" disabled={busy} onClick={() => void step(async () => setCli(await shellCall<CommandLine>("install_command_line")))}>{t("安装命令行工具", "Install command line tool")}</button></div>
          : status?.platform !== "windows" && <div className="cx-plan"><pre aria-label={t("加入 PATH 的命令", "Command that adds it to PATH")}>{"ln -sf '" + cli.program.replace(/'/g, "'\\''") + "' ~/.local/bin/cranibox"}</pre></div>)}
      </div>}
      <div className="cx-actions" style={{ marginTop: 12 }}>
        <button className="cx-btn" disabled={busy} onClick={() => void step(() => shellCall("hide").then(() => undefined))}>{t("隐藏窗口", "Hide window")}</button>
        <button className="cx-btn danger" disabled={busy} onClick={() => setQuit(true)}><LogOut size={15} aria-hidden="true" />{t("退出 Cranibox", "Quit Cranibox")}</button>
      </div>
    </Card>}
    {section !== "app" && <Card label={t("Agent 接入（MCP 配置）", "Agent access (MCP config)")}>
      <div className="cx-card-head"><span className="cx-card-icon"><PlugZap size={20} aria-hidden="true" /></span><div className="cx-grow"><h3>{t("Agent 接入（MCP 配置）", "Agent access (MCP config)")}</h3>
        <p className="cx-muted cx-small">{t("把这段配置加入 Codex、Claude Code 等 Agent，它们就能使用你所有设备上的终端、桌面与浏览器。", "Add this configuration to Codex, Claude Code or another agent so it can use the terminal, desktop and browser on all your devices.")}</p></div></div>
      <label className="cx-check"><input type="checkbox" checked={runs} onChange={event => { setRuns(event.target.checked); setConfig(""); }} />{t("包含托管运行工具", "Include managed run tools")}</label>
      <div className="cx-actions"><button className="cx-btn primary" disabled={busy} onClick={() => void step(async () => setConfig(JSON.stringify(await shellCall("mcp_config", { include_runs: runs }), null, 2)))}>{t("生成配置", "Generate configuration")}</button>
        {config && <button className="cx-btn" onClick={() => void navigator.clipboard.writeText(config).then(() => setCopied(true), () => setCopied(false))}><Copy size={15} aria-hidden="true" />{copied ? t("已复制", "Copied") : t("复制", "Copy")}</button>}</div>
      {config && <div className="cx-plan" style={{ marginTop: 12 }}><pre aria-label={t("MCP 配置", "MCP configuration")}>{config}</pre></div>}
    </Card>}
    {error && <Alert>{error}</Alert>}
    {quit && <Sheet title={t("退出 Cranibox？", "Quit Cranibox?")} t={t} onClose={() => setQuit(false)}>
      <p>{t("退出后，其他设备将无法连接这台设备，正在这台设备上运行的终端和 Agent 会话也会结束。", "After quitting, your other devices can no longer reach this one, and terminal and agent sessions running on it end.")}</p>
      <div className="cx-actions"><button className="cx-btn" onClick={() => setQuit(false)}>{t("取消", "Cancel")}</button><button className="cx-btn danger" disabled={busy} onClick={() => void step(() => shellCall("quit").then(() => undefined))}>{t("退出", "Quit")}</button></div>
    </Sheet>}
  </>;
}
