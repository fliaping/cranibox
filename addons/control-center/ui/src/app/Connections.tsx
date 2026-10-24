import { Languages, Moon, Sun, SunMoon } from "lucide-react";
import { Link } from "react-router-dom";
import { DesktopAppSettings } from "./DesktopAppSettings";
import { inShell } from "./shell";
import { useApp, type Theme } from "./store";
import { Card } from "./ui";
import { UpdatePanel } from "./UpdatePanel";

export function Settings() {
  const { t, theme, setTheme, locale, setLocale } = useApp();
  const themes: [Theme, string, typeof Sun][] = [["system", t("跟随系统", "System"), SunMoon], ["light", t("浅色", "Light"), Sun], ["dark", t("深色", "Dark"), Moon]];
  return <div className="cx-stack loose">
    {inShell() && <DesktopAppSettings section="app" />}
    <UpdatePanel node="self" />
    <Card label={t("外观", "Appearance")}><div className="cx-between"><div><h3>{t("外观", "Appearance")}</h3><p className="cx-muted cx-small">{t("浅色、深色，或跟随系统。", "Light, dark, or follow your system.")}</p></div>
      <div className="cx-segment" role="group" aria-label={t("外观", "Appearance")}>{themes.map(([value, label, Icon]) => <button key={value} aria-pressed={theme === value} onClick={() => setTheme(value)}><Icon size={15} aria-hidden="true" /> {label}</button>)}</div></div></Card>
    <Card label={t("语言", "Language")}><div className="cx-between"><div><h3><Languages size={16} aria-hidden="true" /> {t("语言", "Language")}</h3><p className="cx-muted cx-small">{t("所有页面都会切换。", "Applies to every page.")}</p></div>
      <label><span className="cx-sr">{t("语言", "Language")}</span><select className="cx-select" aria-label={t("语言", "Language")} value={locale.startsWith("zh") ? "zh-cn" : "en"} onChange={event => setLocale(event.target.value)}><option value="zh-cn">简体中文</option><option value="en">English</option></select></label></div></Card>
    <Card label={t("设置向导", "Setup guide")}><div className="cx-between"><div><h3>{t("设置向导", "Setup guide")}</h3><p className="cx-muted cx-small">{t("重新检查这台设备的权限、身份与恢复码。", "Check this device's permissions, identity and recovery code again.")}</p></div>
      <Link className="cx-btn" to="/app/setup">{t("打开向导", "Open setup guide")}</Link></div></Card>
    <Card label={t("隐私", "Privacy")}><h3>{t("隐私", "Privacy")}</h3><p className="cx-muted cx-small">{t("设备之间的流量端到端加密，经对等连接直接传输。没有中心服务器保存你的数据；需要你决定的操作都会出现在「待办」。", "Traffic between your devices is end-to-end encrypted and travels over direct peer connections. No central server keeps your data; anything that needs your decision appears in your Inbox.")}</p></Card>
  </div>;
}
