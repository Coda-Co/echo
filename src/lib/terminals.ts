// One xterm.js terminal per repo's dev server, kept alive while the panel is
// hidden or another repo is selected, so switching back is instant and
// scroll position survives. Output arrives as `dev-server-output` events with
// a sequence number; a terminal created later starts from Rust's scrollback
// snapshot and skips events it already has.

import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ipc } from "./ipc";

const FONT_FAMILY = '"Geist Mono Variable", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

export const TERMINAL_BG = "#0c0c0e";

interface Slot {
  term: Terminal;
  fit: FitAddon;
  host: HTMLDivElement;
  opened: boolean;
  /** Last output chunk written. */
  seq: number;
  /** Events that arrived before the scrollback snapshot. */
  queue: { seq: number; data: string }[] | null;
  cols: number;
  rows: number;
}

const slots = new Map<string, Slot>();

function create(repoId: string): Slot {
  const host = document.createElement("div");
  host.style.width = "100%";
  host.style.height = "100%";
  const term = new Terminal({
    fontFamily: FONT_FAMILY,
    fontSize: 12,
    lineHeight: 1.2,
    scrollback: 5000,
    cursorBlink: false,
    cursorStyle: "bar",
    cursorInactiveStyle: "none",
    theme: {
      background: TERMINAL_BG,
      foreground: "#d4d4d8",
      cursor: "#d4d4d8",
      selectionBackground: "#3f3f46",
      black: "#27272a",
      red: "#f87171",
      green: "#4ade80",
      yellow: "#fcd34d",
      blue: "#60a5fa",
      magenta: "#e879f9",
      cyan: "#67e8f9",
      white: "#e4e4e7",
      brightBlack: "#71717a",
      brightRed: "#fca5a5",
      brightGreen: "#86efac",
      brightYellow: "#fde68a",
      brightBlue: "#93c5fd",
      brightMagenta: "#f0abfc",
      brightCyan: "#a5f3fc",
      brightWhite: "#fafafa",
    },
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  // Keystrokes go to the running process; Rust ignores them when nothing runs.
  term.onData((data) => void ipc.devServerWrite(repoId, data).catch(() => {}));

  const slot: Slot = { term, fit, host, opened: false, seq: 0, queue: [], cols: 0, rows: 0 };
  ipc
    .devServerSnapshot(repoId)
    .then((snap) => {
      term.write(snap.output);
      slot.seq = snap.seq;
      for (const q of slot.queue ?? []) {
        if (q.seq > slot.seq) {
          term.write(q.data);
          slot.seq = q.seq;
        }
      }
    })
    .catch(() => {})
    .finally(() => (slot.queue = null));
  slots.set(repoId, slot);
  return slot;
}

export const terminals = {
  /** Route a `dev-server-output` event to its terminal, if one exists. */
  output(repoId: string, seq: number, data: string) {
    const slot = slots.get(repoId);
    if (!slot) return; // Created later from the snapshot, which includes this.
    if (slot.queue) {
      slot.queue.push({ seq, data });
      return;
    }
    if (seq <= slot.seq) return;
    slot.seq = seq;
    slot.term.write(data);
  },

  /** Show `repoId`'s terminal inside `container`. */
  attach(repoId: string, container: HTMLElement) {
    const slot = slots.get(repoId) ?? create(repoId);
    container.appendChild(slot.host);
    if (!slot.opened) {
      slot.term.open(slot.host);
      slot.opened = true;
      // Re-measure once the bundled font has loaded.
      document.fonts?.load(`12px ${FONT_FAMILY}`).then(() => {
        slot.term.options.fontFamily = "monospace";
        slot.term.options.fontFamily = FONT_FAMILY;
        terminals.fit(repoId);
      });
    }
  },

  detach(repoId: string) {
    slots.get(repoId)?.host.remove();
  },

  /** Fit to the container; returns the new size if it changed. */
  fit(repoId: string): { cols: number; rows: number } | null {
    const slot = slots.get(repoId);
    if (!slot?.opened || !slot.host.isConnected) return null;
    try {
      slot.fit.fit();
    } catch {
      return null;
    }
    const { cols, rows } = slot.term;
    if (cols === slot.cols && rows === slot.rows) return null;
    slot.cols = cols;
    slot.rows = rows;
    return { cols, rows };
  },

  /** Size to start a process at. */
  size(repoId: string): { cols: number; rows: number } {
    const slot = slots.get(repoId);
    return slot?.opened ? { cols: slot.term.cols, rows: slot.term.rows } : { cols: 120, rows: 30 };
  },

  focus(repoId: string) {
    slots.get(repoId)?.term.focus();
  },
};
