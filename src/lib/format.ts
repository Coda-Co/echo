import { useEffect, useState } from "react";
import type { BranchInfo, BranchStatus, SyncState } from "./types";

/** "12s ago", "3m ago", "2h ago", "4d ago". */
export function relativeTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

export const shortSha = (sha: string) => sha.slice(0, 7);

/** Re-render every `ms` so relative times stay fresh. */
export function useNow(ms = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

export type Badge = { label: string; tone: "accent" | "info" | "warn" | "muted" | "ok" };

/** Badge for a branch row: status > current. The synced branch is marked by its icon instead. */
export function branchBadge(b: BranchInfo): Badge | null {
  if (b.deleted) return { label: "deleted", tone: "muted" };
  const s = statusBadge(b.status);
  if (s) return s;
  if (b.isCurrent && !b.isSynced) return { label: "current", tone: "muted" };
  return null;
}

export function statusBadge(s: BranchStatus): Badge | null {
  switch (s.kind) {
    case "new":
      return { label: "new", tone: "accent" };
    case "unseen":
      return { label: `${s.count} new`, tone: "info" };
    case "rewritten":
      return { label: "rewritten", tone: "warn" };
    default:
      return null;
  }
}

export type Tone = "ok" | "busy" | "warn" | "error" | "muted";

export function stateTone(s: SyncState): Tone {
  switch (s.kind) {
    case "idle":
      return "ok";
    case "switching":
    case "pulling":
      return "busy";
    case "blockedDirty":
    case "blockedLocalCommits":
    case "blockedDiverged":
    case "blockedForcePush":
      return "warn";
    case "error":
      return "error";
    default:
      return "muted";
  }
}

export function stateLabel(s: SyncState): string {
  switch (s.kind) {
    case "off":
      return "Not syncing";
    case "switching":
      return "Switching…";
    case "idle":
      return "Up to date";
    case "pulling":
      return "Pulling…";
    case "paused":
      if (s.reason.kind === "user") return "Paused";
      if (s.reason.kind === "gitOperation") return "Waiting: git operation in progress";
      return `Paused: checkout changed to ${s.reason.current ?? "a detached HEAD"}`;
    case "blockedDirty":
      return s.files.length > 0 ? "Blocked: local changes" : "Blocked: untracked files in the way";
    case "blockedLocalCommits":
      return "Blocked: local commits";
    case "blockedDiverged":
      return "Blocked: branch diverged";
    case "blockedForcePush":
      return "Remote was force-pushed";
    case "remoteBranchDeleted":
      return "Branch deleted on remote (merged?)";
    case "error":
      return s.error.kind === "auth"
        ? "Check git credentials"
        : s.error.kind === "network"
          ? "Offline: retrying every minute"
          : "Error";
  }
}

export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
export const fileManagerName = isMac ? "Finder" : navigator?.userAgent?.includes("Windows") ? "Explorer" : "Files";

export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

export function matchesFilter(name: string, filter: { mode: "all" | "prefix"; prefix?: string }): boolean {
  return filter.mode === "all" || name.startsWith(filter.prefix ?? "");
}
