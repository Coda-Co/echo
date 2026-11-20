import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { SyncState } from "../../lib/types";
import { BlockedPanel, type BlockedActions } from "../BlockedPanel";

function setup(state: SyncState) {
  const actions: BlockedActions = {
    discard: vi.fn(),
    moveAside: vi.fn(),
    resetToRemote: vi.fn(),
    retry: vi.fn(),
    syncInstead: vi.fn(),
    switchBack: vi.fn(),
    stop: vi.fn(),
  };
  const r = render(<BlockedPanel state={state} branch="claude/foo" actions={actions} />);
  return { actions, ...r };
}

const commit = { sha: "1234567890", subject: "Local work", author: "Me", committedAt: 0 };

describe("BlockedPanel", () => {
  it("renders nothing for healthy states", () => {
    for (const s of [{ kind: "idle" }, { kind: "pulling" }, { kind: "off" }, { kind: "paused", reason: { kind: "user" } }] as SyncState[]) {
      const { container, unmount } = setup(s);
      expect(container).toBeEmptyDOMElement();
      unmount();
    }
  });

  it("dirty tracked files offer discard", async () => {
    const { actions } = setup({ kind: "blockedDirty", files: ["src/a.ts", "b.ts"], untracked: [], crlfHint: false });
    expect(screen.getByText("src/a.ts")).toBeInTheDocument();
    expect(screen.queryByText("Move files aside")).toBeNull();
    await userEvent.click(screen.getByText("Discard and continue"));
    expect(actions.discard).toHaveBeenCalledWith(["src/a.ts", "b.ts"]);
  });

  it("untracked files offer move aside, not discard", async () => {
    const { actions } = setup({ kind: "blockedDirty", files: [], untracked: [".env.local"], crlfHint: false });
    expect(screen.queryByText("Discard and continue")).toBeNull();
    await userEvent.click(screen.getByText("Move files aside"));
    expect(actions.moveAside).toHaveBeenCalledWith([".env.local"]);
  });

  it("shows the autocrlf hint", () => {
    setup({ kind: "blockedDirty", files: ["a"], untracked: [], crlfHint: true });
    expect(screen.getByText(/core.autocrlf/)).toBeInTheDocument();
  });

  it("local commits and diverged offer reset to remote", async () => {
    const { actions, unmount } = setup({ kind: "blockedLocalCommits", commits: [commit] });
    expect(screen.getByText("Local work")).toBeInTheDocument();
    await userEvent.click(screen.getByText("Reset to remote"));
    expect(actions.resetToRemote).toHaveBeenCalledWith("1 local commit will be discarded.");
    unmount();
    const d = setup({ kind: "blockedDiverged", commits: [commit, { ...commit, sha: "fedcba9876" }] });
    expect(screen.getByText(/was rewritten/)).toBeInTheDocument();
    await userEvent.click(screen.getByText("Reset to remote"));
    expect(d.actions.resetToRemote).toHaveBeenCalledWith("2 local commits will be discarded.");
  });

  it("force-push with ask policy offers reset to the new tip", async () => {
    const { actions } = setup({ kind: "blockedForcePush", remoteSha: "abcdef0123" });
    await userEvent.click(screen.getByText("Reset to abcdef0"));
    expect(actions.resetToRemote).toHaveBeenCalled();
  });

  it("checkout changed offers sync-instead and switch back", async () => {
    const { actions } = setup({ kind: "paused", reason: { kind: "checkoutChanged", current: "main" } });
    await userEvent.click(screen.getByText("Sync main instead"));
    expect(actions.syncInstead).toHaveBeenCalledWith("main");
    await userEvent.click(screen.getByText("Switch back to claude/foo"));
    expect(actions.switchBack).toHaveBeenCalled();
  });

  it("detached HEAD only offers switch back", () => {
    setup({ kind: "paused", reason: { kind: "checkoutChanged", current: null } });
    expect(screen.queryByText(/instead/)).toBeNull();
    expect(screen.getByText(/detached HEAD/)).toBeInTheDocument();
  });

  it("deleted branch offers stop", async () => {
    const { actions } = setup({ kind: "remoteBranchDeleted" });
    await userEvent.click(screen.getByText("Stop syncing"));
    expect(actions.stop).toHaveBeenCalled();
  });

  it("errors offer retry with a helpful message", async () => {
    const { actions } = setup({ kind: "error", error: { kind: "auth", message: "fatal: Authentication failed" } });
    expect(screen.getByText(/couldn't authenticate/)).toBeInTheDocument();
    await userEvent.click(screen.getByText("Retry"));
    expect(actions.retry).toHaveBeenCalled();
  });
});
