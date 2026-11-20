import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { DevCommands } from "../../lib/types";
import { CommandField } from "../CommandField";

const now = 1_700_000_000_000;
const commands: DevCommands = {
  pinned: [{ name: "all agents", command: "pnpm dev --agents=planner,reviewer,executor" }],
  recent: [
    { command: "pnpm dev --agents=planner,reviewer", at: now - 2 * 3600_000 },
    { command: "pnpm dev", at: now - 3 * 86400_000 },
  ],
};

function setup(initial = "pnpm dev --agents=planner", cmds = commands) {
  const handlers = { onSubmit: vi.fn(), onPin: vi.fn(), onUnpin: vi.fn() };
  function Harness() {
    const [value, setValue] = useState(initial);
    return <CommandField value={value} onChange={setValue} commands={cmds} now={now} {...handlers} />;
  }
  render(<Harness />);
  return { ...handlers, input: screen.getByRole("combobox", { name: "Dev server command" }) };
}

describe("CommandField", () => {
  it("starts the typed command on Enter", async () => {
    const { input, onSubmit } = setup();
    await userEvent.clear(input);
    await userEvent.type(input, "pnpm dev --agents=reviewer{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("pnpm dev --agents=reviewer");
  });

  it("lists pinned then recent commands, and Enter fills the highlighted one", async () => {
    const { input, onSubmit } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowDown}");
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(3);
    expect(options[0]).toHaveTextContent("all agents");
    expect(options[1]).toHaveTextContent("2h ago");
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(input).toHaveValue("pnpm dev --agents=planner,reviewer");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onSubmit).not.toHaveBeenCalled();
    await userEvent.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("pnpm dev --agents=planner,reviewer");
  });

  it("Ctrl+Enter starts the highlighted command straight away", async () => {
    const { input, onSubmit } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowUp}{Control>}{Enter}{/Control}");
    expect(onSubmit).toHaveBeenCalledWith("pnpm dev");
  });

  it("clicking a command fills the field", async () => {
    const { input } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Show recent commands" }));
    await userEvent.click(screen.getByRole("option", { name: /all agents/ }));
    expect(input).toHaveValue("pnpm dev --agents=planner,reviewer,executor");
  });

  it("pins a recent command under a name, and unpins", async () => {
    const { input, onPin, onUnpin } = setup();
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowDown}");
    // The pinned command has no Pin button.
    expect(screen.queryByRole("button", { name: "Pin pnpm dev --agents=planner,reviewer,executor" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Pin pnpm dev" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Name for this command" }), "plain{Enter}");
    expect(onPin).toHaveBeenCalledWith("pnpm dev", "plain");
    await userEvent.click(screen.getByRole("button", { name: "Unpin all agents" }));
    expect(onUnpin).toHaveBeenCalledWith("pnpm dev --agents=planner,reviewer,executor");
  });

  it("has no dropdown when nothing has been run", async () => {
    const { input } = setup("", { pinned: [], recent: [] });
    expect(screen.queryByRole("button", { name: "Show recent commands" })).toBeNull();
    await userEvent.click(input);
    await userEvent.keyboard("{ArrowDown}");
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});
