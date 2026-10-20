import { Bell, Check, Circle, X } from "lucide-react";
import { useEffect, useRef, useId, type ReactNode } from "react";
import type { Approval, Invite, Pill as Status } from "./api";
export type Translate = (zh: string, en: string) => string;
export function Card({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return <section className={"pa-card " + className}>{children}</section>;
}
export function Pill({
  children,
  tone = "",
}: {
  children: ReactNode;
  tone?: string;
}) {
  return <span className={"pa-pill " + tone}>{children}</span>;
}
export function StatusPill({
  status,
  t,
  onClick,
}: {
  status: Status | null;
  t: Translate;
  onClick: () => void;
}) {
  const labels = {
    connected: t("已连接", "Connected"),
    local: t("在本机运行", "Running locally"),
    attention: t("需要你确认", "Needs your attention"),
    no_models: t("添加模型即可开始", "Add a model to begin"),
    offline: t("暂时离线", "Offline"),
  };
  return (
    <button
      className={"pa-status " + (status?.state || "offline")}
      onClick={onClick}
    >
      <Circle size={9} fill="currentColor" aria-hidden="true" />
      {status ? labels[status.state] : t("正在读取状态…", "Checking status…")}
    </button>
  );
}
export function Sheet({
  title,
  children,
  onClose,
  t,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  t: Translate;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const el = dialog.current;
    el?.showModal();
    return () => el?.close();
  }, []);
  return (
    <dialog
      aria-labelledby={titleId}
      className="pa-sheet"
      ref={dialog}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="pa-sheet-head">
        <h2 id={titleId}>{title}</h2>
        <button
          className="pa-icon"
          aria-label={t("关闭", "Close")}
          onClick={onClose}
        >
          <X aria-hidden="true" />
        </button>
      </div>
      {children}
    </dialog>
  );
}
export function QrPanel({ invite, t }: { invite: Invite; t: Translate }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [copied, setCopied] = useState(false);
  // SVG is decoded as an image: scripts and foreignObject never enter the page DOM.
  useEffect(() => {
    let alive = true;
    const image = new Image();
    image.onload = () => {
      if (alive) {
        const ctx = canvas.current?.getContext("2d");
        ctx?.clearRect(0, 0, 240, 240);
        ctx?.drawImage(image, 0, 0, 240, 240);
      }
    };
    image.src =
      "data:image/svg+xml;charset=utf-8," + encodeURIComponent(invite.qr_svg);
    return () => {
      alive = false;
    };
  }, [invite.qr_svg]);
  return (
    <div className="pa-qr">
      <canvas
        ref={canvas}
        width={240}
        height={240}
        role="img"
        aria-label={t("设备配对二维码", "Device pairing QR code")}
      />
      <label>
        {t("邀请链接", "Invitation link")}
        <input
          readOnly
          value={invite.link}
          onFocus={(e) => e.target.select()}
        />
      </label>
      <button
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(invite.link);
            setCopied(true);
          } catch {
            setCopied(false);
          }
        }}
      >
        {copied ? t("已复制", "Copied") : t("复制链接", "Copy link")}
      </button>
      <small>
        {t(
          "15 分钟内有效，仅可使用一次。",
          "Valid for 15 minutes. Can be used once.",
        )}
      </small>
    </div>
  );
}
import { useState } from "react";
export function ApprovalCard({
  card,
  t,
  busy,
  onDecide,
}: {
  card: Approval;
  t: Translate;
  busy: boolean;
  onDecide: (action: string) => void;
}) {
  const kinds: Record<string, string> = {
    pairing: t("核对设备", "Verify a device"),
    message: t("一条设备消息等待确认", "A device message needs your approval"),
    capability: t("设备请求新的权限", "A device requests access"),
    agent_tool: t("Agent 请求使用工具", "Agent requests a tool"),
    computer_use: t("允许操作你的屏幕？", "Allow access to your screen?"),
    model_write: t("确认修改模型配置", "Confirm model settings"),
  };
  const expired = !!card.expires_at && card.expires_at * 1000 <= Date.now();
  return (
    <Card className="pa-approval">
      <div className="pa-row">
        <span className="pa-card-icon">
          <Bell size={19} aria-hidden="true" />
        </span>
        <div className="pa-grow">
          <h3>{kinds[card.kind] || t("需要你确认", "Needs your attention")}</h3>
          <small>{card.from_node || card.agent || card.on_node}</small>
        </div>
        <Pill tone={card.risk === "high" ? "warn" : ""}>
          {card.risk === "high"
            ? t("高风险", "High risk")
            : t("待确认", "Pending")}
        </Pill>
      </div>
      {Object.entries(card.title_args).map(([key, value]) => (
        <p className="pa-untrusted" key={key}>
          {key === "command" ? (
            <code>{value}</code>
          ) : key === "code" ? (
            <strong>{value}</strong>
          ) : (
            value
          )}
        </p>
      ))}
      {card.detail && <p className="pa-untrusted">{card.detail}</p>}
      <div className="pa-row pa-wrap">
        <small>
          {expired
            ? t("已过期，默认拒绝", "Expired. Denied by default.")
            : t("仅在你同意后继续。", "Continues only with your permission.")}
        </small>
        <div className="pa-actions">
          {card.actions.map((action) => (
            <button
              key={action}
              disabled={busy || expired}
              className={action === "deny" ? "" : "pa-primary"}
              onClick={() => onDecide(action)}
            >
              {action === "deny" ? (
                <X size={16} aria-hidden="true" />
              ) : (
                <Check size={16} aria-hidden="true" />
              )}
              {action === "deny"
                ? t("拒绝", "Deny")
                : action === "allow_always"
                  ? t("始终允许", "Always allow")
                  : card.kind === "pairing"
                    ? t("两边一致", "Codes match")
                    : t("允许一次", "Allow once")}
            </button>
          ))}
        </div>
      </div>
    </Card>
  );
}
