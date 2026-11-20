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
  FlagKind,
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
};

export interface EventPayloads {
  "repos-changed": null;
  "branches-updated": { repoId: string; snapshot: BranchSnapshot };
  "sync-status-changed": { repoId: string; status: SyncStatus };
  "sync-event": { repoId: string; event: SyncEvent };
  "flags-changed": { repoId: string; flags: ActiveFlag[] };
  "settings-changed": null;
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
