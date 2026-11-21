import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

vi.mock("../../lib/terminals", () => ({ TERMINAL_BG: "#000", terminals: {} }));
vi.mock("../../lib/ipc", () => ({ ipc: { installUpdate: vi.fn(async () => {}) }, on: vi.fn(), errorMessage: String }));

const { ipc } = await import("../../lib/ipc");
const { UpdateButton } = await import("../../App");
const { useEcho } = await import("../../lib/store");

describe("UpdateButton", () => {
  it("is hidden until an update has downloaded, then installs on click", async () => {
    const { container } = render(<UpdateButton />);
    expect(container).toBeEmptyDOMElement();
    act(() => useEcho.setState({ update: { version: "0.1.3", notes: null } }));
    const button = screen.getByRole("button", { name: /Restart to update to 0.1.3/ });
    expect(button).not.toHaveTextContent("stops dev servers");
    await userEvent.click(button);
    expect(ipc.installUpdate).toHaveBeenCalled();
  });

  it("warns when dev servers are running", () => {
    act(() =>
      useEcho.setState({
        update: { version: "0.1.3", notes: null },
        repos: [{ devServer: { kind: "running", command: "x", startedAt: 0, url: null } } as never],
      }),
    );
    render(<UpdateButton />);
    expect(screen.getByRole("button", { name: /Restart to update/ })).toHaveTextContent("stops dev servers");
  });
});
