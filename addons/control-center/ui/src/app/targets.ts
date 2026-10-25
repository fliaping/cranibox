import { useCallback, useMemo } from "react";
import type { Device } from "../personal/api";
import { directTarget, terminalTarget, useTerminalEnvironment, type TerminalTarget } from "../terminal/TerminalEnvironment";
import { useWorkspace } from "../workspace";
import { isThis } from "./ui";
import { useApp } from "./store";

/** One label for every terminal surface, including saved targets and route aliases. */
export function terminalDeviceName(id: string, nodes: TerminalTarget[], devices: Device[]) {
  const node = nodes.find(node => node.id === id) || nodes.find(node => node.aliases?.includes(id));
  const identities = [id, node?.id, ...(node?.aliases || [])].filter((value): value is string => value !== undefined).map(value => terminalTarget(value).node);
  const device = devices.find(device => identities.includes(device.node_id) || !id && isThis(device));
  if (device?.name.trim() && device.name !== device.node_id) return device.name;
  const ownName = node?.name.trim();
  if (ownName && !identities.includes(ownName)) return ownName;
  const raw = terminalTarget(node?.id || id).node || '';
  return raw.replace(/^(node-[a-f0-9]{8})[a-f0-9]+$/i, '$1…');
}

export function useTerminalDevices() {
  const { devices, access } = useApp();
  const { targets, localNode } = useTerminalEnvironment();
  const nodes = useMemo(() => {
    const result = targets.map(node => {
      const id = terminalTarget(node.id).node;
      const device = devices.find(device => device.node_id === id || node.aliases?.includes(device.node_id));
      return { ...node, available: node.available && (!device || isThis(device) || device.online) };
    });
    for (const device of devices) {
      const id = isThis(device) ? localNode || '' : directTarget(device.node_id);
      if (!id || result.some(node => node.id === id || node.aliases?.includes(device.node_id))) continue;
      result.push({ id, aliases: [device.node_id], name: device.node_id, available: device.online });
    }
    return result;
  }, [targets, devices, localNode]);
  return { nodes, displayName: (id: string) => {
    const node = terminalTarget(id).node;
    const device = devices.find(device => device.node_id === node);
    if (device?.name.trim() && device.name !== node) return device.name;
    const records = [...(access?.peers || []).map(peer => ({ from: peer.node, name: peer.name })), ...(access?.pending || []), ...(access?.sessions || [])];
    const name = records.find(record => record.from === node && record.name.trim() && record.name !== node)?.name;
    return name || terminalDeviceName(id, nodes, devices);
  } };
}

/** Terminal routes: this device uses the local engine, every other peer its direct route. */
export function useTerminalNode() {
  const { snapshot } = useWorkspace();
  const local = snapshot?.local_node?.id || "";
  return useCallback((device: Device) => (isThis(device) ? local : directTarget(device.node_id)), [local]);
}
