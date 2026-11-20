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

function iconA11y(title?: string) {
  return title ? { role: "img", "aria-label": title } : { "aria-hidden": true };
}

export function PauseIcon({ title, className = "" }: { title?: string; className?: string }) {
  return (
    <svg {...iconA11y(title)} width="12" height="12" viewBox="0 0 12 12" fill="currentColor" className={`shrink-0 ${className}`}>
      {title && <title>{title}</title>}
      <rect x="2.5" y="2" width="2.5" height="8" rx="0.5" />
      <rect x="7" y="2" width="2.5" height="8" rx="0.5" />
    </svg>
  );
}

export function PlayIcon({ title, className = "" }: { title?: string; className?: string }) {
  return (
    <svg {...iconA11y(title)} width="12" height="12" viewBox="0 0 12 12" fill="currentColor" className={`shrink-0 ${className}`}>
      {title && <title>{title}</title>}
      <path d="M3 2.2v7.6a.5.5 0 0 0 .76.43l6.2-3.8a.5.5 0 0 0 0-.86l-6.2-3.8A.5.5 0 0 0 3 2.2Z" />
    </svg>
  );
}

/** GitHub's mark (Octicons, MIT). */
export function GitHubIcon({ className = "" }: { className?: string }) {
  return (
    <svg aria-hidden width="14" height="14" viewBox="0 0 16 16" fill="currentColor" className={`shrink-0 ${className}`}>
      <path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z" />
    </svg>
  );
}

/** Pull request glyph (Octicons, MIT). */
export function PullRequestIcon({ className = "" }: { className?: string }) {
  return (
    <svg aria-hidden width="11" height="11" viewBox="0 0 16 16" fill="currentColor" className={`shrink-0 ${className}`}>
      <path d="M1.5 3.25a2.25 2.25 0 1 1 3 2.122v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.25 2.25 0 0 1 1.5 3.25Zm5.677-.177L9.573.677A.25.25 0 0 1 10 .854V2.5h1A2.5 2.5 0 0 1 13.5 5v5.628a2.251 2.251 0 1 1-1.5 0V5a1 1 0 0 0-1-1h-1v1.646a.25.25 0 0 1-.427.177L7.177 3.427a.25.25 0 0 1 0-.354ZM3.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm0 9.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm8.25.75a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Z" />
    </svg>
  );
}

/** Prompt glyph marking a repo whose dev server is running. */
export function TerminalIcon({ title, className = "" }: { title?: string; className?: string }) {
  return (
    <svg {...iconA11y(title)} width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" className={`shrink-0 ${className}`}>
      {title && <title>{title}</title>}
      <path d="M2 3.5 4.75 6 2 8.5M6.25 9h4" />
    </svg>
  );
}

export function PinIcon({ filled = false, className = "" }: { filled?: boolean; className?: string }) {
  return (
    <svg aria-hidden width="11" height="11" viewBox="0 0 12 12" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" className={`shrink-0 ${className}`}>
      <path d="M4 1.25h4l-.75 3 2 1.75h-6.5l2-1.75z" />
      <path d="M6 6v4.75" fill="none" strokeLinecap="round" />
    </svg>
  );
}

export function StopIcon({ className = "" }: { className?: string }) {
  return (
    <svg aria-hidden width="9" height="9" viewBox="0 0 10 10" fill="currentColor" className={`shrink-0 ${className}`}>
      <rect x="1" y="1" width="8" height="8" rx="1.25" />
    </svg>
  );
}

export function RestartIcon({ className = "" }: { className?: string }) {
  return (
    <svg aria-hidden width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" className={`shrink-0 ${className}`}>
      <path d="M10 6a4 4 0 1 1-1.2-2.85" />
      <path d="M9.5 1.5v2.25H7.25" />
    </svg>
  );
}

export function ChevronIcon({ up = false, className = "" }: { up?: boolean; className?: string }) {
  return (
    <svg aria-hidden width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className={`shrink-0 ${className}`}>
      <path d={up ? "M3 7.5 6 4.5l3 3" : "M3 4.5 6 7.5l3-3"} />
    </svg>
  );
}
