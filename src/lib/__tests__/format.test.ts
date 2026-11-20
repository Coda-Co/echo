import { describe, expect, it } from "vitest";
import { matchesFilter, relativeTime, stateLabel, stateTone } from "../format";

describe("format", () => {
  it("relativeTime", () => {
    const now = 10_000_000;
    expect(relativeTime(now - 3_000, now)).toBe("3s ago");
    expect(relativeTime(now - 125_000, now)).toBe("2m ago");
    expect(relativeTime(now - 2 * 3600_000, now)).toBe("2h ago");
    expect(relativeTime(now - 3 * 86400_000, now)).toBe("3d ago");
    expect(relativeTime(now + 5_000, now)).toBe("0s ago");
  });

  it("state labels and tones", () => {
    expect(stateLabel({ kind: "idle" })).toBe("Up to date");
    expect(stateTone({ kind: "idle" })).toBe("ok");
    expect(stateTone({ kind: "blockedDiverged", commits: [] })).toBe("warn");
    expect(stateLabel({ kind: "paused", reason: { kind: "checkoutChanged", current: "main" } })).toContain("main");
    expect(stateLabel({ kind: "error", error: { kind: "network", message: "" } })).toContain("Offline");
  });

  it("filters", () => {
    expect(matchesFilter("claude/x", { mode: "prefix", prefix: "claude/" })).toBe(true);
    expect(matchesFilter("main", { mode: "prefix", prefix: "claude/" })).toBe(false);
    expect(matchesFilter("main", { mode: "all" })).toBe(true);
  });
});
