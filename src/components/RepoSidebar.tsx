import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { ipc } from "../lib/ipc";
import { copyText, fileManagerName, stateTone } from "../lib/format";
import { useEcho } from "../lib/store";
import type { RepoSummary } from "../lib/types";
import { Button, Dot, PauseIcon, SyncIcon, TerminalIcon } from "./ui";

async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

export function RepoSidebar() {
  const repos = useEcho((s) => s.repos);
  const selectedId = useEcho((s) => s.selectedId);
  const { select, run, showToast } = useEcho.getState();
  const [menu, setMenu] = useState<{ repo: RepoSummary; x: number; y: number } | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  const addRepo = async () => {
    const path = await pickFolder("Add a git repository");
    if (!path) return;
    const repo = await run(() => ipc.addRepo(path));
    if (repo) {
      await useEcho.getState().refreshRepos();
      select(repo.id);
      showToast(`Added ${repo.config.displayName}`);
    }
  };

  return (
    <nav
      aria-label="Repositories"
      className="flex w-56 shrink-0 flex-col border-r border-zinc-200 bg-zinc-100 dark:border-zinc-700 dark:bg-zinc-900"
    >
      <div className="px-3 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Repos</div>
      <ul className="min-h-0 flex-1 overflow-y-auto px-1.5">
        {repos.map((r) => (
          <li key={r.id}>
            {renaming === r.id ? (
              <RenameInput
                initial={r.config.displayName}
                onDone={(name) => {
                  setRenaming(null);
                  if (name && name !== r.config.displayName)
                    void run(() => ipc.updateRepoConfig(r.id, { displayName: name }));
                }}
              />
            ) : (
              <button
                onClick={() => select(r.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setMenu({ repo: r, x: e.clientX, y: e.clientY });
                }}
                title={r.config.path}
                className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] ${
                  r.id === selectedId
                    ? "bg-blue-600 text-white"
                    : "text-zinc-800 hover:bg-zinc-200 dark:text-zinc-200 dark:hover:bg-zinc-800"
                } ${r.available ? "" : "opacity-60"}`}
              >
                <span className="flex w-2 justify-center">
                  {r.hasUnseen && <Dot tone="accent" title="Unseen activity" className={r.id === selectedId ? "!bg-white" : ""} />}
                </span>
                <span className="min-w-0 flex-1 truncate">{r.config.displayName}</span>
                {!r.available && <span className="text-[10px] uppercase">missing</span>}
                {r.available && <RepoDevIcon repo={r} selected={r.id === selectedId} />}
                {r.available && r.sync.branch && r.sync.state.kind !== "off" && (
                  <RepoSyncIcon repo={r} selected={r.id === selectedId} />
                )}
              </button>
            )}
          </li>
        ))}
      </ul>
      <div className="border-t border-zinc-200 p-2 dark:border-zinc-700">
        <Button className="w-full justify-center" onClick={addRepo}>
          + Add repo
        </Button>
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            { label: "Rename", onClick: () => setRenaming(menu.repo.id) },
            { label: `Open in ${fileManagerName}`, onClick: () => run(() => ipc.openInFileManager(menu.repo.id)) },
            { label: "Open in terminal", onClick: () => run(() => ipc.openInTerminal(menu.repo.id)) },
            {
              label: "Copy path",
              onClick: async () => {
                await copyText(menu.repo.config.path);
                showToast("Path copied");
              },
            },
            {
              label: "Repo settings…",
              onClick: () => {
                select(menu.repo.id);
                useEcho.getState().openSettings("repo");
              },
            },
            {
              label: "Remove",
              danger: true,
              onClick: () =>
                useEcho
                  .getState()
                  .setPrompt({ kind: "removeRepo", repoId: menu.repo.id, name: menu.repo.config.displayName }),
            },
          ]}
        />
      )}
    </nav>
  );
}

/** Shown while a repo's dev server runs (green) or after it crashed (red). */
function RepoDevIcon({ repo, selected }: { repo: RepoSummary; selected: boolean }) {
  const s = repo.devServer;
  if (s.kind === "running" || s.kind === "stopping") {
    return <TerminalIcon title="Dev server running" className={selected ? "text-white" : "text-green-600"} />;
  }
  if (s.kind === "exited" && !s.success && !s.stoppedByUser) {
    return (
      <span className={`flex rounded-sm ${selected ? "bg-white p-px" : ""}`}>
        <TerminalIcon title="Dev server exited with an error" className="text-red-600" />
      </span>
    );
  }
  return null;
}

function RepoSyncIcon({ repo, selected }: { repo: RepoSummary; selected: boolean }) {
  const tone = stateTone(repo.sync.state);
  const color = selected
    ? "text-white"
    : tone === "warn"
      ? "text-amber-600"
      : tone === "error"
        ? "text-red-600"
        : tone === "muted"
          ? "text-zinc-400"
          : "text-green-600";
  if (repo.sync.state.kind === "paused") {
    return <PauseIcon title={`Paused on ${repo.sync.branch}`} className={selected ? "text-white/80" : "text-zinc-400"} />;
  }
  return <SyncIcon title={`Syncing ${repo.sync.branch}`} className={color} />;
}

function RenameInput({ initial, onDone }: { initial: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <input
      autoFocus
      aria-label="Repo name"
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => onDone(value.trim() || null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone(value.trim() || null);
        if (e.key === "Escape") onDone(null);
      }}
      className="my-0.5 w-full rounded-md border border-blue-500 bg-white px-2 py-1 text-[13px] outline-none dark:bg-zinc-800"
    />
  );
}

interface MenuItem {
  label: string;
  danger?: boolean;
  onClick: () => void;
}

function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const ref = useRef<HTMLUListElement>(null);
  useEffect(() => {
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [onClose]);
  return (
    <ul
      ref={ref}
      role="menu"
      style={{ left: x, top: y }}
      className="fixed z-50 min-w-44 rounded-md border border-zinc-300 bg-white py-1 text-[13px] shadow-lg dark:border-zinc-600 dark:bg-zinc-800"
    >
      {items.map((it) => (
        <li key={it.label}>
          <button
            role="menuitem"
            className={`block w-full px-3 py-1 text-left hover:bg-blue-600 hover:text-white ${it.danger ? "text-red-600" : ""}`}
            onClick={() => {
              onClose();
              it.onClick();
            }}
          >
            {it.label}
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Shown in place of the branch list when a repo's folder is gone. */
export function UnavailableRepo({ repo }: { repo: RepoSummary }) {
  const { run, setPrompt } = useEcho.getState();
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
      <div className="text-sm font-medium">Can't find {repo.config.displayName}</div>
      <p className="max-w-sm text-xs text-zinc-500">
        <code className="break-all">{repo.config.path}</code> was moved or deleted. Echo has stopped watching it.
      </p>
      <div className="flex gap-2">
        <Button
          variant="primary"
          onClick={async () => {
            const path = await pickFolder(`Locate ${repo.config.displayName}`);
            if (path) await run(() => ipc.locateRepo(repo.id, path));
          }}
        >
          Locate…
        </Button>
        <Button onClick={() => setPrompt({ kind: "removeRepo", repoId: repo.id, name: repo.config.displayName })}>
          Remove
        </Button>
      </div>
    </div>
  );
}
