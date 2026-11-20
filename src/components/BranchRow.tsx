import { branchBadge, relativeTime } from "../lib/format";
import type { BranchInfo } from "../lib/types";
import { Badge, Button, Dot } from "./ui";

interface Props {
  branch: BranchInfo;
  selected: boolean;
  now: number;
  onClick: () => void;
  onSwitch: () => void;
}

export function BranchRow({ branch: b, selected, now, onClick, onSwitch }: Props) {
  const badge = branchBadge(b);
  const dot = !b.deleted && b.status.kind !== "seen";
  const canSwitch = !b.deleted && !b.isSynced;
  return (
    <li
      role="option"
      aria-selected={selected}
      data-testid={`branch-${b.name}`}
      onClick={onClick}
      className={`group flex cursor-default select-none items-center gap-2 border-b border-zinc-100 px-3 py-2 dark:border-zinc-800 ${
        selected ? "bg-blue-50 dark:bg-blue-950/50" : "hover:bg-zinc-50 dark:hover:bg-zinc-800/50"
      } ${b.deleted ? "opacity-50" : ""}`}
    >
      <div className="flex w-3 justify-center self-start pt-1.5">
        {dot && <Dot tone="accent" title="Unseen activity" />}
        {!dot && b.isSynced && <span title="Synced" className="text-[11px] leading-none text-green-600">↻</span>}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={`truncate font-mono text-[12.5px] ${dot ? "font-semibold" : ""} ${b.deleted ? "line-through" : ""}`}>
            {b.name}
          </span>
          {badge && <Badge badge={badge} />}
          <span className="ml-auto shrink-0 text-[11px] tabular-nums text-zinc-500">
            {b.committedAt ? relativeTime(b.committedAt * 1000, now) : ""}
          </span>
        </div>
        <div className="truncate text-xs text-zinc-500 dark:text-zinc-400">
          {b.subject}
          {b.author && <span className="text-zinc-400 dark:text-zinc-500"> · {b.author}</span>}
        </div>
      </div>
      {canSwitch && (
        <Button
          variant="primary"
          aria-label={`Switch to ${b.name}`}
          title="Check out this branch and keep it synced"
          onClick={(e) => {
            e.stopPropagation();
            onSwitch();
          }}
          className={`shrink-0 ${selected ? "" : "opacity-0 focus-visible:opacity-100 group-hover:opacity-100"}`}
        >
          Switch
        </Button>
      )}
    </li>
  );
}
