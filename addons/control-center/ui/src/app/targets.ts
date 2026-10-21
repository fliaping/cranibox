import { useCallback } from "react";
import type { Device } from "../personal/api";
import { directTarget } from "../terminal/TerminalEnvironment";
import { useWorkspace } from "../workspace";
import { isThis } from "./ui";

/** Terminal routes: this device uses the local engine, every other peer its direct route. */
export function useTerminalNode() {
  const { snapshot } = useWorkspace();
  const local = snapshot?.local_node?.id || "";
  return useCallback((device: Device) => (isThis(device) ? local : directTarget(device.node_id)), [local]);
}
