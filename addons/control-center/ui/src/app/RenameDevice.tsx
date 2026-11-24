import { Pencil } from "lucide-react";
import { useMemo, useState } from "react";
import type { Device } from "../personal/api";
import { useApp } from "./store";
import { formatDeviceLabel, isThis, Sheet } from "./ui";

const NAME_MAX = 64;

function normalize(value: string) {
  return value.trim();
}

/** Rename control used on the device list and the device page. */
export function RenameDevice({ device }: { device: Device }) {
  const { t } = useApp();
  const [open, setOpen] = useState(false);
  const info = formatDeviceLabel(device.name, device.node_id, t);
  return <>
    <button type="button" className="cx-icon-btn cx-rename-btn" aria-label={t("重命名 " + info.label, "Rename " + info.label)} onClick={event => { event.preventDefault(); event.stopPropagation(); setOpen(true); }}>
      <Pencil size={15} aria-hidden="true" />
    </button>
    {open && <RenameSheet device={device} onClose={() => setOpen(false)} />}
  </>;
}

function RenameSheet({ device, onClose }: { device: Device; onClose: () => void }) {
  const { t, api, run, busy, devices, setNotice } = useApp();
  const [value, setValue] = useState(device.name);
  const name = normalize(value);
  const tooLong = [...name].length > NAME_MAX;
  const invalid = tooLong || [...name].some(char => char.charCodeAt(0) < 32);
  const duplicate = useMemo(() => {
    if (!name) return false;
    return devices.some(other => other.node_id !== device.node_id && other.name.trim().toLowerCase() === name.toLowerCase());
  }, [devices, device.node_id, name]);
  const local = isThis(device);
  const save = () => {
    if (invalid) return;
    void run(async () => {
      const result = await api.call<{ name?: string | null; cleared?: boolean; duplicate?: boolean }>(
        "PUT",
        "/devices/" + encodeURIComponent(device.node_id),
        { name },
      );
      const shown = result?.name || name;
      const duplicateNote = result?.duplicate ? t("另有一台设备也叫这个名字，可以用设备 ID 区分它们。", " Another device already uses this name; you can tell them apart by device ID.") : "";
      setNotice(result?.cleared
        ? t("已恢复为设备原来的名称。", "The device will use its own name again.") + duplicateNote
        : t("已将设备重命名为“" + shown + "”。", "Renamed this device to “" + shown + "”.") + duplicateNote);
      onClose();
    });
  };
  return <Sheet title={t("重命名设备", "Rename device")} onClose={onClose} t={t}>
    <form onSubmit={event => { event.preventDefault(); save(); }}>
      <label className="cx-field">{t("名称", "Name")}
        <input autoFocus maxLength={NAME_MAX} value={value} disabled={busy} autoComplete="off" spellCheck={false} onChange={event => setValue(event.target.value)} />
      </label>
      <p className="cx-muted cx-small">{local
        ? t("这是这台设备自己使用的名称，你的其他设备也会看到。", "This is the name this device uses for itself. Your other devices will see it too.")
        : t("你给这台设备起的名称，会同步到你的其他设备。不会改变它对自己的称呼。", "A name you give this device. It syncs to your other devices, and does not change what the device calls itself.")}
        {" "}{t("留空则使用设备原来的名称。", "Leave empty to use the device's own name.")}</p>
      {invalid && <p className="cx-form-error" role="alert">{tooLong ? t("最多 64 个字符。", "Use at most 64 characters.") : t("名称不能包含控制字符。", "The name cannot contain control characters.")}</p>}
      {!invalid && duplicate && <p className="cx-form-warn" role="status">{t("另有一台设备也叫“" + name + "”。仍然可以保存，之后可以用设备 ID 区分它们。", "Another device is already named “" + name + "”. You can still save; tell them apart by device ID.")}</p>}
      <div className="cx-actions">
        <button type="button" className="cx-btn" disabled={busy} onClick={onClose}>{t("取消", "Cancel")}</button>
        <button type="submit" className="cx-btn primary" disabled={busy || invalid}>{busy ? t("正在保存…", "Saving…") : name ? t("保存", "Save") : t("恢复原名", "Use original name")}</button>
      </div>
    </form>
  </Sheet>;
}
