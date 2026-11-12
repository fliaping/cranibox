import { CornerDownLeft, Search, type LucideIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Translate } from "./ui";

/** One thing the palette can do. `words` adds search terms that are not shown. */
export type Command = { id: string; group: string; label: string; hint?: string; words?: string; icon: LucideIcon; run: () => void };

function matches(command: Command, query: string) {
  const text = (command.label + " " + (command.hint || "") + " " + (command.words || "") + " " + command.group).toLowerCase();
  return query.toLowerCase().split(/\s+/).filter(Boolean).every(word => text.includes(word));
}

/** Search and run: every page, device and frequent action from the keyboard. */
export function Palette({ commands, onClose, t }: { commands: Command[]; onClose: () => void; t: Translate }) {
  const dialog = useRef<HTMLDialogElement>(null), list = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState(""), [active, setActive] = useState(0);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const element = dialog.current, restore = document.activeElement as HTMLElement | null;
    element?.showModal();
    return () => { element?.close(); if (restore?.isConnected) restore.focus(); };
  }, []);
  const found = useMemo(() => commands.filter(command => matches(command, query)), [commands, query]);
  const current = Math.min(active, Math.max(0, found.length - 1));
  useEffect(() => { list.current?.querySelector("[aria-selected=true]")?.scrollIntoView({ block: "nearest" }); }, [current, query]);
  const run = (command?: Command) => { if (!command) return; close.current(); command.run(); };
  const groups: [string, Command[]][] = [];
  for (const command of found) {
    const group = groups.find(entry => entry[0] === command.group);
    if (group) group[1].push(command); else groups.push([command.group, [command]]);
  }
  return <dialog ref={dialog} className="cx-palette" aria-label={t("搜索与跳转", "Search and jump")}
    onCancel={event => { event.preventDefault(); close.current(); }}
    onClick={event => { if (event.target === event.currentTarget) close.current(); }}>
    <div className="cx-palette-input">
      <Search size={18} aria-hidden="true" />
      <input autoFocus role="combobox" aria-expanded="true" aria-controls="cx-palette-list" aria-activedescendant={found[current] ? "cx-palette-" + found[current].id : undefined}
        aria-label={t("搜索设备、页面或操作", "Search devices, pages or actions")} placeholder={t("搜索设备、页面或操作…", "Search devices, pages or actions…")}
        value={query} onChange={event => { setQuery(event.target.value); setActive(0); }}
        onKeyDown={event => {
          if (event.key === "ArrowDown") { event.preventDefault(); setActive(found.length ? (current + 1) % found.length : 0); }
          else if (event.key === "ArrowUp") { event.preventDefault(); setActive(found.length ? (current - 1 + found.length) % found.length : 0); }
          else if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); run(found[current]); }
        }} />
      <kbd>Esc</kbd>
    </div>
    <div ref={list} id="cx-palette-list" className="cx-palette-list" role="listbox" aria-label={t("结果", "Results")}>
      {groups.map(([group, items]) => <div key={group} role="group" aria-label={group}>
        <div className="cx-palette-group" aria-hidden="true">{group}</div>
        {items.map(command => {
          const selected = found[current] === command;
          return <div key={command.id} id={"cx-palette-" + command.id} role="option" aria-selected={selected} className="cx-palette-item"
            onMouseMove={() => { if (!selected) setActive(found.indexOf(command)); }} onClick={() => run(command)}>
            <command.icon size={17} aria-hidden="true" /><span className="cx-grow">{command.label}</span>
            {command.hint && <small>{command.hint}</small>}{selected && <CornerDownLeft size={14} aria-hidden="true" />}
          </div>;
        })}
      </div>)}
      {!found.length && <p className="cx-palette-empty">{t("没有匹配的结果", "Nothing matches")}</p>}
    </div>
  </dialog>;
}
