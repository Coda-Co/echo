import { create } from "zustand";
import { errorMessage, ipc, on } from "./ipc";
import { terminals } from "./terminals";
import type {
  AppInfo,
  BranchSnapshot,
  CmdError,
  PullRequestSnapshot,
  ReadyUpdate,
  RepoSummary,
  Settings,
  SyncEvent,
} from "./types";

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
  /** Dev server panel expanded (otherwise a status line in the footer). */
  devPanelOpen: boolean;
  /** Downloaded update waiting for a restart. */
  update: ReadyUpdate | null;

  init: () => Promise<() => void>;
  select: (id: string | null) => void;
  refreshRepos: () => Promise<void>;
  setPrompt: (p: Prompt | null) => void;
  showToast: (msg: string) => void;
  openSettings: (tab: false | "global" | "repo") => void;
  setDevPanelOpen: (open: boolean) => void;
  /** Run an action, showing failures as a toast. */
  run: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
  startSync: (repoId: string, branch: string, discard?: boolean) => Promise<void>;
}

const SELECTED_KEY = "echo.selectedRepo";
const DEV_PANEL_KEY = "echo.devPanelOpen";

export function readLocal(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
export function writeLocal(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
}
function readFlag(key: string, fallback: boolean): boolean {
  const v = readLocal(key);
  return v === null ? fallback : v === "1";
}
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
  devPanelOpen: readFlag(DEV_PANEL_KEY, true),
  update: null,

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
      on("dev-server-changed", ({ repoId, status }) =>
        set((s) => ({ repos: s.repos.map((r) => (r.id === repoId ? { ...r, devServer: status } : r)) })),
      ),
      on("dev-server-output", ({ repoId, seq, data }) => terminals.output(repoId, seq, data)),
      on("update-ready", (update) => set({ update })),
    ]);
    const [appInfo, settings, repos] = await Promise.all([ipc.appInfo(), ipc.getSettings(), ipc.listRepos()]);
    // Restore the user's last selection; activity elsewhere only shows as dots.
    const saved = readSelected();
    const selectedId = (saved && repos.some((r) => r.id === saved) ? saved : repos[0]?.id) ?? null;
    set({ appInfo, settings, repos, ready: true });
    ipc
      .updateStatus()
      .then((update) => set({ update }))
      .catch(() => {});
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

  setDevPanelOpen: (open) => {
    set({ devPanelOpen: open });
    writeLocal(DEV_PANEL_KEY, open ? "1" : "0");
  },

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

export const selectedRepo = (s: EchoState) => s.repos.find((r) => r.id === s.selectedId) ?? null;
