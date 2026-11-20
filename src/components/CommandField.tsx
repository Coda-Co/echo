import { useEffect, useRef, useState } from "react";
import { isMac, relativeTime } from "../lib/format";
import type { DevCommands } from "../lib/types";
import { PinIcon } from "./ui";

interface Props {
  value: string;
  onChange: (v: string) => void;
  /** Start the server with this command. */
  onSubmit: (command: string) => void;
  commands: DevCommands;
  onPin: (command: string, name: string) => void;
  onUnpin: (command: string) => void;
  now: number;
  autoFocus?: boolean;
}

type Item = { kind: "pinned"; name: string; command: string } | { kind: "recent"; command: string; at: number };

const mod = isMac ? "⌘" : "Ctrl+";

/**
 * The dev server command, editable before each start, with a dropdown of
 * pinned and recent commands. ↑/↓ browse, Enter fills the field (or starts
 * when nothing is highlighted), ⌘/Ctrl+Enter starts straight away.
 */
export function CommandField({ value, onChange, onSubmit, commands, onPin, onUnpin, now, autoFocus }: Props) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const [naming, setNaming] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const pinnedSet = new Set(commands.pinned.map((p) => p.command));
  const items: Item[] = [
    ...commands.pinned.map((p) => ({ kind: "pinned" as const, ...p })),
    ...commands.recent.map((r) => ({ kind: "recent" as const, ...r })),
  ];
  const hasItems = items.length > 0;

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  useEffect(() => {
    if (!open) {
      setActive(-1);
      setNaming(null);
    }
  }, [open]);

  const fill = (command: string) => {
    onChange(command);
    setOpen(false);
    inputRef.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!hasItems) return;
      e.preventDefault();
      if (!open) {
        setOpen(true);
        setActive(e.key === "ArrowDown" ? 0 : items.length - 1);
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((a) => (a + step + items.length) % items.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const highlighted = open && active >= 0 ? items[active] : null;
      if (e.metaKey || e.ctrlKey) {
        setOpen(false);
        onSubmit(highlighted?.command ?? value);
      } else if (highlighted) {
        fill(highlighted.command);
      } else {
        onSubmit(value);
      }
    } else if (e.key === "Escape" && open) {
      e.preventDefault();
      setOpen(false);
    }
  };

  const groupStart = (i: number) => i === 0 || items[i - 1].kind !== items[i].kind;

  return (
    <div ref={rootRef} className="relative min-w-0 flex-1">
      <div className="flex h-7 items-center gap-1.5 rounded-md border border-zinc-300 bg-white pl-2 pr-0.5 focus-within:border-blue-500 focus-within:ring-2 focus-within:ring-blue-500/20 dark:border-zinc-600 dark:bg-zinc-800">
        <span aria-hidden className="text-xs text-zinc-400">
          $
        </span>
        <input
          ref={inputRef}
          autoFocus={autoFocus}
          aria-label="Dev server command"
          role="combobox"
          aria-expanded={open}
          aria-controls="dev-command-list"
          aria-activedescendant={open && active >= 0 ? `dev-command-${active}` : undefined}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          placeholder="pnpm dev"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          className="min-w-0 flex-1 bg-transparent font-mono text-xs outline-none placeholder:text-zinc-400"
        />
        {hasItems && (
          <button
            type="button"
            aria-label={open ? "Hide recent commands" : "Show recent commands"}
            title="Pinned and recent commands (↑↓)"
            onClick={() => {
              setOpen((o) => !o);
              inputRef.current?.focus();
            }}
            className="flex h-6 w-6 items-center justify-center rounded text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-700"
          >
            <svg aria-hidden width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <path d={open ? "M3 7.5 6 4.5l3 3" : "M3 4.5 6 7.5l3-3"} />
            </svg>
          </button>
        )}
      </div>

      {open && hasItems && (
        <div className="absolute left-0 right-0 top-8 z-30 overflow-hidden rounded-lg border border-zinc-300 bg-white py-1 text-xs shadow-xl dark:border-zinc-600 dark:bg-zinc-800">
          <ul id="dev-command-list" role="listbox" aria-label="Dev server commands" className="max-h-64 overflow-y-auto">
            {items.map((it, i) => (
              <li key={`${it.kind}:${it.command}`} role="presentation">
                {groupStart(i) && (
                  <div
                    role="presentation"
                    className={`px-2.5 pb-0.5 pt-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-zinc-500 ${
                      i > 0 ? "mt-1 border-t border-zinc-200 dark:border-zinc-700" : ""
                    }`}
                  >
                    {it.kind === "pinned" ? "Pinned" : "Recent"}
                  </div>
                )}
                {naming === it.command && it.kind === "recent" ? (
                  <PinNameInput
                    command={it.command}
                    onDone={(name) => {
                      setNaming(null);
                      if (name !== null) onPin(it.command, name);
                      inputRef.current?.focus();
                    }}
                  />
                ) : (
                  <div
                    id={`dev-command-${i}`}
                    role="option"
                    aria-selected={i === active}
                    onMouseEnter={() => setActive(i)}
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => fill(it.command)}
                    className={`group flex cursor-default items-center gap-2 px-2.5 py-1 ${
                      i === active ? "bg-blue-600 text-white" : "text-zinc-800 dark:text-zinc-100"
                    }`}
                  >
                    {it.kind === "pinned" ? (
                      <>
                        <PinIcon filled className={i === active ? "text-white" : "text-blue-600"} />
                        <span className="w-28 shrink-0 truncate font-semibold" title={it.name}>
                          {it.name}
                        </span>
                        <span className={`min-w-0 flex-1 truncate font-mono ${i === active ? "text-blue-100" : "text-zinc-600 dark:text-zinc-300"}`}>
                          {it.command}
                        </span>
                        <RowButton
                          label={`Unpin ${it.name}`}
                          active={i === active}
                          onClick={() => onUnpin(it.command)}
                        >
                          Unpin
                        </RowButton>
                      </>
                    ) : (
                      <>
                        <span className="w-3 shrink-0" />
                        <span className="min-w-0 flex-1 truncate font-mono">{it.command}</span>
                        <span className={`shrink-0 text-[11px] ${i === active ? "text-blue-100" : "text-zinc-500"}`}>
                          {relativeTime(it.at, now)}
                        </span>
                        {!pinnedSet.has(it.command) && (
                          <RowButton label={`Pin ${it.command}`} active={i === active} onClick={() => setNaming(it.command)}>
                            <PinIcon />
                            Pin
                          </RowButton>
                        )}
                      </>
                    )}
                  </div>
                )}
              </li>
            ))}
          </ul>
          <div className="mt-1 border-t border-zinc-100 px-2.5 pb-0.5 pt-1.5 text-[11px] text-zinc-500 dark:border-zinc-700">
            ↑↓ browse · Enter fills · {mod}Enter starts
          </div>
        </div>
      )}
    </div>
  );
}

function RowButton({
  label,
  active,
  onClick,
  children,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onMouseDown={(e) => e.preventDefault()}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={`inline-flex shrink-0 items-center gap-1 rounded border px-1.5 text-[11px] ${
        active
          ? "border-white/50 text-white hover:bg-white/15"
          : "border-zinc-300 text-zinc-600 opacity-0 hover:bg-zinc-100 group-hover:opacity-100 dark:border-zinc-600 dark:text-zinc-300 dark:hover:bg-zinc-700"
      }`}
    >
      {children}
    </button>
  );
}

function PinNameInput({ command, onDone }: { command: string; onDone: (name: string | null) => void }) {
  const [name, setName] = useState("");
  return (
    <div className="flex items-center gap-2 px-2.5 py-1">
      <PinIcon className="text-blue-600" />
      <input
        autoFocus
        aria-label="Name for this command"
        placeholder="Name, e.g. all agents"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            e.preventDefault();
            onDone(name.trim() || command);
          }
          if (e.key === "Escape") {
            e.preventDefault();
            onDone(null);
          }
        }}
        onBlur={() => onDone(null)}
        className="h-6 w-36 rounded border border-blue-500 bg-white px-1.5 text-xs outline-none dark:bg-zinc-900"
      />
      <span className="min-w-0 flex-1 truncate font-mono text-zinc-500">{command}</span>
    </div>
  );
}
