import { describe, expect, it, vi } from "vitest";

vi.mock("../format", async (orig) => ({ ...(await orig<typeof import("../format")>()), isMac: false, copyText: vi.fn(async () => {}) }));
vi.mock("../ipc", () => ({ ipc: {} }));
vi.mock("@xterm/xterm", () => ({ Terminal: class {} }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class {} }));

const { copyPasteKeys, outputLines } = await import("../terminals");
const { copyText } = await import("../format");

function fakeTerm(selection: string) {
  return {
    hasSelection: () => selection !== "",
    getSelection: () => selection,
    clearSelection: vi.fn(),
  } as unknown as Parameters<typeof copyPasteKeys>[0];
}
const key = (k: string, mods: Partial<KeyboardEventInit> = {}) => new KeyboardEvent("keydown", { key: k, ...mods });

describe("terminal copy and paste keys (Windows/Linux)", () => {
  it("Ctrl+C copies a selection instead of interrupting", () => {
    const term = fakeTerm("Error: boom\n    at x.ts:1");
    expect(copyPasteKeys(term, key("c", { ctrlKey: true }))).toBe(false);
    expect(copyText).toHaveBeenCalledWith("Error: boom\n    at x.ts:1");
  });

  it("Ctrl+C with nothing selected still goes to the server", () => {
    expect(copyPasteKeys(fakeTerm(""), key("c", { ctrlKey: true }))).toBe(true);
  });

  it("Ctrl+Shift+C never interrupts; Ctrl+V pastes", () => {
    expect(copyPasteKeys(fakeTerm(""), key("C", { ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(copyPasteKeys(fakeTerm(""), key("v", { ctrlKey: true }))).toBe(false);
    expect(copyPasteKeys(fakeTerm(""), key("r"))).toBe(true);
  });
});

describe("outputLines", () => {
  it("joins wrapped lines and drops trailing blank lines", () => {
    const rows = [
      { t: "line one", w: false },
      { t: "a very long li", w: false },
      { t: "ne that wrapped", w: true },
      { t: "", w: false },
      { t: "", w: false },
    ];
    const term = {
      buffer: {
        active: {
          length: rows.length,
          getLine: (i: number) => ({ translateToString: () => rows[i].t, isWrapped: rows[i].w }),
        },
      },
    } as unknown as Parameters<typeof outputLines>[0];
    expect(outputLines(term)).toEqual(["line one", "a very long line that wrapped"]);
    expect(outputLines(term, 1)).toEqual(["a very long line that wrapped"]);
    expect(outputLines(term, 50)).toHaveLength(2);
  });
});
