# Echo

Echo keeps one remote branch synced into your local checkout, so a dev server
you run locally hot-reloads as cloud coding agents push. It watches every
remote branch of a repo, shows which ones have commits you haven't seen, and
fast-forwards the branch you pick every few seconds. Think GitHub Desktop
stripped back to that one job.

Internal Coda tool. macOS (Apple Silicon and Intel) and Windows x64.

## How it works

1. Cloud agents each push to their own branch (`claude/...`).
2. You run the dev server from your normal checkout, in your own terminal.
3. Echo shows an activity dot on branches with new commits. Double-click one
   to switch the checkout to it and keep it fast-forwarded.

Echo never silently loses work. If the checkout has local changes, local
commits, or untracked files in the way, sync stops and the sync bar explains
why and offers a fix (discard, move files aside, or reset to the remote), each
behind a confirmation. Untracked files are never deleted: "Move files aside"
renames them to `<name>.echo-<timestamp>`.

**Assumption:** you don't edit files in a repo while Echo is syncing it. Echo
operates on the repo you add, not a separate worktree.

After a pull or switch that touches dependency manifests, migrations or
`.env.example`, Echo raises a flag (click it to copy `pnpm install` or
whatever your install command is).

## Install (unsigned builds)

Download the `.dmg` (macOS) or `.msi` / `-setup.exe` (Windows) from the
GitHub release.

- **macOS:** the app isn't notarised. On first launch, right-click Echo in
  Applications and choose **Open**, or run:
  ```bash
  xattr -dr com.apple.quarantine "/Applications/Echo.app"
  ```
- **Windows:** SmartScreen will warn. Click **More info**, then **Run anyway**.

Echo needs git 2.23 or newer. On macOS it looks in `/opt/homebrew/bin`,
`/usr/local/bin`, then `/usr/bin`; set a path in Settings if yours is elsewhere.
It uses your normal git credentials (osxkeychain, Git Credential Manager, SSH
config) and never prompts for a password; if auth fails, it pauses that repo
and tells you.

## Develop

Prerequisites: Rust (stable), Node 22, pnpm 9. On Linux you also need the
webview and tray libraries:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
```

```bash
pnpm install
```

```bash
pnpm tauri dev
```

Checks (the same ones CI runs):

```bash
pnpm lint && pnpm typecheck && pnpm test
```

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

### Layout

```
crates/echo-core/     The engine. No UI dependencies, so it's testable anywhere.
  src/git.rs          run git, discovery, output parsers, stderr classification
  src/branches.rs     branch poll: ls-remote, fetch on change, unseen counts
  src/sync.rs         sync state machine: tick, switch, discard, reset, move aside
  src/core.rs         per-repo watchers (poll + sync loops), IPC operations, events
  src/flags.rs        dependency / migration / env change flags
  src/store.rs        config.json and state.json
  src/interval.rs     adaptive sync interval
  src/notify.rs       notification rate limiting
  tests/integration.rs  real git repos: bare origin, an "agent" clone, the synced clone
src-tauri/            Tauri shell: commands, tray, notifications, autostart
src/                  React + TypeScript UI (Vite, Tailwind, Zustand)
```

All git work runs in Rust through the system `git` CLI. The UI calls typed
commands (`src/lib/ipc.ts`) and listens for events (`branches-updated`,
`sync-status-changed`, `sync-event`, `flags-changed`, `repos-changed`). No
shell access is exposed to the webview.

Settings and seen-state live in the app data directory as `config.json` and
`state.json` (macOS: `~/Library/Application Support/com.coda.echo`, Windows:
`%APPDATA%\com.coda.echo`). Logs go to the platform log directory.

## Release

Push a `v*` tag. GitHub Actions builds a universal macOS `.dmg` and Windows
`.msi` / NSIS installer and attaches them to a draft release. Run the
Release workflow manually to get installers as build artefacts without a
release.

## Known limitations

- Builds are unsigned and there's no auto-update yet.
- Clicking a notification brings Echo forward on the repo it was about only
  when the OS activates the app on click (macOS does; Windows varies). Tauri's
  desktop notifications don't report clicks.
- Only the `origin` remote is watched. Submodules and LFS get no special handling.
