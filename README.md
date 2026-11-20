# Echo

Echo keeps one remote branch synced into your local checkout, so a dev server
you run locally hot-reloads as cloud coding agents push. It watches every
remote branch of a repo, shows which ones have commits you haven't seen, and
fast-forwards the branch you pick every few seconds. Think GitHub Desktop
stripped back to that one job.

Internal Coda tool. macOS (Apple Silicon and Intel) and Windows x64.

**[Download the latest release →](https://github.com/Coda-Co/echo/releases/latest)**

## How it works

1. Cloud agents each push to their own branch (`claude/...`).
2. You run the dev server from your normal checkout, either in Echo's dev
   server panel or in your own terminal.
3. Echo shows an activity dot on branches with new commits. Click one to
   switch the checkout to it and keep it fast-forwarded, or hover it and
   press **History** to see its recent commits without switching.

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

### Dev server panel

The panel under the branch list runs one command per repo from the repo root
and shows its output in a terminal, so you can watch hot-reloads without
switching windows. Type into the terminal as usual (Vite's `r` and `o`
shortcuts, prompts and Ctrl+C all work).

- **Start** with any command. The field is pre-filled with the last one you
  ran (or the `dev` script from `package.json`), so changing flags such as
  `--agents=planner,reviewer` is a quick edit.
- **Recent and pinned commands.** Press ↑/↓ or the arrow in the field to pick
  one. Pin a recent command and name it ("all agents") to keep it at the top.
- **Restart** reruns exactly the command that's running. **Stop** stops the
  server and everything it started (workers, agents), so the port is freed.
- When Echo pulls or switches branch, it writes a line such as
  `── echo · pulled 2 commits on claude/… ──` into the output, so you can see
  which push caused a reload or a crash.
- Collapse the panel to a one-line status in the footer. The sidebar shows a
  terminal icon for repos with a running server, red if it crashed.

Commands run in your login shell's environment (read once when Echo starts),
so tools set up in `.zshrc` or `.zprofile`, such as nvm and pnpm, are on the
`PATH`. Closing the window keeps servers running in the background; quitting
Echo stops them.

## Install (unsigned builds)

Download from the [latest release](https://github.com/Coda-Co/echo/releases/latest)
(older versions are on the [releases page](https://github.com/Coda-Co/echo/releases)):

- **macOS:** `Echo_<version>_universal.dmg` (Apple Silicon and Intel)
- **Windows:** `Echo_<version>_x64-setup.exe`, or the `.msi` if you prefer

The builds are unsigned, so the first launch needs one extra step:

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

Day to day, run the app from your checkout with `pnpm app:dev` rather than
reinstalling. Use the installer only for the checks listed under
[What needs an installed build](#what-needs-an-installed-build).

### One-time setup

**Windows**

1. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
   with the **Desktop development with C++** workload.
2. Install Rust with [rustup](https://rustup.rs) (the default MSVC toolchain).
3. Install Node 22, then pnpm: `npm i -g pnpm`.
4. WebView2 is already part of Windows 10 and 11.

**macOS**: install the Xcode command line tools (`xcode-select --install`),
Rust with rustup, Node 22 and pnpm.

**Linux** (CI, or if you're curious): also install the webview and tray libraries:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
```

Then, in the repo:

```bash
pnpm install
```

### Run the dev app

```bash
pnpm app:dev
```

This starts **Echo Dev**: the app built from your working tree, with its own
name, tray entry and app ID (`com.coda.echo.dev`). It keeps its settings and
seen-state in a separate folder from an installed Echo, and both can run at
the same time.

While it runs:

- **UI changes** (`src/`) appear in the open window almost instantly, without
  restarting the app.
- **Rust changes** (`src-tauri/`, `crates/echo-core/`) recompile and relaunch
  the app automatically. The first build takes a few minutes; after that,
  expect roughly 5 to 40 seconds per change.

The first `pnpm app:dev` opens an empty Echo Dev window; add a repo as usual.

`pnpm tauri dev` also works, but it runs under the installed app's ID, so it
shares the installed Echo's settings, and if Echo is already running it just
brings that window forward instead of starting.

### Ways to work on it

**With Claude Code on your own machine.** Run `pnpm app:dev` in one
terminal and Claude Code in another, in the same folder. As Claude edits files,
the dev app reloads. Run the dev app yourself rather than asking Claude to
start it: it's long-running and you'll want its output in front of you.

**With cloud agents, using Echo itself.** Install a released Echo, add your
local `echo` checkout to it, and sync the branch a cloud session is pushing to
(`claude/...`). Run `pnpm app:dev` in that checkout. Each push lands in your
checkout within seconds and Echo Dev reloads. This is the workflow Echo is
built for, so you're also testing it as you go.

### Checks

These are the same checks CI runs:

```bash
pnpm lint && pnpm typecheck && pnpm test
```

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

The Rust integration tests in `crates/echo-core/tests/integration.rs` create
real git repos in a temp folder, so they're the quickest way to check a change
to sync behaviour without clicking through the app.

### What needs an installed build

A few things only behave properly in an installed app:

- **Notifications.** On Windows the dev app's notifications may show under
  PowerShell's name or not at all, because Windows ties them to an installed
  app.
- **Launch at login.** This is turned off in dev builds on purpose, so the
  dev binary doesn't register itself to start with Windows.
- **First launch.** The SmartScreen warning, the Start menu entry and the
  installer itself.

To get installers without making a release, run the **Release** workflow
from the Actions tab and download them from the run's artefacts.

### Layout

```
crates/echo-core/     The engine. No UI dependencies, so it's testable anywhere.
  src/git.rs          run git, discovery, output parsers, stderr classification
  src/branches.rs     branch poll: ls-remote, fetch on change, unseen counts
  src/sync.rs         sync state machine: tick, switch, discard, reset, move aside
  src/core.rs         per-repo watchers (poll + sync loops), IPC operations, events
  src/devserver.rs    dev server per repo: pseudo-terminal, scrollback, stopping the process tree
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
`sync-status-changed`, `sync-event`, `flags-changed`, `repos-changed`,
`dev-server-output`, `dev-server-changed`). The only process the webview can
start is a repo's dev server command, run from the repo root; recent and
pinned commands are stored per repo in `config.json`.

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
- Notifications are informational. Activity never changes the selected repo
  or branch: it shows as dots until you pick what to look at.
- Only the `origin` remote is watched. Submodules and LFS get no special handling.
- The dev server panel runs one command per repo. On Windows commands run
  through `cmd.exe`, so double quotes inside a command may not survive.
