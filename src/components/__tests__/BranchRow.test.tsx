import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { BranchInfo } from "../../lib/types";
import { BranchRow } from "../BranchRow";

const now = 1_700_000_100_000;
const base: BranchInfo = {
  name: "claude/add-filters",
  sha: "abc1234def",
  subject: "Add date range filter",
  author: "Claude",
  committedAt: 1_700_000_088,
  status: { kind: "seen" },
  isCurrent: false,
  isSynced: false,
  deleted: false,
};

function renderRow(b: Partial<BranchInfo>, handlers = { onSwitch: vi.fn(), onPreview: vi.fn() }, syncPaused = false) {
  render(
    <ul>
      <BranchRow branch={{ ...base, ...b }} selected={false} now={now} syncPaused={syncPaused} {...handlers} />
    </ul>,
  );
  return handlers;
}

describe("BranchRow", () => {
  it("shows name, subject, author and relative time", () => {
    renderRow({});
    expect(screen.getByText("claude/add-filters")).toBeInTheDocument();
    expect(screen.getByText(/Add date range filter/)).toBeInTheDocument();
    expect(screen.getByText(/Claude/)).toBeInTheDocument();
    expect(screen.getByText("12s ago")).toBeInTheDocument();
    expect(screen.queryByTestId("badge")).toBeNull();
    expect(screen.queryByLabelText("Unseen activity")).toBeNull();
  });

  it.each([
    [{ status: { kind: "unseen", count: 3 } }, "3 new", true],
    [{ status: { kind: "new" } }, "new", true],
    [{ status: { kind: "rewritten" } }, "rewritten", true],
    [{ isSynced: true }, "synced", false],
    [{ isCurrent: true }, "current", false],
    [{ deleted: true, status: { kind: "new" } }, "deleted", false],
    [{ isSynced: true, status: { kind: "unseen", count: 2 } }, "synced", true],
  ] as [Partial<BranchInfo>, string, boolean][])("badge for %j is %s", (patch, label, dot) => {
    renderRow(patch);
    expect(screen.getByTestId("badge")).toHaveTextContent(label);
    expect(!!screen.queryByLabelText("Unseen activity")).toBe(dot);
  });

  it("the synced branch shows as paused while sync is paused", () => {
    renderRow({ isSynced: true }, undefined, true);
    expect(screen.getByTestId("badge")).toHaveTextContent("paused");
    expect(screen.getByRole("img", { name: "Sync paused" })).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "Synced" })).toBeNull();
  });

  it("clicking the row switches to it", async () => {
    const h = renderRow({});
    await userEvent.click(screen.getByTestId("branch-claude/add-filters"));
    expect(h.onSwitch).toHaveBeenCalledOnce();
    expect(h.onPreview).not.toHaveBeenCalled();
  });

  it.each([[{ isSynced: true }], [{ deleted: true }]] as [Partial<BranchInfo>][])(
    "clicking the row does nothing for %j",
    async (patch) => {
      const h = renderRow(patch);
      await userEvent.click(screen.getByTestId("branch-claude/add-filters"));
      expect(h.onSwitch).not.toHaveBeenCalled();
    },
  );

  it("History opens the history panel without switching", async () => {
    const h = renderRow({});
    await userEvent.click(screen.getByRole("button", { name: "Show history of claude/add-filters" }));
    expect(h.onPreview).toHaveBeenCalledOnce();
    expect(h.onSwitch).not.toHaveBeenCalled();
  });

  it("synced branches still have History", () => {
    renderRow({ isSynced: true });
    expect(screen.getByRole("button", { name: "Show history of claude/add-filters" })).toBeInTheDocument();
  });

  it("no History button for deleted branches", () => {
    renderRow({ deleted: true });
    expect(screen.queryByRole("button", { name: /history/i })).toBeNull();
  });
});
