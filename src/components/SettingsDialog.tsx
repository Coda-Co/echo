import { useEffect, useState } from "react";
import { ipc, type RepoConfigPatch, type SettingsPatch } from "../lib/ipc";
import { selectedRepo, useEcho } from "../lib/store";
import type { DefaultPatterns, FlagKind, RepoSummary, Settings } from "../lib/types";
import { Button, Modal } from "./ui";

export function SettingsDialog() {
  const tab = useEcho((s) => s.settingsOpen);
  const repo = useEcho(selectedRepo);
  const { openSettings } = useEcho.getState();
  if (!tab) return null;
  const current = tab === "repo" && repo ? "repo" : "global";
  return (
    <Modal title="Settings" onClose={() => openSettings(false)}>
      <div className="flex gap-1 border-b border-zinc-200 px-4 pt-2 dark:border-zinc-700">
        <Tab active={current === "global"} onClick={() => openSettings("global")}>
          General
        </Tab>
        {repo && (
          <Tab active={current === "repo"} onClick={() => openSettings("repo")}>
            {repo.config.displayName}
          </Tab>
        )}
      </div>
      {current === "global" ? <GlobalSettings /> : repo && <RepoSettings key={repo.id} repo={repo} />}
      <div className="flex justify-end border-t border-zinc-200 bg-zinc-50 px-4 py-2.5 dark:border-zinc-700 dark:bg-zinc-800/50">
        <Button variant="primary" onClick={() => openSettings(false)}>
          Done
        </Button>
      </div>
    </Modal>
  );
}

function Tab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      role="tab"
      aria-selected={active}
      onClick={onClick}
      className={`-mb-px max-w-48 truncate border-b-2 px-3 py-1.5 text-xs ${
        active ? "border-blue-600 font-semibold" : "border-transparent text-zinc-500 hover:text-zinc-800"
      }`}
    >
      {children}
    </button>
  );
}

function GlobalSettings() {
  const settings = useEcho((s) => s.settings);
  const appInfo = useEcho((s) => s.appInfo);
  const { run } = useEcho.getState();
  const [gitPath, setGitPath] = useState(settings?.gitPath ?? "");
  if (!settings) return null;

  const patch = async (p: SettingsPatch) => {
    const next = await run(() => ipc.updateSettings(p));
    if (next) useEcho.setState({ settings: next, appInfo: await ipc.appInfo() });
  };
  const notif = (k: keyof Settings["notifications"], v: boolean) =>
    patch({ notifications: { ...settings.notifications, [k]: v } });

  return (
    <div className="space-y-4 px-4 py-3 text-xs">
      <Section title="Startup">
        <Check label="Launch Echo at login" checked={settings.launchAtLogin} onChange={(v) => patch({ launchAtLogin: v })} />
        <Check
          label="Start hidden in the tray when launched at login"
          checked={settings.startHidden}
          onChange={(v) => patch({ startHidden: v })}
        />
      </Section>
      <Section title="Notifications">
        <Check label="Synced branch pulled" checked={settings.notifications.pulled} onChange={(v) => notif("pulled", v)} />
        <Check label="Synced branch force-pushed and reset" checked={settings.notifications.forcePushed} onChange={(v) => notif("forcePushed", v)} />
        <Check label="Sync blocked or failed" checked={settings.notifications.blocked} onChange={(v) => notif("blocked", v)} />
        <Check label="New branch appeared on the remote" checked={settings.notifications.newBranch} onChange={(v) => notif("newBranch", v)} />
      </Section>
      <Section title="Polling">
        <NumberField
          label="Check all branches every"
          suffix="seconds"
          min={5}
          value={settings.branchPollSeconds}
          onCommit={(v) => patch({ branchPollSeconds: v })}
        />
        <NumberField
          label="Check the synced branch every"
          suffix="seconds (slows down when quiet)"
          min={1}
          value={settings.syncBaseSeconds}
          onCommit={(v) => patch({ syncBaseSeconds: v })}
        />
      </Section>
      <Section title="Git">
        <label className="block">
          <span className="text-zinc-500">Git executable (leave empty to detect)</span>
          <input
            value={gitPath}
            placeholder={appInfo?.gitPath ?? "/usr/bin/git"}
            onChange={(e) => setGitPath(e.target.value)}
            onBlur={() => gitPath !== (settings.gitPath ?? "") && patch({ gitPath: gitPath.trim() || undefined })}
            className="mt-1 h-7 w-full rounded-md border border-zinc-300 bg-white px-2 font-mono outline-none focus:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800"
          />
        </label>
        <p className={appInfo?.gitError ? "text-red-600" : "text-zinc-500"}>
          {appInfo?.gitError ?? `Using git ${appInfo?.gitVersion} at ${appInfo?.gitPath}`}
        </p>
      </Section>
      <p className="text-zinc-400">Echo {appInfo?.version}</p>
    </div>
  );
}

function RepoSettings({ repo }: { repo: RepoSummary }) {
  const { run } = useEcho.getState();
  const [defaults, setDefaults] = useState<DefaultPatterns | null>(null);
  useEffect(() => {
    ipc.defaultFlagPatterns().then(setDefaults).catch(() => {});
  }, []);
  const c = repo.config;
  const patch = (p: RepoConfigPatch) => run(() => ipc.updateRepoConfig(repo.id, p));

  const patternField = (kind: FlagKind, label: string) => {
    const custom = c.flagPatterns?.[kind];
    const value = (custom ?? defaults?.[kind] ?? []).join("\n");
    return (
      <TextArea
        key={kind}
        label={`${label} patterns${custom ? "" : " (defaults)"}`}
        value={value}
        onCommit={(v) => {
          const list = v
            .split("\n")
            .map((s) => s.trim())
            .filter(Boolean);
          const isDefault = defaults && list.join("\n") === defaults[kind].join("\n");
          const next = { ...(c.flagPatterns ?? {}), [kind]: isDefault ? undefined : list };
          void patch({ flagPatterns: next });
        }}
      />
    );
  };

  return (
    <div className="space-y-4 px-4 py-3 text-xs">
      <p className="break-all font-mono text-[11px] text-zinc-500">{c.path}</p>
      <Section title="Watching">
        <Check label="Poll this repo for new commits" checked={c.pollingEnabled} onChange={(v) => patch({ pollingEnabled: v })} />
      </Section>
      <Section title="When the synced branch is force-pushed">
        <Radio
          name="fp"
          options={[
            ["reset", "Reset to the new remote tip and notify (local has no commits of its own)"],
            ["ask", "Ask first"],
          ]}
          value={c.forcePushPolicy}
          onChange={(v) => patch({ forcePushPolicy: v as "reset" | "ask" })}
        />
      </Section>
      <Section title="Commands (shown on change flags, click to copy)">
        <TextField
          label="Install command"
          placeholder={repo.installCommand ?? "pnpm install"}
          value={c.installCommand ?? ""}
          onCommit={(v) => patch({ installCommand: v || undefined })}
        />
        <TextField
          label="Migrate command"
          placeholder="pnpm db:migrate"
          value={c.migrateCommand ?? ""}
          onCommit={(v) => patch({ migrateCommand: v || undefined })}
        />
      </Section>
      <Section title="Change flags (one glob per line)">
        {patternField("deps", "Dependencies")}
        {patternField("migrations", "Migrations")}
        {patternField("env", "Env template")}
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <fieldset className="space-y-1.5">
      <legend className="mb-1 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">{title}</legend>
      {children}
    </fieldset>
  );
}

function Check({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex items-center gap-2">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {label}
    </label>
  );
}

function Radio(props: { name: string; options: [string, string][]; value: string; onChange: (v: string) => void }) {
  return (
    <>
      {props.options.map(([v, label]) => (
        <label key={v} className="flex items-center gap-2">
          <input type="radio" name={props.name} checked={props.value === v} onChange={() => props.onChange(v)} />
          {label}
        </label>
      ))}
    </>
  );
}

const inputCls =
  "h-7 rounded-md border border-zinc-300 bg-white px-2 outline-none focus:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800";

function NumberField(props: { label: string; suffix: string; min: number; value: number; onCommit: (v: number) => void }) {
  const [v, setV] = useState(String(props.value));
  useEffect(() => setV(String(props.value)), [props.value]);
  const commit = () => {
    const n = Math.max(props.min, Math.round(Number(v)));
    if (Number.isFinite(n) && n !== props.value) props.onCommit(n);
    else setV(String(props.value));
  };
  return (
    <label className="flex items-center gap-2">
      {props.label}
      <input type="number" min={props.min} value={v} onChange={(e) => setV(e.target.value)} onBlur={commit} className={`${inputCls} w-16`} />
      <span className="text-zinc-500">{props.suffix}</span>
    </label>
  );
}

function TextField(props: { label: string; placeholder?: string; value: string; onCommit: (v: string) => void }) {
  const [v, setV] = useState(props.value);
  useEffect(() => setV(props.value), [props.value]);
  return (
    <label className="block">
      <span className="text-zinc-500">{props.label}</span>
      <input
        value={v}
        placeholder={props.placeholder}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => v.trim() !== props.value && props.onCommit(v.trim())}
        className={`${inputCls} mt-1 w-full font-mono`}
      />
    </label>
  );
}

function TextArea(props: { label: string; value: string; onCommit: (v: string) => void }) {
  const [v, setV] = useState(props.value);
  useEffect(() => setV(props.value), [props.value]);
  return (
    <label className="block">
      <span className="text-zinc-500">{props.label}</span>
      <textarea
        rows={4}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => v !== props.value && props.onCommit(v)}
        className="mt-1 w-full rounded-md border border-zinc-300 bg-white px-2 py-1 font-mono text-[11px] outline-none focus:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800"
      />
    </label>
  );
}
