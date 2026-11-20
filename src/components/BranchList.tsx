import { useEffect, useMemo, useState } from "react";
import { ipc } from "../lib/ipc";
import { matchesFilter, relativeTime, shortSha, useNow } from "../lib/format";
import { useEcho } from "../lib/store";
import type { BranchFilter, CommitSummary, RepoSummary } from "../lib/types";
import { BranchRow } from "./BranchRow";
import { Button, Spinner } from "./ui";

export function BranchList({ repo }: { repo: RepoSummary }) {
  const snapshot = useEcho((s) => s.branches[repo.id]);
  const { run, startSync } = useEcho.getState();
  const now = useNow(30_000);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const filter = repo.config.branchFilter;

  useEffect(() => setSelected(null), [repo.id]);

  const branches = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (snapshot?.branches ?? []).filter(
      (b) => (b.isSynced || matchesFilter(b.name, filter)) && (!q || b.name.toLowerCase().includes(q)),
    );
  }, [snapshot, filter, query]);

  const hiddenCount = (snapshot?.branches.length ?? 0) - branches.length;
  const anyUnseen = branches.some((b) => !b.deleted && b.status.kind !== "seen");
  const selectedBranch = branches.find((b) => b.name === selected) ?? null;

  const setFilter = (f: BranchFilter) => run(() => ipc.updateRepoConfig(repo.id, { branchFilter: f }));

  const onClick = (name: string, hasDot: boolean) => {
    setSelected(name === selected ? null : name);
    if (hasDot) void run(() => ipc.markSeen(repo.id, name));
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b border-zinc-200 px-3 py-2 dark:border-zinc-700">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Branches</span>
        <FilterPicker filter={filter} onChange={setFilter} />
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search branches"
          aria-label="Search branches"
          className="h-7 min-w-0 flex-1 rounded-md border border-zinc-300 bg-white px-2 text-xs outline-none focus:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800"
        />
        <Button variant="ghost" disabled={!anyUnseen} onClick={() => run(() => ipc.markSeen(repo.id))}>
          Mark all as seen
        </Button>
      </div>

      {snapshot?.error && (
        <div className="border-b border-red-200 bg-red-50 px-3 py-1.5 text-xs text-red-800 dark:border-red-900 dark:bg-red-950/50 dark:text-red-200">
          {snapshot.error.kind === "auth"
            ? "Can't reach origin: check your git credentials, then press Retry in the sync bar."
            : snapshot.error.kind === "network"
              ? "Offline: retrying every minute."
              : snapshot.error.message}
        </div>
      )}

      <div className="flex min-h-0 flex-1">
        <ul role="listbox" aria-label="Branches" className="min-w-0 flex-1 overflow-y-auto">
          {!snapshot && (
            <li className="flex items-center gap-2 p-4 text-xs text-zinc-500">
              <Spinner /> Loading branches…
            </li>
          )}
          {snapshot && branches.length === 0 && (
            <li className="p-6 text-center text-xs text-zinc-500">
              {query
                ? "No branches match your search."
                : filter.mode === "prefix"
                  ? `No branches start with “${filter.prefix}”.`
                  : "This remote has no branches yet."}
              {hiddenCount > 0 && !query && (
                <div className="mt-2">
                  <Button variant="ghost" onClick={() => setFilter({ mode: "all" })}>
                    Show all {snapshot.branches.length} branches
                  </Button>
                </div>
              )}
            </li>
          )}
          {branches.map((b) => (
            <BranchRow
              key={b.name}
              branch={b}
              now={now}
              selected={b.name === selected}
              onClick={() => onClick(b.name, !b.deleted && b.status.kind !== "seen")}
              onDoubleClick={() => !b.deleted && !b.isSynced && startSync(repo.id, b.name)}
            />
          ))}
        </ul>
        {selectedBranch && (
          <BranchDetail
            repoId={repo.id}
            name={selectedBranch.name}
            sha={selectedBranch.sha}
            isSynced={selectedBranch.isSynced}
            deleted={selectedBranch.deleted}
            onClose={() => setSelected(null)}
          />
        )}
      </div>
    </div>
  );
}

function FilterPicker({ filter, onChange }: { filter: BranchFilter; onChange: (f: BranchFilter) => void }) {
  const stored = filter.mode === "all" ? "all" : filter.prefix === "claude/" ? "claude" : "custom";
  const [mode, setMode] = useState(stored);
  const [prefix, setPrefix] = useState(filter.prefix ?? "");
  useEffect(() => {
    setMode(stored);
    setPrefix(filter.prefix ?? "");
  }, [stored, filter.prefix]);

  const commitPrefix = () => {
    if (prefix !== filter.prefix || filter.mode !== "prefix") onChange({ mode: "prefix", prefix });
  };

  return (
    <>
      <select
        aria-label="Branch filter"
        value={mode}
        onChange={(e) => {
          const v = e.target.value;
          setMode(v);
          if (v === "all") onChange({ mode: "all" });
          else if (v === "claude") onChange({ mode: "prefix", prefix: "claude/" });
        }}
        className="h-7 rounded-md border border-zinc-300 bg-white px-1.5 text-xs dark:border-zinc-600 dark:bg-zinc-800"
      >
        <option value="all">All</option>
        <option value="claude">claude/*</option>
        <option value="custom">Custom prefix</option>
      </select>
      {mode === "custom" && (
        <input
          aria-label="Branch prefix"
          value={prefix}
          placeholder="prefix/"
          onChange={(e) => setPrefix(e.target.value)}
          onBlur={commitPrefix}
          onKeyDown={(e) => e.key === "Enter" && commitPrefix()}
          className="h-7 w-28 rounded-md border border-zinc-300 bg-white px-2 font-mono text-xs outline-none focus:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800"
        />
      )}
    </>
  );
}

function BranchDetail(props: {
  repoId: string;
  name: string;
  sha: string;
  isSynced: boolean;
  deleted: boolean;
  onClose: () => void;
}) {
  const { repoId, name, sha, isSynced, deleted, onClose } = props;
  const [commits, setCommits] = useState<CommitSummary[] | null>(null);
  const startSync = useEcho((s) => s.startSync);
  const now = useNow(30_000);

  useEffect(() => {
    let live = true;
    setCommits(null);
    if (!deleted)
      ipc
        .getBranchLog(repoId, name, 10)
        .then((c) => live && setCommits(c))
        .catch(() => live && setCommits([]));
    return () => {
      live = false;
    };
  }, [repoId, name, sha, deleted]);

  return (
    <aside
      aria-label={`Details for ${name}`}
      className="flex w-72 shrink-0 flex-col border-l border-zinc-200 bg-zinc-50/60 dark:border-zinc-700 dark:bg-zinc-900/60"
    >
      <div className="flex items-start gap-2 border-b border-zinc-200 p-3 dark:border-zinc-700">
        <div className="min-w-0 flex-1 break-all font-mono text-xs font-semibold">{name}</div>
        <button aria-label="Close details" className="text-zinc-400 hover:text-zinc-700" onClick={onClose}>
          ✕
        </button>
      </div>
      <div className="p-3">
        <Button variant="primary" disabled={isSynced || deleted} onClick={() => startSync(repoId, name)}>
          {isSynced ? "Syncing this branch" : "Sync this branch"}
        </Button>
      </div>
      <div className="px-3 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">Recent commits</div>
      <ol className="min-h-0 flex-1 overflow-y-auto px-3 py-1">
        {commits === null && !deleted && (
          <li className="py-2">
            <Spinner />
          </li>
        )}
        {commits?.map((c) => (
          <li key={c.sha} className="border-b border-zinc-200 py-1.5 last:border-0 dark:border-zinc-800">
            <div className="text-xs leading-snug">{c.subject}</div>
            <div className="text-[11px] text-zinc-500">
              <span className="font-mono">{shortSha(c.sha)}</span> · {c.author} · {relativeTime(c.committedAt * 1000, now)}
            </div>
          </li>
        ))}
      </ol>
    </aside>
  );
}
