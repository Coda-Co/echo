// Typed wrappers for the Tauri commands and events. All git work happens in
// Rust; the UI only calls these.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ActiveFlag,
  AppInfo,
  BranchSnapshot,
  CmdError,
  CommitSummary,
  DefaultPatterns,
  DevServerSnapshot,
  DevServerStatus,
  FlagKind,
  PullRequestSnapshot,
  RepoConfig,
  RepoSummary,
  Settings,
  SyncEvent,
  SyncStatus,
} from "./types";

export type RepoConfigPatch = Partial<
  Pick<
    RepoConfig,
    | "displayName"
    | "pollingEnabled"
    | "branchFilter"
    | "forcePushPolicy"
    | "installCommand"
    | "migrateCommand"
    | "flagPatterns"
  >
>;

export type SettingsPatch = Partial<Omit<Settings, "version">>;

export const ipc = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listRepos: () => invoke<RepoSummary[]>("list_repos"),
  addRepo: (path: string) => invoke<RepoSummary>("add_repo", { path }),
  removeRepo: (id: string) => invoke<void>("remove_repo", { id }),
  locateRepo: (id: string, path: string) => invoke<RepoSummary>("locate_repo", { id, path }),
  updateRepoConfig: (id: string, patch: RepoConfigPatch) =>
    invoke<RepoSummary>("update_repo_config", { id, patch }),
  getBranches: (repoId: string) => invoke<BranchSnapshot>("get_branches", { repoId }),
  markSeen: (repoId: string, branch?: string) => invoke<void>("mark_seen", { repoId, branch: branch ?? null }),
  getBranchLog: (repoId: string, branch: string, n = 10) =>
    invoke<CommitSummary[]>("get_branch_log", { repoId, branch, n }),
  startSync: (repoId: string, branch: string, discard = false) =>
    invoke<SyncStatus>("start_sync", { repoId, branch, discard }),
  stopSync: (repoId: string) => invoke<SyncStatus>("stop_sync", { repoId }),
  pauseSync: (repoId: string) => invoke<SyncStatus>("pause_sync", { repoId }),
  resumeSync: (repoId: string) => invoke<SyncStatus>("resume_sync", { repoId }),
  syncNow: (repoId: string) => invoke<SyncStatus>("sync_now", { repoId }),
  discardChanges: (repoId: string) => invoke<SyncStatus>("discard_changes", { repoId }),
  moveUntrackedAside: (repoId: string, paths: string[]) =>
    invoke<string[]>("move_untracked_aside", { repoId, paths }),
  resetToRemote: (repoId: string) => invoke<SyncStatus>("reset_to_remote", { repoId }),
  dismissFlag: (repoId: string, kind: FlagKind) => invoke<ActiveFlag[]>("dismiss_flag", { repoId, kind }),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: SettingsPatch) => invoke<Settings>("update_settings", { patch }),
  defaultFlagPatterns: () => invoke<DefaultPatterns>("default_flag_patterns"),
  openInFileManager: (id: string) => invoke<void>("open_in_file_manager", { id }),
  openInTerminal: (id: string) => invoke<void>("open_in_terminal", { id }),
  getPullRequests: (repoId: string) => invoke<PullRequestSnapshot>("get_pull_requests", { repoId }),
  /** Opens the repo's GitHub page in the browser (URL is built in Rust). */
  openOnGitHub: (repoId: string) => invoke<void>("open_on_github", { repoId }),
  /** Opens the open PR for a branch in the browser (URL is built in Rust). */
  openPullRequest: (repoId: string, branch: string) => invoke<void>("open_pull_request", { repoId, branch }),

  /** Run `command` from the repo root in the user's shell. */
  devServerStart: (repoId: string, command: string, cols: number, rows: number) =>
    invoke<DevServerStatus>("dev_server_start", { repoId, command, cols, rows }),
  devServerStop: (repoId: string) => invoke<DevServerStatus>("dev_server_stop", { repoId }),
  devServerRestart: (repoId: string, cols: number, rows: number) =>
    invoke<DevServerStatus>("dev_server_restart", { repoId, cols, rows }),
  devServerWrite: (repoId: string, data: string) => invoke<void>("dev_server_write", { repoId, data }),
  devServerResize: (repoId: string, cols: number, rows: number) =>
    invoke<void>("dev_server_resize", { repoId, cols, rows }),
  devServerSnapshot: (repoId: string) => invoke<DevServerSnapshot>("dev_server_snapshot", { repoId }),
  /** Opens the local URL the dev server printed (taken from Rust, localhost only). */
  openDevServerUrl: (repoId: string) => invoke<void>("open_dev_server_url", { repoId }),
  pinDevCommand: (repoId: string, command: string, name: string) =>
    invoke<RepoSummary>("pin_dev_command", { repoId, command, name }),
  unpinDevCommand: (repoId: string, command: string) =>
    invoke<RepoSummary>("unpin_dev_command", { repoId, command }),
};

export interface EventPayloads {
  "repos-changed": null;
  "branches-updated": { repoId: string; snapshot: BranchSnapshot };
  "sync-status-changed": { repoId: string; status: SyncStatus };
  "sync-event": { repoId: string; event: SyncEvent };
  "flags-changed": { repoId: string; flags: ActiveFlag[] };
  "settings-changed": null;
  "pull-requests-updated": { repoId: string; prs: PullRequestSnapshot };
  "dev-server-output": { repoId: string; seq: number; data: string };
  "dev-server-changed": { repoId: string; status: DevServerStatus };
  /** Only sent after the user picks a branch in the tray menu that needs a decision. */
  "focus-repo": { repoId: string };
}

export function on<K extends keyof EventPayloads>(
  name: K,
  handler: (payload: EventPayloads[K]) => void,
): Promise<UnlistenFn> {
  return listen<EventPayloads[K]>(name, (e) => handler(e.payload));
}

export function isCmdError(e: unknown): e is CmdError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export function errorMessage(e: unknown): string {
  if (isCmdError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
