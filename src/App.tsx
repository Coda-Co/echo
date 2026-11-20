import { useEffect } from "react";
import { BranchList } from "./components/BranchList";
import { DevServerPanel, DevServerStatusLine } from "./components/DevServerPanel";
import { PromptDialog } from "./components/PromptDialog";
import { RepoSidebar, UnavailableRepo } from "./components/RepoSidebar";
import { SettingsDialog } from "./components/SettingsDialog";
import { SyncBar } from "./components/SyncBar";
import { Button, Spinner } from "./components/ui";
import { selectedRepo, useEcho } from "./lib/store";

export function App() {
  const ready = useEcho((s) => s.ready);
  const repo = useEcho(selectedRepo);
  const repoCount = useEcho((s) => s.repos.length);
  const appInfo = useEcho((s) => s.appInfo);
  const toast = useEcho((s) => s.toast);
  const devPanelOpen = useEcho((s) => s.devPanelOpen);

  useEffect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    useEcho
      .getState()
      .init()
      .then((c) => (cancelled ? c() : (cleanup = c)))
      .catch((e) => useEcho.getState().showToast(String(e)));
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, []);

  if (!ready) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }

  return (
    <div className="flex h-full">
      <RepoSidebar />
      <main className="flex min-w-0 flex-1 flex-col">
        {appInfo?.gitError && (
          <div className="flex items-center gap-2 border-b border-red-200 bg-red-50 px-4 py-2 text-xs text-red-800 dark:border-red-900 dark:bg-red-950/50 dark:text-red-200">
            <span className="flex-1">{appInfo.gitError}</span>
            <Button onClick={() => useEcho.getState().openSettings("global")}>Set git path…</Button>
          </div>
        )}
        {repo ? (
          repo.available ? (
            <>
              <SyncBar repo={repo} />
              <BranchList repo={repo} />
              {devPanelOpen && <DevServerPanel repo={repo} />}
            </>
          ) : (
            <UnavailableRepo repo={repo} />
          )
        ) : (
          <EmptyState hasRepos={repoCount > 0} />
        )}
        <footer className="flex items-center justify-end gap-2 border-t border-zinc-200 px-3 py-1 dark:border-zinc-700">
          {repo?.available && !devPanelOpen && <DevServerStatusLine repo={repo} />}
          <Button variant="ghost" onClick={() => useEcho.getState().openSettings(repo ? "repo" : "global")}>
            ⚙ Settings
          </Button>
        </footer>
      </main>
      <PromptDialog />
      <SettingsDialog />
      {toast && (
        <div
          role="status"
          className="fixed bottom-10 left-1/2 z-50 max-w-md -translate-x-1/2 rounded-md bg-zinc-900 px-3 py-2 text-xs text-white shadow-lg dark:bg-zinc-100 dark:text-zinc-900"
        >
          {toast}
        </div>
      )}
    </div>
  );
}

function EmptyState({ hasRepos }: { hasRepos: boolean }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-2 p-8 text-center">
      <div className="text-sm font-medium">{hasRepos ? "Select a repo" : "Add a repo to get started"}</div>
      <p className="max-w-sm text-xs text-zinc-500">
        Echo watches a repo's remote branches and keeps the one you pick fast-forwarded in your local checkout, so your
        dev server hot-reloads as cloud agents push.
      </p>
    </div>
  );
}
