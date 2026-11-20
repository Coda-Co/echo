import { shortSha } from "../lib/format";
import type { CommitSummary, SyncState } from "../lib/types";
import { Button, FileList } from "./ui";

export interface BlockedActions {
  discard: (files: string[]) => void;
  moveAside: (paths: string[]) => void;
  resetToRemote: (detail: string) => void;
  retry: () => void;
  syncInstead: (branch: string) => void;
  switchBack: () => void;
  stop: () => void;
}

/** Inline explanation and one-click fix for every blocked / attention state. */
export function BlockedPanel({ state, branch, actions }: { state: SyncState; branch: string; actions: BlockedActions }) {
  switch (state.kind) {
    case "blockedDirty":
      return (
        <Panel tone="warn">
          {state.files.length > 0 && (
            <>
              <p>
                {state.files.length} tracked file{state.files.length === 1 ? " has" : "s have"} local changes, so new
                commits can't be pulled.
              </p>
              <FileList files={state.files} />
              {state.crlfHint && (
                <p className="text-[11px] text-zinc-600 dark:text-zinc-400">
                  These files differ only in line endings. That usually means <code>core.autocrlf</code> differs from
                  what the repo expects; check your git config before discarding.
                </p>
              )}
            </>
          )}
          {state.untracked.length > 0 && (
            <>
              <p>
                {state.untracked.length} untracked file{state.untracked.length === 1 ? " is" : "s are"} in the way of the
                incoming commits.
              </p>
              <FileList files={state.untracked} />
            </>
          )}
          <Row>
            {state.files.length > 0 && (
              <Button variant="danger" onClick={() => actions.discard(state.files)}>
                Discard and continue
              </Button>
            )}
            {state.untracked.length > 0 && (
              <Button variant="primary" onClick={() => actions.moveAside(state.untracked)}>
                Move files aside
              </Button>
            )}
          </Row>
        </Panel>
      );
    case "blockedLocalCommits":
      return (
        <Panel tone="warn">
          <p>
            Your local <code>{branch}</code> has {plural(state.commits.length, "commit")} that aren't on the remote.
          </p>
          <Commits commits={state.commits} />
          <Row>
            <Button variant="danger" onClick={() => actions.resetToRemote(`${plural(state.commits.length, "local commit")} will be discarded.`)}>
              Reset to remote
            </Button>
          </Row>
        </Panel>
      );
    case "blockedDiverged":
      return (
        <Panel tone="warn">
          <p>
            The remote <code>{branch}</code> was rewritten, and your local branch has its own commits.
          </p>
          <Commits commits={state.commits} />
          <Row>
            <Button variant="danger" onClick={() => actions.resetToRemote(`${plural(state.commits.length, "local commit")} will be discarded.`)}>
              Reset to remote
            </Button>
          </Row>
        </Panel>
      );
    case "blockedForcePush":
      return (
        <Panel tone="warn">
          <p>
            <code>{branch}</code> was force-pushed. Your checkout has no commits of its own.
          </p>
          <Row>
            <Button variant="primary" onClick={() => actions.resetToRemote(`The checkout will move to ${shortSha(state.remoteSha)}.`)}>
              Reset to {shortSha(state.remoteSha)}
            </Button>
          </Row>
        </Panel>
      );
    case "remoteBranchDeleted":
      return (
        <Panel tone="muted">
          <p>
            <code>{branch}</code> no longer exists on the remote. It was probably merged. Pick another branch below.
          </p>
          <Row>
            <Button onClick={actions.stop}>Stop syncing</Button>
          </Row>
        </Panel>
      );
    case "paused": {
      if (state.reason.kind !== "checkoutChanged") return null;
      const current = state.reason.current;
      return (
        <Panel tone="muted">
          <p>
            The checkout was switched outside Echo
            {current ? (
              <>
                {" "}
                to <code>{current}</code>
              </>
            ) : (
              " (detached HEAD)"
            )}
            . Sync is paused.
          </p>
          <Row>
            {current && (
              <Button variant="primary" onClick={() => actions.syncInstead(current)}>
                Sync {current} instead
              </Button>
            )}
            <Button onClick={actions.switchBack}>Switch back to {branch}</Button>
          </Row>
        </Panel>
      );
    }
    case "error":
      return (
        <Panel tone="error">
          <p>
            {state.error.kind === "auth"
              ? "Git couldn't authenticate with origin. Check your credentials (try a git fetch in a terminal), then retry."
              : state.error.kind === "network"
                ? "Can't reach origin. Echo will keep retrying every minute."
                : state.error.message}
          </p>
          {state.error.kind !== "network" && state.error.kind !== "auth" && (
            <p className="font-mono text-[11px] text-zinc-500">{state.error.kind}</p>
          )}
          <Row>
            <Button onClick={actions.retry}>Retry</Button>
          </Row>
        </Panel>
      );
    default:
      return null;
  }
}

function plural(n: number, word: string) {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function Panel({ tone, children }: { tone: "warn" | "error" | "muted"; children: React.ReactNode }) {
  const tones = {
    warn: "border-amber-300 bg-amber-50 dark:border-amber-800 dark:bg-amber-950/40",
    error: "border-red-300 bg-red-50 dark:border-red-900 dark:bg-red-950/40",
    muted: "border-zinc-300 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-800/50",
  };
  return (
    <div data-testid="blocked-panel" className={`mt-2 space-y-2 rounded-md border p-2.5 text-xs ${tones[tone]}`}>
      {children}
    </div>
  );
}

function Row({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-wrap gap-2 pt-1">{children}</div>;
}

function Commits({ commits }: { commits: CommitSummary[] }) {
  return (
    <ul className="max-h-32 overflow-y-auto rounded border border-zinc-200 bg-white px-2 py-1 text-[11px] dark:border-zinc-700 dark:bg-zinc-900">
      {commits.map((c) => (
        <li key={c.sha} className="truncate py-px">
          <span className="font-mono text-zinc-500">{shortSha(c.sha)}</span> {c.subject}
        </li>
      ))}
    </ul>
  );
}
