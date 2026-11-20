import { create } from "zustand";
import { errorMessage, ipc, on } from "./ipc";
import type { AppInfo, BranchSnapshot, CmdError, PullRequestSnapshot, RepoSummary, Settings, SyncEvent } from "./types";

/** A pending "are you sure" prompt shown as a modal. */
export type Prompt =
  | { kind: "dirtySwitch"; repoId: string; branch: string; files: string[] }
  | { kind: "untrackedSwitch"; repoId: string; branch: string; paths: string[] }
  | { kind: "discard"; repoId: string; files: string[] }
  | { kind: "resetToRemote"; repoId: string; branch: string; detail: string }
  | { kind: "removeRepo"; repoId: string; name: string };

interface EchoState {
  ready: boolean;
  appInfo: AppInfo | null;
  settings: Settings | null;
  repos: RepoSummary[];
  selectedId: string | null;
  branches: Record<string, BranchSnapshot>;
  pullRequests: Record<string, PullRequestSnapshot>;
  lastEvent: Record<string, SyncEvent>;
  prompt: Prompt | null;
  toast: string | null;
  settingsOpen: false | "global" | "repo";

  init: () => Promise<() => void>;
  select: (id: string | null) => void;
  refreshRepos: () => Promise<void>;
  setPrompt: (p: Prompt | null) => void;
  showToast: (msg: string) => void;
  openSettings: (tab: false | "global" | "repo") => void;
  /** Run an action, showing failures as a toast. */
  run: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
  startSync: (repoId: string, branch: string, discard?: boolean) => Promise<void>;
}

export const useEcho = create<EchoState>((set, get) => ({
  ready: false,
  appInfo: null,
  settings: null,
  repos: [],
  selectedId: null,
  branches: {},
  pullRequests: {},
  lastEvent: {},
  prompt: null,
  toast: null,
  settingsOpen: false,

  init: async () => {
    const unlisten = await Promise.all([
      on("repos-changed", () => void get().refreshRepos()),
      on("branches-updated", ({ repoId, snapshot }) =>
        set((s) => ({ branches: { ...s.branches, [repoId]: snapshot } })),
      ),
      on("sync-status-changed", ({ repoId, status }) =>
        set((s) => ({ repos: s.repos.map((r) => (r.id === repoId ? { ...r, sync: status } : r)) })),
      ),
      on("flags-changed", ({ repoId, flags }) =>
        set((s) => ({ repos: s.repos.map((r) => (r.id === repoId ? { ...r, flags } : r)) })),
      ),
      on("pull-requests-updated", ({ repoId, prs }) =>
        set((s) => ({ pullRequests: { ...s.pullRequests, [repoId]: prs } })),
      ),
      on("sync-event", ({ repoId, event }) => set((s) => ({ lastEvent: { ...s.lastEvent, [repoId]: event } }))),
      on("settings-changed", async () => set({ settings: await ipc.getSettings(), appInfo: await ipc.appInfo() })),
      on("focus-repo", ({ repoId }) => get().select(repoId)),
    ]);
    const [appInfo, settings, repos] = await Promise.all([ipc.appInfo(), ipc.getSettings(), ipc.listRepos()]);
    // Restore the user's last selection; activity elsewhere only shows as dots.
    const saved = readSelected();
    const selectedId = (saved && repos.some((r) => r.id === saved) ? saved : repos[0]?.id) ?? null;
    set({ appInfo, settings, repos, ready: true });
    get().select(selectedId);
    return () => unlisten.forEach((u) => u());
  },

  select: (id) => {
    set({ selectedId: id });
    writeSelected(id);
    if (id && !get().branches[id]) {
      ipc
        .getBranches(id)
        .then((snapshot) => set((s) => ({ branches: { ...s.branches, [id]: snapshot } })))
        .catch(() => {});
    }
    if (id && !get().pullRequests[id]) {
      ipc
        .getPullRequests(id)
        .then((prs) => set((s) => ({ pullRequests: { ...s.pullRequests, [id]: prs } })))
        .catch(() => {});
    }
  },

  refreshRepos: async () => {
    const repos = await ipc.listRepos();
    set((s) => ({
      repos,
      selectedId: s.selectedId && repos.some((r) => r.id === s.selectedId) ? s.selectedId : (repos[0]?.id ?? null),
    }));
  },

  setPrompt: (prompt) => set({ prompt }),

  showToast: (msg) => {
    set({ toast: msg });
    setTimeout(() => {
      if (get().toast === msg) set({ toast: null });
    }, 5000);
  },

  openSettings: (tab) => set({ settingsOpen: tab }),

  run: async (fn) => {
    try {
      return await fn();
    } catch (e) {
      get().showToast(errorMessage(e));
      return undefined;
    }
  },

  startSync: async (repoId, branch, discard = false) => {
    try {
      await ipc.startSync(repoId, branch, discard);
    } catch (e) {
      const err = e as CmdError;
      if (err?.kind === "dirty") set({ prompt: { kind: "dirtySwitch", repoId, branch, files: err.paths } });
      else if (err?.kind === "untrackedInTheWay")
        set({ prompt: { kind: "untrackedSwitch", repoId, branch, paths: err.paths } });
      else get().showToast(errorMessage(e));
    }
  },
}));

const SELECTED_KEY = "echo.selectedRepo";
function readSelected(): string | null {
  try {
    return localStorage.getItem(SELECTED_KEY);
  } catch {
    return null;
  }
}
function writeSelected(id: string | null) {
  try {
    if (id) localStorage.setItem(SELECTED_KEY, id);
  } catch {
    /* ignore */
  }
}

export const selectedRepo = (s: EchoState) => s.repos.find((r) => r.id === s.selectedId) ?? null;
