// Mirrors the serde types in crates/echo-core (camelCase JSON).

export type FlagKind = "deps" | "migrations" | "env";

export type GitErrorKind =
  | { kind: "lockHeld" }
  | { kind: "auth" }
  | { kind: "network" }
  | { kind: "notFastForward" }
  | { kind: "wouldOverwrite"; paths: string[]; untracked: boolean }
  | { kind: "remoteGone" }
  | { kind: "notARepo" }
  | { kind: "gitNotFound" }
  | { kind: "other" };

export type GitError = GitErrorKind & { message: string };

export type BranchStatus =
  | { kind: "seen" }
  | { kind: "new" }
  | { kind: "unseen"; count: number }
  | { kind: "rewritten" };

export interface BranchInfo {
  name: string;
  sha: string;
  subject: string;
  author: string;
  /** Unix seconds. */
  committedAt: number;
  status: BranchStatus;
  isCurrent: boolean;
  isSynced: boolean;
  deleted: boolean;
}

export interface BranchSnapshot {
  branches: BranchInfo[];
  currentBranch: string | null;
  polledAt: number | null;
  error: GitError | null;
}

export interface CommitSummary {
  sha: string;
  subject: string;
  author: string;
  committedAt: number;
}

export type PauseReason =
  | { kind: "user" }
  | { kind: "checkoutChanged"; current: string | null }
  | { kind: "gitOperation" };

export type SyncState =
  | { kind: "off" }
  | { kind: "switching" }
  | { kind: "idle" }
  | { kind: "pulling" }
  | { kind: "paused"; reason: PauseReason }
  | { kind: "blockedDirty"; files: string[]; untracked: string[]; crlfHint: boolean }
  | { kind: "blockedLocalCommits"; commits: CommitSummary[] }
  | { kind: "blockedDiverged"; commits: CommitSummary[] }
  | { kind: "blockedForcePush"; remoteSha: string }
  | { kind: "remoteBranchDeleted" }
  | { kind: "error"; error: GitError };

export interface SyncStatus {
  branch: string | null;
  state: SyncState;
  /** Unix millis. */
  lastChecked: number | null;
  head: string | null;
}

export type SyncEvent = {
  branch: string;
  flags: FlagKind[];
  at: number;
} & (
  | { kind: "pulled"; from: string; to: string; count: number; subject: string; reset: boolean }
  | { kind: "switched"; from: string | null; to: string; sha: string }
  | { kind: "blocked"; message: string }
  | { kind: "error"; message: string }
);

export interface ActiveFlag {
  kind: FlagKind;
  files: string[];
  at: number;
}

export interface BranchFilter {
  mode: "all" | "prefix";
  prefix?: string;
}

export interface FlagPatterns {
  deps?: string[];
  migrations?: string[];
  env?: string[];
}

export interface RepoConfig {
  id: string;
  path: string;
  displayName: string;
  pollingEnabled: boolean;
  branchFilter: BranchFilter;
  syncedBranch?: string;
  syncPaused: boolean;
  forcePushPolicy: "reset" | "ask";
  installCommand?: string;
  migrateCommand?: string;
  flagPatterns?: FlagPatterns;
  devCommands?: DevCommands;
}

export interface DevCommands {
  /** Newest first. */
  recent: { command: string; at: number }[];
  pinned: PinnedDevCommand[];
}

export interface PinnedDevCommand {
  name: string;
  command: string;
}

export type DevServerStatus =
  | { kind: "idle" }
  | { kind: "running"; command: string; startedAt: number; url: string | null }
  | { kind: "stopping"; command: string; startedAt: number; url: string | null }
  | {
      kind: "exited";
      command: string;
      startedAt: number;
      endedAt: number;
      exitCode: number | null;
      success: boolean;
      stoppedByUser: boolean;
    };

export interface DevServerSnapshot {
  status: DevServerStatus;
  output: string;
  /** Last output chunk included; later events with a higher seq follow on. */
  seq: number;
}

export interface RepoSummary {
  id: string;
  config: RepoConfig;
  available: boolean;
  hasUnseen: boolean;
  sync: SyncStatus;
  flags: ActiveFlag[];
  installCommand: string | null;
  /** https://github.com/<owner>/<repo> when origin is on GitHub. */
  githubUrl: string | null;
  devServer: DevServerStatus;
  /** Guessed from package.json; offered when nothing has been run yet. */
  suggestedDevCommand: string | null;
}

export interface NotificationPrefs {
  pulled: boolean;
  forcePushed: boolean;
  blocked: boolean;
  newBranch: boolean;
}

export interface Settings {
  version: number;
  gitPath?: string;
  launchAtLogin: boolean;
  startHidden: boolean;
  branchPollSeconds: number;
  syncBaseSeconds: number;
  notifications: NotificationPrefs;
}

export interface AppInfo {
  version: string;
  gitPath: string | null;
  gitVersion: string | null;
  gitError: string | null;
  /** GitHub CLI used to list pull requests; null when not installed. */
  ghPath: string | null;
}

export type CmdErrorKind = "dirty" | "untrackedInTheWay" | "git" | "notFound" | "invalid";

export interface CmdError {
  kind: CmdErrorKind;
  message: string;
  paths: string[];
}

export interface DefaultPatterns {
  deps: string[];
  migrations: string[];
  env: string[];
}

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  draft: boolean;
}

/** Open PRs keyed by head branch, from the GitHub CLI. */
export interface PullRequestSnapshot {
  byBranch: Record<string, PullRequest>;
  error: string | null;
}

/** A downloaded update waiting for a restart. */
export interface ReadyUpdate {
  version: string;
  notes: string | null;
}
