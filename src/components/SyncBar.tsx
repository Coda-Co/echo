import { useState } from "react";
import { ipc } from "../lib/ipc";
import { copyText, relativeTime, shortSha, stateLabel, stateTone, useNow } from "../lib/format";
import { useEcho } from "../lib/store";
import type { ActiveFlag, FlagKind, RepoSummary, SyncEvent } from "../lib/types";
import { BlockedPanel, type BlockedActions } from "./BlockedPanel";
import { Button, Dot, GitHubIcon, PauseIcon, PlayIcon, Spinner } from "./ui";

export function SyncBar({ repo }: { repo: RepoSummary }) {
  const { run, setPrompt, startSync, showToast } = useEcho.getState();
  const lastEvent = useEcho((s) => s.lastEvent[repo.id]);
  const now = useNow(1000);
  const { sync } = repo;
  const branch = sync.branch;
  const id = repo.id;
  const [busy, setBusy] = useState(false);

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    await run(fn);
    setBusy(false);
  };

  if (!branch || sync.state.kind === "off") {
    return (
      <section aria-label="Sync bar" className="border-b border-zinc-200 px-4 py-3 dark:border-zinc-700">
        <div className="flex items-center gap-2 text-sm">
          <Dot tone="muted" />
          <span className="text-zinc-600 dark:text-zinc-300">Not syncing.</span>
          <span className="text-xs text-zinc-500">Click a branch to check it out and keep it synced.</span>
          <GitHubButton repo={repo} />
        </div>
        <Flags repo={repo} />
      </section>
    );
  }

  const state = sync.state;
  const tone = stateTone(state);
  const paused = state.kind === "paused" && state.reason.kind === "user";
  const working = state.kind === "switching" || state.kind === "pulling";

  const actions: BlockedActions = {
    discard: (files) => setPrompt({ kind: "discard", repoId: id, files }),
    moveAside: (paths) =>
      act(async () => {
        const moved = await ipc.moveUntrackedAside(id, paths);
        showToast(`Moved ${moved.length} file${moved.length === 1 ? "" : "s"} aside (renamed to *.echo-<time>)`);
      }),
    resetToRemote: (detail) => setPrompt({ kind: "resetToRemote", repoId: id, branch, detail }),
    retry: () => act(() => ipc.syncNow(id)),
    syncInstead: (b) => startSync(id, b),
    switchBack: () => startSync(id, branch),
    stop: () => act(() => ipc.stopSync(id)),
  };

  return (
    <section aria-label="Sync bar" className="border-b border-zinc-200 px-4 py-3 dark:border-zinc-700">
      <div className="flex items-center gap-2">
        {working ? <Spinner /> : <Dot tone={tone} />}
        <span className="min-w-0 truncate text-sm">
          {paused ? "Paused on" : "Syncing"} <code className="font-mono font-semibold">{branch}</code>
        </span>
        <Button
          variant="ghost"
          aria-label={paused ? "Resume syncing" : "Pause syncing"}
          title={paused ? "Resume syncing" : "Pause syncing (keeps the checkout where it is)"}
          disabled={busy || (!paused && working)}
          onClick={() => act(() => (paused ? ipc.resumeSync(id) : ipc.pauseSync(id)))}
          className="!h-6 shrink-0 !px-1.5"
        >
          {paused ? <PlayIcon /> : <PauseIcon />}
        </Button>
        <GitHubButton repo={repo} />
      </div>
      <div className="mt-0.5 pl-4 text-xs text-zinc-500" data-testid="sync-state">
        {stateLabel(state)}
        {sync.lastChecked && !working && state.kind !== "paused" && <> · checked {relativeTime(sync.lastChecked, now)}</>}
        {sync.head && state.kind === "idle" && <> · {shortSha(sync.head)}</>}
      </div>
      <BlockedPanel state={state} branch={branch} actions={actions} />
      {lastEvent && lastEvent.branch === branch && <LastEvent event={lastEvent} now={now} />}
      <Flags repo={repo} />
    </section>
  );
}

function LastEvent({ event, now }: { event: SyncEvent; now: number }) {
  if (now - event.at > 5 * 60_000) return null;
  let text: string;
  switch (event.kind) {
    case "pulled":
      text = event.reset
        ? `Force-pushed: reset to ${shortSha(event.to)}`
        : `Pulled ${event.count} commit${event.count === 1 ? "" : "s"}: ${event.subject}`;
      break;
    default:
      return null;
  }
  return (
    <div className="mt-0.5 truncate pl-4 text-[11px] text-zinc-500">
      {text} · {relativeTime(event.at, now)}
    </div>
  );
}

const flagText: Record<FlagKind, string> = {
  deps: "Dependencies changed",
  migrations: "Migrations changed",
  env: "Env template changed",
};

function Flags({ repo }: { repo: RepoSummary }) {
  const { run, showToast } = useEcho.getState();
  if (repo.flags.length === 0) return null;
  const command = (f: ActiveFlag) =>
    f.kind === "deps" ? repo.installCommand : f.kind === "migrations" ? (repo.config.migrateCommand ?? null) : null;
  const fallback: Record<FlagKind, string> = { deps: "run install", migrations: "run migrate", env: "check your .env" };

  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {repo.flags.map((f) => {
        const cmd = command(f);
        return (
          <span
            key={f.kind}
            data-testid={`flag-${f.kind}`}
            className="inline-flex items-center overflow-hidden rounded-full border border-amber-300 bg-amber-50 text-[11px] text-amber-900 dark:border-amber-700 dark:bg-amber-950/50 dark:text-amber-100"
          >
            <button
              className="px-2 py-0.5 hover:bg-amber-100 dark:hover:bg-amber-900/60"
              title={`${f.files.slice(0, 8).join("\n")}${cmd ? "\n\nClick to copy the command" : ""}`}
              onClick={async () => {
                if (!cmd) return;
                await copyText(cmd);
                showToast(`Copied “${cmd}”`);
              }}
            >
              ⚠ {flagText[f.kind]}: {cmd ? <code className="font-mono">{cmd}</code> : fallback[f.kind]}
            </button>
            <button
              aria-label={`Dismiss ${flagText[f.kind]}`}
              className="border-l border-amber-300 px-1.5 py-0.5 hover:bg-amber-100 dark:border-amber-700 dark:hover:bg-amber-900/60"
              onClick={() => run(() => ipc.dismissFlag(repo.id, f.kind))}
            >
              ✕
            </button>
          </span>
        );
      })}
    </div>
  );
}

/** Opens the repo on GitHub. Amber when pull requests can't be listed. */
function GitHubButton({ repo }: { repo: RepoSummary }) {
  const prError = useEcho((s) => s.pullRequests[repo.id]?.error ?? null);
  const run = useEcho((s) => s.run);
  if (!repo.githubUrl) return null;
  const label = `Open ${repo.githubUrl.replace("https://", "")} in your browser`;
  return (
    <Button
      variant="ghost"
      aria-label="Open on GitHub"
      title={prError ? `${label}\n\nPull requests unavailable: ${prError}` : label}
      onClick={() => run(() => ipc.openOnGitHub(repo.id))}
      className={`!h-6 ml-auto shrink-0 !px-1.5 ${prError ? "!text-amber-600" : ""}`}
    >
      <GitHubIcon />
    </Button>
  );
}
