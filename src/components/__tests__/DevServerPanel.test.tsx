import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DevServerStatus, RepoSummary } from "../../lib/types";

vi.mock("../../lib/terminals", () => ({
  TERMINAL_BG: "#000",
  terminals: {
    attach: vi.fn(),
    detach: vi.fn(),
    fit: vi.fn(() => null),
    size: vi.fn(() => ({ cols: 100, rows: 20 })),
    focus: vi.fn(),
    output: vi.fn(),
    copy: vi.fn(async (_id: string, last?: number) => ({ kind: "lines", count: last ?? 1234 })),
  },
}));
vi.mock("../../lib/ipc", () => ({
  ipc: {
    devServerStart: vi.fn(async () => ({})),
    devServerStop: vi.fn(async () => ({})),
    devServerRestart: vi.fn(async () => ({})),
    devServerResize: vi.fn(async () => {}),
    openDevServerUrl: vi.fn(async () => {}),
    pinDevCommand: vi.fn(async () => ({})),
    unpinDevCommand: vi.fn(async () => ({})),
  },
  on: vi.fn(),
  errorMessage: String,
}));

globalThis.ResizeObserver = class {
  observe() {}
  disconnect() {}
  unobserve() {}
};

const { ipc } = await import("../../lib/ipc");
const { terminals } = await import("../../lib/terminals");
const { DevServerPanel, DevServerStatusLine, initialCommand } = await import("../DevServerPanel");
const { useEcho } = await import("../../lib/store");

const now = Date.now();
function repo(devServer: DevServerStatus, extra: Partial<RepoSummary> = {}): RepoSummary {
  return {
    id: "r1",
    config: {
      id: "r1",
      path: "/p",
      displayName: "agent-platform",
      pollingEnabled: true,
      branchFilter: { mode: "prefix", prefix: "claude/" },
      syncPaused: false,
      forcePushPolicy: "reset",
      devCommands: { recent: [{ command: "pnpm dev --agents=planner", at: now - 60_000 }], pinned: [] },
    },
    available: true,
    hasUnseen: false,
    sync: { branch: null, state: { kind: "off" }, lastChecked: null, head: null },
    flags: [],
    installCommand: null,
    githubUrl: null,
    devServer,
    suggestedDevCommand: "pnpm dev",
    ...extra,
  };
}

const running: DevServerStatus = {
  kind: "running",
  command: "pnpm dev --agents=planner,reviewer",
  startedAt: now - 14 * 60_000,
  url: "http://localhost:5173/",
};
const crashed: DevServerStatus = {
  kind: "exited",
  command: "pnpm dev --agents=planner",
  startedAt: now - 60_000,
  endedAt: now - 40_000,
  exitCode: 1,
  success: false,
  stoppedByUser: false,
};

describe("DevServerPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it("pre-fills the last command, else the newest recent, else the package.json guess", () => {
    expect(initialCommand(repo(running))).toBe("pnpm dev --agents=planner,reviewer");
    expect(initialCommand(repo({ kind: "idle" }))).toBe("pnpm dev --agents=planner");
    const fresh = repo({ kind: "idle" });
    fresh.config.devCommands = undefined;
    expect(initialCommand(fresh)).toBe("pnpm dev");
  });

  it("starts the edited command at the terminal's size", async () => {
    render(<DevServerPanel repo={repo({ kind: "idle" })} />);
    expect(screen.getByText("not running")).toBeInTheDocument();
    const input = screen.getByRole("combobox", { name: "Dev server command" });
    expect(input).toHaveValue("pnpm dev --agents=planner");
    await userEvent.type(input, ",reviewer");
    await userEvent.click(screen.getByRole("button", { name: /Start/ }));
    expect(ipc.devServerStart).toHaveBeenCalledWith("r1", "pnpm dev --agents=planner,reviewer", 100, 20);
  });

  it("shows the running command, uptime and port, with Restart and Stop", async () => {
    render(<DevServerPanel repo={repo(running)} />);
    expect(screen.getByText("pnpm dev --agents=planner,reviewer")).toBeInTheDocument();
    expect(screen.getByTestId("dev-uptime")).toHaveTextContent("up 14m · :5173");
    await userEvent.click(screen.getByRole("button", { name: ":5173" }));
    expect(ipc.openDevServerUrl).toHaveBeenCalledWith("r1");
    await userEvent.click(screen.getByRole("button", { name: /Restart/ }));
    expect(ipc.devServerRestart).toHaveBeenCalledWith("r1", 100, 20);
    await userEvent.click(screen.getByRole("button", { name: /Stop/ }));
    expect(ipc.devServerStop).toHaveBeenCalledWith("r1");
  });

  it("after a crash offers Restart, or editing the command first", async () => {
    render(<DevServerPanel repo={repo(crashed)} />);
    expect(screen.getByText(/exited with code 1/)).toBeInTheDocument();
    expect(screen.queryByRole("combobox")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Edit command" }));
    expect(screen.getByRole("combobox", { name: "Dev server command" })).toHaveValue("pnpm dev --agents=planner");
  });

  it("copies the selection or the last N lines, with N picked from a menu", async () => {
    localStorage.removeItem("echo.devCopyLines");
    render(<DevServerPanel repo={repo(crashed)} />);
    await userEvent.click(screen.getByRole("button", { name: /Copy last 200/ }));
    expect(terminals.copy).toHaveBeenLastCalledWith("r1", 200, true);
    expect(useEcho.getState().toast).toBe("Copied 200 lines");

    await userEvent.click(screen.getByRole("button", { name: "Choose how many lines to copy" }));
    const items = screen.getAllByRole("menuitemradio");
    expect(items.map((i) => i.textContent)).toEqual([
      "Copy last 50 lines",
      "✓Copy last 200 lines",
      "Copy last 500 lines",
      "Copy last 1000 lines",
      "Copy all output",
    ]);
    await userEvent.click(screen.getByRole("menuitemradio", { name: /all output/ }));
    expect(terminals.copy).toHaveBeenLastCalledWith("r1", undefined, false);
    expect(screen.getByRole("button", { name: /Copy all/ })).toBeInTheDocument();
    expect(localStorage.getItem("echo.devCopyLines")).toBe("all");
  });

  it("collapses to a status line in the footer", async () => {
    act(() => useEcho.getState().setDevPanelOpen(true));
    render(<DevServerPanel repo={repo(running)} />);
    await userEvent.click(screen.getByRole("button", { name: "Collapse dev server panel" }));
    expect(useEcho.getState().devPanelOpen).toBe(false);

    render(<DevServerStatusLine repo={repo(crashed)} />);
    const line = screen.getByRole("button", { name: "Show dev server" });
    expect(line).toHaveTextContent("exited with code 1");
    await userEvent.click(line);
    expect(useEcho.getState().devPanelOpen).toBe(true);
  });
});
