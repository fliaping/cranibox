import { ArrowLeftRight, ArrowRight, Crown, Handshake, ServerCog, UserRound, type LucideIcon } from "lucide-react";
import type { Device, PairPurpose, Relation } from "../personal/api";
import "./device-graph.css";

type Translate = (zh: string, en: string) => string;

/** Older nodes send only `guest`. */
export function relationOf(device: Pick<Device, "relation" | "guest">): Relation {
  return device.relation || (device.guest ? "guest" : "peer");
}
export const relationIcons: Record<Relation, LucideIcon> = {
  peer: ArrowLeftRight, controlled: ServerCog, controller: Crown, guest: UserRound, host: Handshake,
};
/** CSS tone per relation; matches the edge colours of the device graph. */
export const relationTones: Record<Relation, string> = {
  peer: "info", controlled: "accent", controller: "warn", guest: "guest", host: "shared",
};
export function relationLabel(relation: Relation, t: Translate) {
  return ({
    peer: t("对等设备", "Peer device"),
    controlled: t("受控设备", "Controlled device"),
    controller: t("控制这台设备", "Controls this device"),
    guest: t("访客", "Guest"),
    host: t("共享给我", "Shared with me"),
  } as const)[relation];
}
/** One sentence a person can act on: who may use whom. `name` is the device described;
 * `others` the devices on the other end (its controllers, what it uses, who uses it). */
export function relationSentence(relation: Relation, name: string, t: Translate, others: string[] = []) {
  const list = (fallbackZh: string, fallbackEn: string) => others.length ? others.join(t("、", ", ")) : t(fallbackZh, fallbackEn);
  return ({
    peer: () => t(`${name} 是你的设备：双方可以互相使用，共享设备名单和模型订阅。`, `${name} is one of your devices: each can use the other, and they share the device list and model subscriptions.`),
    controlled: () => { const by = list("你的设备", "Your devices"); return t(`${by} 可以在 ${name} 上执行任务；${name} 不能访问你的任何设备，也看不到你的设备名单。`, `${by} can run tasks on ${name}. ${name} cannot reach any of your devices or see your device list.`); },
    controller: () => { const on = list("这台设备", "this device"); return t(`${name} 可以在 ${on} 上执行命令、打开终端和管理项目；反过来不行。`, `${name} can run commands, open terminals and manage projects on ${on}, not the other way round.`); },
    guest: () => { const on = list("你的设备", "your device"); return t(`${name} 只能使用 ${on} 上你逐项允许的功能；你的设备不能访问 ${name}。`, `${name} can use only what you allow on ${on}, one capability at a time. Your devices cannot reach ${name}.`); },
    host: () => { const by = list("你的设备", "Your devices"); return t(`${by} 可以使用 ${name} 允许的功能；${name} 不能访问你的设备。`, `${by} can use what ${name} allows. ${name} cannot reach your devices.`); },
  } as const)[relation]();
}

/** What each pairing choice means, from the inviting device. */
export function purposeInfo(purpose: PairPurpose, t: Translate) {
  return ({
    owner_device: {
      title: t("对等设备", "Peer device"), kicker: t("我的另一台设备", "Another device of mine"),
      from: t("这台", "This"), to: t("新设备", "New"), both: true,
      text: t("加入你的身份：共享设备名单和模型订阅，可以互相打开终端、桌面和浏览器。只用于你自己、信任的设备。", "Joins your identity: shares the device list and model subscriptions, and each can open the other's terminal, desktop and browser. Only for devices you own and trust."),
    },
    worker: {
      title: t("受控设备", "Controlled device"), kicker: "Worker",
      from: t("这台", "This"), to: t("新设备", "New"), both: false,
      text: t("这台设备可以在新设备上执行命令、打开终端和管理项目；新设备不能访问你的任何设备。适合不完全信任的执行节点。", "This device can run commands, open terminals and manage projects on the new one. The new device cannot reach any of your devices. For workers you do not fully trust."),
    },
    guest: {
      title: t("访客", "Guest"), kicker: t("别人的设备", "Someone else's device"),
      from: t("新设备", "New"), to: t("这台", "This"), both: false,
      text: t("新设备只能使用这台设备上你逐项允许的功能，不能使用模型订阅；这台设备不能访问它。", "The new device can use only what you allow on this device, never your model subscriptions. This device cannot reach it."),
    },
  } as const)[purpose];
}
/** What the device that opened an invitation is agreeing to. `other` is the inviting device. */
export function joinerInfo(purpose: PairPurpose, other: string, t: Translate) {
  return ({
    owner_device: {
      title: t(`加入「${other}」所在的身份`, `Join ${other}'s identity`), from: other, to: t("这台", "This"), both: true, warn: false,
      text: t("这台设备会成为对等设备：共享设备名单和模型订阅，可以互相使用。只在两台都是你的设备时确认。", "This device becomes a peer: it shares the device list and model subscriptions, and each can use the other. Confirm only if both devices are yours."),
    },
    worker: {
      title: t(`让「${other}」控制这台设备`, `Let ${other} control this device`), from: other, to: t("这台", "This"), both: false, warn: true,
      text: t(`确认后，${other} 可以在这台设备上执行命令、打开终端和管理项目。这台设备不能访问 ${other}。`, `Once confirmed, ${other} can run commands, open terminals and manage projects on this device. This device cannot reach ${other}.`),
    },
    guest: {
      title: t(`成为「${other}」的访客`, `Become a guest of ${other}`), from: t("这台", "This"), to: other, both: false, warn: false,
      text: t(`这台设备可以使用 ${other} 允许的功能；${other} 不能访问这台设备。`, `This device can use what ${other} allows. ${other} cannot reach this device.`),
    },
  } as const)[purpose];
}
/** "A ⇄ B" or "A → B" as two chips and an arrow. */
export function Direction({ from, to, both }: { from: string; to: string; both: boolean }) {
  const Arrow = both ? ArrowLeftRight : ArrowRight;
  return <span className="cx-direction"><span>{from}</span><Arrow aria-hidden="true" /><span className="cx-sr">{both ? "⇄" : "→"}</span><span>{to}</span></span>;
}
/** Name of the device a hop goes through, from a `device:<node>` path. */
export function hopName(via: string | null | undefined, deviceName: (node: string) => string) {
  return via?.startsWith("device:") ? deviceName(via.slice("device:".length)) : null;
}
