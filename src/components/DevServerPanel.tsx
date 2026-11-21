import { useEffect, useRef, useState } from "react";
import { ipc } from "../lib/ipc";
import { isMac, portLabel, relativeTime, shortDuration, useNow } from "../lib/format";
import { readLocal, useEcho, writeLocal } from "../lib/store";
import { TERMINAL_BG, terminals } from "../lib/terminals";
import type { DevServerStatus, RepoSummary } from "../lib/types";
import { CommandField } from "./CommandField";
import { Button, ChevronIcon, CopyIcon, RestartIcon, StopIcon } from "./ui";

const HEIGHT_KEY = "echo.devPanelHeight";
const MIN_HEIGHT = 120;

const isLive = (s: DevServerStatus) => s.kind === "running" || s.kind === "stopping";
const isCrash = (s: DevServerStatus) => s.kind === "exited" && !s.success && !s.stoppedByUser;

/** The command to pre-fill: what ran last, else the newest recent, else a guess from package.json. */
export function initialCommand(repo: RepoSummary): string {
  const s = repo.devServer;
  if (s.kind !== "idle") return s.command;
  return repo.config.devCommands?.recent[0]?.command ?? repo.suggestedDevCommand ?? "";
}

/** Dev server for the selected repo: a header with its controls over a terminal. */
export function DevServerPanel({ repo }: { repo: RepoSummary }) {
  const { run, setDevPanelOpen } = useEcho.getState();
  const status = repo.devServer;
  const live = status.kind === "running" || status.kind === "stopping";
  const crashed = isCrash(status);
  const now = useNow(1000);
  const [editing, setEditing] = useState(false);
  const [command, setCommand] = useState(() => initialCommand(repo));
  const [height, setHeight] = useState(() => Number(readLocal(HEIGHT_KEY)) || 280);

  // A different repo: start from its own command.
  useEffect(() => {
    setCommand(initialCommand(repo));
    setEditing(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo.id]);

  // Once a run starts, the field shows that command next time.
  useEffect(() => {
    if (status.kind === "running" || status.kind === "stopping") {
      setEditing(false);
      setCommand(status.command);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [live, repo.id]);

  const [starting, setStarting] = useState(false);
  const start = async (cmd: string) => {
    if (!cmd.trim() || starting) return;
    setCommand(cmd);
    setStarting(true);
    const { cols, rows } = terminals.size(repo.id);
    const ok = await run(() => ipc.devServerStart(repo.id, cmd, cols, rows));
    setStarting(false);
    if (ok) terminals.focus(repo.id);
  };
  const restart = async () => {
    const { cols, rows } = terminals.size(repo.id);
    const ok = await run(() => ipc.devServerRestart(repo.id, cols, rows));
    if (ok) terminals.focus(repo.id);
  };
  const stop = () => run(() => ipc.devServerStop(repo.id));

  const onDragStart = (e: React.PointerEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = height;
    const max = () => Math.max(MIN_HEIGHT, window.innerHeight - 200);
    let h = startH;
    const move = (ev: PointerEvent) => {
      h = Math.min(max(), Math.max(MIN_HEIGHT, startH + startY - ev.clientY));
      setHeight(h);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      writeLocal(HEIGHT_KEY, String(Math.round(h)));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const collapse = (
    <Button
      variant="ghost"
      aria-label="Collapse dev server panel"
      title="Collapse"
      onClick={() => setDevPanelOpen(false)}
      className="!h-6 shrink-0 !px-1.5"
    >
      <ChevronIcon />
    </Button>
  );

  let header: React.ReactNode;
  if (isLive(status)) {
    header = (
      <div className="flex items-center gap-2 border-b border-zinc-200 bg-zinc-50 py-1.5 pl-3 pr-2 dark:border-zinc-700 dark:bg-zinc-900">
        <span className={`h-2 w-2 shrink-0 rounded-full ${status.kind === "stopping" ? "animate-pulse bg-amber-500" : "bg-green-500"}`} />
        <span className="shrink-0 text-xs font-semibold">Dev server</span>
        <code
          title={status.command}
          className="min-w-0 truncate rounded border border-zinc-200 bg-white px-1.5 py-px text-xs text-zinc-700 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-200"
        >
          {status.command}
        </code>
        <span className="shrink-0 whitespace-nowrap text-[11px] text-zinc-500" data-testid="dev-uptime">
          {status.kind === "stopping" ? "stopping…" : `up ${shortDuration(now - status.startedAt)}`}
          {status.url && (
            <>
              {" · "}
              <button
                type="button"
                title={`Open ${status.url} in your browser`}
                onClick={() => run(() => ipc.openDevServerUrl(repo.id))}
                className="text-blue-600 hover:underline dark:text-blue-400"
              >
                {portLabel(status.url)}
              </button>
            </>
          )}
        </span>
        <span className="flex-1" />
        <Button onClick={restart} disabled={status.kind === "stopping"} title="Stop, then start the same command">
          <RestartIcon />
          Restart
        </Button>
        <Button onClick={stop} disabled={status.kind === "stopping"} title="Stop the dev server and everything it started">
          <StopIcon />
          Stop
        </Button>
        {collapse}
      </div>
    );
  } else if (crashed && !editing && status.kind === "exited") {
    header = (
      <div className="flex items-center gap-2 border-b border-red-200 bg-red-50 py-1.5 pl-3 pr-2 dark:border-red-900 dark:bg-red-950/40">
        <span className="h-2 w-2 shrink-0 rounded-full bg-red-500" />
        <span className="shrink-0 text-xs font-semibold">Dev server</span>
        <span className="shrink-0 whitespace-nowrap text-xs text-red-800 dark:text-red-200">
          exited{status.exitCode !== null ? ` with code ${status.exitCode}` : ""} · {relativeTime(status.endedAt, now)}
        </span>
        <code
          title={status.command}
          className="min-w-0 truncate rounded border border-red-200 bg-white px-1.5 py-px text-xs text-zinc-700 dark:border-red-900 dark:bg-zinc-800 dark:text-zinc-200"
        >
          {status.command}
        </code>
        <span className="flex-1" />
        <Button onClick={() => setEditing(true)}>Edit command</Button>
        <Button variant="primary" onClick={restart}>
          <RestartIcon />
          Restart
        </Button>
        {collapse}
      </div>
    );
  } else {
    const label = status.kind === "exited" ? (status.stoppedByUser ? "stopped" : "exited") : "not running";
    header = (
      <div className="flex items-center gap-2 border-b border-zinc-200 bg-zinc-50 py-1.5 pl-3 pr-2 dark:border-zinc-700 dark:bg-zinc-900">
        <span className="h-2 w-2 shrink-0 rounded-full border-[1.5px] border-zinc-400" />
        <span className="shrink-0 text-xs font-semibold">Dev server</span>
        <span className="shrink-0 text-[11px] text-zinc-500">{label}</span>
        <CommandField
          key={repo.id}
          value={command}
          onChange={setCommand}
          onSubmit={start}
          commands={repo.config.devCommands ?? { recent: [], pinned: [] }}
          onPin={(cmd, name) => run(() => ipc.pinDevCommand(repo.id, cmd, name))}
          onUnpin={(cmd) => run(() => ipc.unpinDevCommand(repo.id, cmd))}
          now={now}
          autoFocus={editing}
        />
        <Button
          variant="primary"
          disabled={!command.trim() || starting}
          onClick={() => start(command)}
          title="Run from the repo root"
        >
          <svg aria-hidden width="9" height="9" viewBox="0 0 10 10" fill="currentColor">
            <path d="M2 1.2v7.6l6.5-3.8z" />
          </svg>
          {starting ? "Starting…" : "Start"}
        </Button>
        {editing && (
          <Button variant="ghost" onClick={() => setEditing(false)}>
            Cancel
          </Button>
        )}
        {collapse}
      </div>
    );
  }

  return (
    <section
      aria-label="Dev server"
      style={{ height }}
      className="relative flex shrink-0 flex-col border-t border-zinc-200 dark:border-zinc-700"
    >
      <div
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize dev server panel"
        onPointerDown={onDragStart}
        className="absolute -top-1 left-0 right-0 z-10 h-2 cursor-row-resize"
      />
      {header}
      <TerminalView repoId={repo.id} />
    </section>
  );
}

function TerminalView({ repoId }: { repoId: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    terminals.attach(repoId, el);
    const refit = () => {
      const size = terminals.fit(repoId);
      if (size) void ipc.devServerResize(repoId, size.cols, size.rows).catch(() => {});
    };
    refit();
    const ro = new ResizeObserver(refit);
    ro.observe(el);
    return () => {
      ro.disconnect();
      terminals.detach(repoId);
    };
  }, [repoId]);
  return (
    <div className="group relative min-h-0 flex-1 py-1.5 pl-3 pr-1" style={{ background: TERMINAL_BG }}>
      <div ref={ref} data-testid="dev-terminal" className="h-full w-full" />
      <CopyButton repoId={repoId} />
    </div>
  );
}

const COPY_KEY = "echo.devCopyLines";
/** `null` = everything. */
export const COPY_CHOICES: (number | null)[] = [50, 200, 500, 1000, null];
const choiceLabel = (n: number | null) => (n === null ? "all output" : `last ${n} lines`);

function readCopyChoice(): number | null {
  const v = readLocal(COPY_KEY);
  if (v === "all") return null;
  const n = Number(v);
  return COPY_CHOICES.includes(n) ? n : 200;
}

/**
 * Split button over the terminal: copies the selection if there is one,
 * otherwise the last N lines; the menu picks N (remembered) and copies.
 */
function CopyButton({ repoId }: { repoId: string }) {
  const [choice, setChoice] = useState(readCopyChoice);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [open]);

  const copy = async (n: number | null, preferSelection: boolean) => {
    const done = await terminals.copy(repoId, n ?? undefined, preferSelection);
    const { showToast } = useEcho.getState();
    if (!done) showToast("Nothing to copy");
    else if (done.kind === "selection") showToast("Copied selection");
    else showToast(`Copied ${done.count} line${done.count === 1 ? "" : "s"}`);
  };

  const pick = (n: number | null) => {
    setChoice(n);
    writeLocal(COPY_KEY, n === null ? "all" : String(n));
    setOpen(false);
    void copy(n, false);
  };

  const base = "inline-flex h-6 items-center border-zinc-600 bg-zinc-800/90 text-[11px] text-zinc-200 hover:bg-zinc-700";
  return (
    <div
      ref={rootRef}
      // Keep the terminal's selection when the buttons are pressed.
      onMouseDown={(e) => e.preventDefault()}
      className={`absolute right-4 top-2 z-10 flex transition-opacity focus-within:opacity-100 group-hover:opacity-100 ${
        open ? "opacity-100" : "opacity-0"
      }`}
    >
      <button
        type="button"
        title={`Copies the selection if there is one, otherwise the ${choiceLabel(choice)}. You can also select text and press ${isMac ? "⌘C" : "Ctrl+C"}.`}
        onClick={() => copy(choice, true)}
        className={`${base} gap-1 rounded-l-md border px-2`}
      >
        <CopyIcon />
        Copy {choice === null ? "all" : `last ${choice}`}
      </button>
      <button
        type="button"
        aria-label="Choose how many lines to copy"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={`${base} rounded-r-md border border-l-0 px-1`}
      >
        <ChevronIcon />
      </button>
      {open && (
        <ul
          role="menu"
          aria-label="Copy output"
          className="absolute right-0 top-7 min-w-40 rounded-md border border-zinc-600 bg-zinc-800 py-1 text-[11px] text-zinc-200 shadow-xl"
        >
          {COPY_CHOICES.map((n) => (
            <li key={String(n)} role="none">
              <button
                type="button"
                role="menuitemradio"
                aria-checked={n === choice}
                onClick={() => pick(n)}
                className="flex w-full items-center gap-2 whitespace-nowrap px-2.5 py-1 text-left hover:bg-blue-600 hover:text-white"
              >
                <span className="w-2">{n === choice ? "✓" : ""}</span>
                Copy {choiceLabel(n)}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** One line in the footer while the panel is collapsed. */
export function DevServerStatusLine({ repo }: { repo: RepoSummary }) {
  const now = useNow(15_000);
  const s = repo.devServer;
  const crashed = isCrash(s);
  const dot =
    s.kind === "running"
      ? "bg-green-500"
      : s.kind === "stopping"
        ? "animate-pulse bg-amber-500"
        : crashed
          ? "bg-red-500"
          : "border-[1.5px] border-zinc-400";
  let detail: string;
  switch (s.kind) {
    case "running":
      detail = `· up ${shortDuration(now - s.startedAt)}${s.url ? ` · ${portLabel(s.url)}` : ""}`;
      break;
    case "stopping":
      detail = "· stopping…";
      break;
    case "exited":
      detail = crashed ? `· exited${s.exitCode !== null ? ` with code ${s.exitCode}` : ""}` : "· stopped";
      break;
    default:
      detail = "· not running";
  }
  return (
    <button
      type="button"
      aria-label="Show dev server"
      onClick={() => useEcho.getState().setDevPanelOpen(true)}
      className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 text-left text-xs text-zinc-700 hover:bg-zinc-200/70 dark:text-zinc-200 dark:hover:bg-zinc-700/70"
    >
      <span className={`h-2 w-2 shrink-0 rounded-full ${dot}`} />
      <span className="shrink-0 font-semibold">Dev server</span>
      {s.kind !== "idle" && <span className="min-w-0 truncate font-mono text-zinc-600 dark:text-zinc-300">{s.command}</span>}
      <span className={`shrink-0 whitespace-nowrap ${crashed ? "text-red-700 dark:text-red-300" : "text-zinc-500"}`}>{detail}</span>
      <ChevronIcon up className="shrink-0 text-zinc-500" />
    </button>
  );
}
