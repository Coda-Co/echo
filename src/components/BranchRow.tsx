import { branchBadge, relativeTime } from "../lib/format";
import type { BranchInfo, PullRequest } from "../lib/types";
import { Badge, Button, Dot, PauseIcon, PullRequestIcon, SyncIcon } from "./ui";

interface Props {
  branch: BranchInfo;
  /** This branch is open in the preview panel. */
  selected: boolean;
  now: number;
  /** Syncing is paused (only affects the synced branch's row). */
  syncPaused?: boolean;
  /** Open pull request with this branch as its head. */
  pr?: PullRequest;
  onOpenPr?: () => void;
  /** Blue dot click: mark this branch's activity as seen. */
  onMarkSeen?: () => void;
  /** Row click: check out this branch and keep it synced. */
  onSwitch: () => void;
  /** History button: toggle the history panel for this branch. */
  onPreview: () => void;
}

export function BranchRow({ branch: b, selected, now, syncPaused = false, pr, onOpenPr, onMarkSeen, onSwitch, onPreview }: Props) {
  const badge = branchBadge(b);
  const dot = !b.deleted && b.status.kind !== "seen";
  const canSwitch = !b.deleted && !b.isSynced;
  return (
    <li
      role="option"
      aria-selected={selected}
      data-testid={`branch-${b.name}`}
      title={canSwitch ? "Click to switch to this branch" : undefined}
      onClick={() => canSwitch && onSwitch()}
      className={`group relative flex cursor-default select-none items-center gap-2 border-b border-zinc-100 px-3 py-2 dark:border-zinc-800 ${
        selected ? "bg-blue-50 dark:bg-blue-950/50" : "hover:bg-zinc-50 dark:hover:bg-zinc-800/50"
      } ${b.deleted ? "opacity-50" : ""}`}
    >
      <div className="flex w-3 justify-center self-start pt-1.5">
        {dot && (
          <button
            type="button"
            aria-label={`Mark ${b.name} as seen`}
            title="Mark as seen"
            onClick={(e) => {
              e.stopPropagation();
              onMarkSeen?.();
            }}
            className="-m-1.5 rounded-full p-1.5 hover:bg-blue-500/15 focus-visible:outline focus-visible:outline-1 focus-visible:outline-blue-500"
          >
            <Dot tone="accent" title="Unseen activity" className="block" />
          </button>
        )}
        {!dot &&
          b.isSynced &&
          (syncPaused ? (
            <PauseIcon title="Sync paused" className="text-zinc-400" />
          ) : (
            <SyncIcon title="Synced" className="text-green-600" />
          ))}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={`truncate font-mono text-[12.5px] ${dot ? "font-semibold" : ""} ${b.deleted ? "line-through" : ""}`}>
            {b.name}
          </span>
          {badge && <Badge badge={badge} />}
          {pr && !b.deleted && (
            <button
              type="button"
              aria-label={`Open pull request #${pr.number}${pr.draft ? " (draft)" : ""} on GitHub`}
              title={`${pr.draft ? "Draft PR" : "PR"} #${pr.number}: ${pr.title}`}
              onClick={(e) => {
                e.stopPropagation();
                onOpenPr?.();
              }}
              className={`inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 py-px text-[10px] font-semibold hover:underline ${
                pr.draft
                  ? "bg-zinc-200 text-zinc-700 dark:bg-zinc-700 dark:text-zinc-200"
                  : "bg-green-100 text-green-800 dark:bg-green-900/60 dark:text-green-200"
              }`}
            >
              <PullRequestIcon />#{pr.number}
            </button>
          )}
          <span className="ml-auto shrink-0 text-[11px] tabular-nums text-zinc-500">
            {b.committedAt ? relativeTime(b.committedAt * 1000, now) : ""}
          </span>
        </div>
        <div className="truncate text-xs text-zinc-500 dark:text-zinc-400">
          {b.subject}
          {b.author && <span className="text-zinc-400 dark:text-zinc-500"> · {b.author}</span>}
        </div>
      </div>
      {!b.deleted && (
        <Button
          aria-label={`${selected ? "Hide history of" : "Show history of"} ${b.name}`}
          aria-pressed={selected}
          title="Show recent commits without switching"
          onClick={(e) => {
            e.stopPropagation();
            onPreview();
          }}
          className={`absolute right-3 top-1/2 -translate-y-1/2 shadow-sm ${
            selected ? "" : "pointer-events-none opacity-0 focus-visible:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100"
          }`}
        >
          {selected ? "Hide" : "History"}
        </Button>
      )}
    </li>
  );
}
