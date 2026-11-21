import { useState } from "react";
import { ipc } from "../lib/ipc";
import { useEcho } from "../lib/store";
import { Button, FileList, Modal } from "./ui";

/** Confirmation for every destructive or blocking action. */
export function PromptDialog() {
  const prompt = useEcho((s) => s.prompt);
  const { setPrompt, run, showToast } = useEcho.getState();
  const [busy, setBusy] = useState(false);
  if (!prompt) return null;
  const close = () => !busy && setPrompt(null);
  const go = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    const ok = (await run(async () => (await fn(), true))) ?? false;
    setBusy(false);
    if (ok) setPrompt(null);
  };

  switch (prompt.kind) {
    case "dirtySwitch":
      return (
        <Modal title={`Switch to ${prompt.branch}?`} onClose={close}>
          <Body>
            <p>These tracked files have local changes. Switching will discard them.</p>
            <FileList files={prompt.files} />
            <p className="text-zinc-500">Untracked files are left alone.</p>
          </Body>
          <Footer>
            <Button onClick={close}>Cancel</Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                setPrompt(null);
                await useEcho.getState().startSync(prompt.repoId, prompt.branch, true);
                setBusy(false);
              }}
            >
              Discard and switch
            </Button>
          </Footer>
        </Modal>
      );
    case "untrackedSwitch":
      return (
        <Modal title={`Switch to ${prompt.branch}?`} onClose={close}>
          <Body>
            <p>These untracked files would be overwritten by files on {prompt.branch}:</p>
            <FileList files={prompt.paths} />
            <p className="text-zinc-500">
              Moving them aside renames each to <code>name.echo-&lt;time&gt;</code>. Nothing is deleted.
            </p>
          </Body>
          <Footer>
            <Button onClick={close}>Cancel</Button>
            <Button
              variant="primary"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                const moved = await run(() => ipc.moveUntrackedAside(prompt.repoId, prompt.paths));
                setPrompt(null);
                setBusy(false);
                if (moved) {
                  showToast(`Moved ${moved.length} file${moved.length === 1 ? "" : "s"} aside`);
                  await useEcho.getState().startSync(prompt.repoId, prompt.branch);
                }
              }}
            >
              Move files aside and switch
            </Button>
          </Footer>
        </Modal>
      );
    case "discard":
      return (
        <Modal title="Discard local changes?" onClose={close}>
          <Body>
            <p>These tracked files will be reset to the last synced commit. This can't be undone.</p>
            <FileList files={prompt.files} />
            <p className="text-zinc-500">Untracked files are left alone.</p>
          </Body>
          <Footer>
            <Button onClick={close}>Cancel</Button>
            <Button variant="danger" disabled={busy} onClick={() => go(() => ipc.discardChanges(prompt.repoId))}>
              Discard and continue
            </Button>
          </Footer>
        </Modal>
      );
    case "resetToRemote":
      return (
        <Modal title={`Reset ${prompt.branch} to the remote?`} onClose={close}>
          <Body>
            <p>{prompt.detail}</p>
            <p className="text-zinc-500">
              Runs <code>git reset --hard origin/{prompt.branch}</code>. Untracked files that would be overwritten stop
              the reset instead of being deleted.
            </p>
          </Body>
          <Footer>
            <Button onClick={close}>Cancel</Button>
            <Button variant="danger" disabled={busy} onClick={() => go(() => ipc.resetToRemote(prompt.repoId))}>
              Reset to remote
            </Button>
          </Footer>
        </Modal>
      );
    case "deleteBranch":
      return (
        <Modal title={`Delete ${prompt.branch}?`} onClose={close}>
          <Body>
            <p>
              Deletes the branch on origin. If someone pushed to it since you looked, Echo stops instead of deleting.
            </p>
            {prompt.prNumber !== null && (
              <p className="font-medium text-amber-700 dark:text-amber-300">
                Its open pull request #{prompt.prNumber} will be closed.
              </p>
            )}
            <p className="text-zinc-500">
              Runs <code>git push origin --delete {prompt.branch}</code>. Local branches aren't touched.
            </p>
          </Body>
          <Footer>
            {prompt.githubUrl && (
              <Button
                variant="ghost"
                className="mr-auto"
                title="Open the branches page on GitHub in your browser"
                onClick={() => void run(() => ipc.openBranchesOnGitHub(prompt.repoId))}
              >
                Open on GitHub
              </Button>
            )}
            <Button onClick={close}>Cancel</Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={() =>
                go(async () => {
                  await ipc.deleteRemoteBranch(prompt.repoId, prompt.branch, prompt.sha);
                  showToast(`Deleted ${prompt.branch}`);
                })
              }
            >
              Delete branch
            </Button>
          </Footer>
        </Modal>
      );
    case "removeRepo":
      return (
        <Modal title={`Remove ${prompt.name}?`} onClose={close}>
          <Body>
            <p>Echo will stop watching this repo. No files are touched.</p>
          </Body>
          <Footer>
            <Button onClick={close}>Cancel</Button>
            <Button variant="danger" disabled={busy} onClick={() => go(() => ipc.removeRepo(prompt.repoId))}>
              Remove
            </Button>
          </Footer>
        </Modal>
      );
  }
}

function Body({ children }: { children: React.ReactNode }) {
  return <div className="space-y-2 px-4 py-3 text-xs">{children}</div>;
}

function Footer({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex justify-end gap-2 border-t border-zinc-200 bg-zinc-50 px-4 py-2.5 dark:border-zinc-700 dark:bg-zinc-800/50">
      {children}
    </div>
  );
}
