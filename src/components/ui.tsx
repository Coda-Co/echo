import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { Badge as BadgeT, Tone } from "../lib/format";

type Variant = "default" | "primary" | "danger" | "ghost";

export function Button({
  variant = "default",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  const styles: Record<Variant, string> = {
    default:
      "border border-zinc-300 bg-white text-zinc-800 hover:bg-zinc-50 dark:border-zinc-600 dark:bg-zinc-800 dark:text-zinc-100 dark:hover:bg-zinc-700",
    primary: "border border-blue-700 bg-blue-600 text-white hover:bg-blue-700",
    danger: "border border-red-700 bg-red-600 text-white hover:bg-red-700",
    ghost: "border border-transparent text-zinc-600 hover:bg-zinc-200/70 dark:text-zinc-300 dark:hover:bg-zinc-700/70",
  };
  return (
    <button
      type="button"
      className={`inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-xs font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${styles[variant]} ${className}`}
      {...props}
    />
  );
}

const dotTone: Record<Tone | "accent", string> = {
  ok: "bg-green-500",
  busy: "bg-blue-500 animate-pulse",
  warn: "bg-amber-500",
  error: "bg-red-500",
  muted: "bg-zinc-400",
  accent: "bg-blue-500",
};

export function Dot({ tone, title, className = "" }: { tone: Tone | "accent"; title?: string; className?: string }) {
  return (
    <span
      aria-label={title}
      title={title}
      className={`inline-block h-2 w-2 shrink-0 rounded-full ${dotTone[tone]} ${className}`}
    />
  );
}

const badgeTone: Record<BadgeT["tone"], string> = {
  accent: "bg-blue-100 text-blue-800 dark:bg-blue-900/60 dark:text-blue-200",
  info: "bg-sky-100 text-sky-800 dark:bg-sky-900/60 dark:text-sky-200",
  warn: "bg-amber-100 text-amber-800 dark:bg-amber-900/60 dark:text-amber-200",
  muted: "bg-zinc-200 text-zinc-700 dark:bg-zinc-700 dark:text-zinc-200",
  ok: "bg-green-100 text-green-800 dark:bg-green-900/60 dark:text-green-200",
};

export function Badge({ badge }: { badge: BadgeT }) {
  return (
    <span
      data-testid="badge"
      className={`rounded-full px-1.5 py-px text-[10px] font-semibold uppercase tracking-wide ${badgeTone[badge.tone]}`}
    >
      {badge.label}
    </span>
  );
}

export function Spinner() {
  return (
    <span
      role="status"
      aria-label="Working"
      className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-blue-500 border-t-transparent"
    />
  );
}

export function Modal({ title, children, onClose }: { title: string; children: ReactNode; onClose: () => void }) {
  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/30 p-6"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
    >
      <div
        role="dialog"
        aria-label={title}
        className="flex max-h-full w-full max-w-lg flex-col overflow-hidden rounded-lg border border-zinc-300 bg-white shadow-xl dark:border-zinc-700 dark:bg-zinc-900"
      >
        <div className="border-b border-zinc-200 px-4 py-3 text-sm font-semibold dark:border-zinc-700">{title}</div>
        <div className="overflow-y-auto">{children}</div>
      </div>
    </div>
  );
}

export function FileList({ files, max = 12 }: { files: string[]; max?: number }) {
  const shown = files.slice(0, max);
  return (
    <ul className="max-h-40 overflow-y-auto rounded border border-zinc-200 bg-zinc-50 px-2 py-1 font-mono text-[11px] dark:border-zinc-700 dark:bg-zinc-800/60">
      {shown.map((f) => (
        <li key={f} className="truncate py-px" title={f}>
          {f}
        </li>
      ))}
      {files.length > max && <li className="py-px text-zinc-500">…and {files.length - max} more</li>}
    </ul>
  );
}

/** Two-way arrow marking the branch Echo keeps in sync with origin. */
export function SyncIcon({ title, className = "" }: { title?: string; className?: string }) {
  return (
    <svg
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${className}`}
    >
      {title && <title>{title}</title>}
      <path d="M1.5 6h9M4 3.5 1.5 6 4 8.5M8 3.5 10.5 6 8 8.5" />
    </svg>
  );
}
