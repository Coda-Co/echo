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
    [{ isCurrent: true }, "current", false],
    [{ deleted: true, status: { kind: "new" } }, "deleted", false],
    [{ isSynced: true, status: { kind: "unseen", count: 2 } }, "2 new", true],
  ] as [Partial<BranchInfo>, string, boolean][])("badge for %j is %s", (patch, label, dot) => {
    renderRow(patch);
    expect(screen.getByTestId("badge")).toHaveTextContent(label);
    expect(!!screen.queryByLabelText("Unseen activity")).toBe(dot);
  });

  it("the synced branch shows as paused while sync is paused", () => {
    renderRow({ isSynced: true }, undefined, true);
    expect(screen.getByRole("img", { name: "Sync paused" })).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "Synced" })).toBeNull();
  });

  it("shows an open PR and opens it without switching", async () => {
    const onOpenPr = vi.fn();
    const onSwitch = vi.fn();
    render(
      <ul>
        <BranchRow
          branch={base}
          selected={false}
          now={now}
          pr={{ number: 42, title: "Add filters", url: "https://github.com/o/r/pull/42", draft: false }}
          onOpenPr={onOpenPr}
          onSwitch={onSwitch}
          onPreview={vi.fn()}
        />
      </ul>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Open pull request #42 on GitHub" }));
    expect(onOpenPr).toHaveBeenCalledOnce();
    expect(onSwitch).not.toHaveBeenCalled();
  });

  it("the synced branch has no badge, just the sync icon", () => {
    renderRow({ isSynced: true, isCurrent: true });
    expect(screen.queryByTestId("badge")).toBeNull();
    expect(screen.getByRole("img", { name: "Synced" })).toBeInTheDocument();
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
