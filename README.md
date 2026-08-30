![Vitaline](.github/assets/logo.png)

A desktop app that shows CI pipeline status as a notch-style HUD at the top
center of your screen. Watches **GitLab, GitHub Actions, and Azure DevOps**
at the same time; runs from the same codebase on Windows and macOS.

Collapsed, it's just a pill: a colored status dot, project counters, and the
count of open merge requests. Hover over it and it opens downward, showing
each watched project's latest pipeline, its stage breakdown, its jobs, and
its open MRs; from there you can retry or cancel jobs, manually start a job,
and read the log tail of a failed job.

When a **new merge request / PR opens** on a watched repo, you get a desktop
notification, and the same announcement scrolls as **ticker text** inside the
notch (click it to open in the browser). The MRs already open when the app
first launches are recorded silently — MRs opened months ago aren't
re-announced on every startup. A short announcement scrolls the same way when
a pipeline breaks or recovers.

## Provider support matrix

| | GitLab | GitHub Actions | Azure DevOps |
|---|---|---|---|
| Latest pipeline + stage/job | ✅ | ✅ (no stages → single "workflow" group) | ✅ (Timeline API) |
| Pipeline retry / cancel | ✅ | ✅ | ✅ |
| Job retry | ✅ | ✅ | ❌ |
| Job cancel / manual start | ✅ | ❌ | ❌ |
| Job log | ✅ | ✅ | ✅ |
| Open MR/PR + notification | ✅ | ✅ | ✅ |

Buttons the provider doesn't support never show up in the UI. Project id
formats — GitLab: `12345` or `group/project`; GitHub: `owner/repo`; Azure:
`Project` or `Project/DefinitionId` (the definition id is the `definitionId`
in the pipeline's URL).

```
        ┌──────────────┐
────────┤  ● 3 1 ✗ 1   ├────────    ← collapsed (opens on hover)
        └──────────────┘
```

## Screenshots

Taken from my own day-to-day watch list, as example data.

| | |
|---|---|
| ![Multiple watched projects, each with its stage breakdown](.github/assets/screenshot-1.png) | ![A project's stages expanded down to individual jobs](.github/assets/screenshot-2.png) |
| ![Reading a job's log tail without leaving the app](.github/assets/screenshot-3.png) | ![A new merge request shows up as its own card](.github/assets/screenshot-4.png) |

## How it works

- **Rust (`src-tauri/`)** — All HTTP traffic happens here. The webview never
  sees the token, and there's no CORS to deal with. A poll loop runs in the
  background; the tray icon and notifications keep updating even while the
  notch is hidden.
- **React (`src/`)** — Display only. Listens for the `pipelines://updated`
  event Rust publishes, sends actions back via `invoke`.
- **Window size** — The frontend measures the panel's real height and
  reports it to Rust via `set_notch_size`; Rust resizes the window to that
  size and re-centers it at the top of the screen. Opening/closing works
  entirely through this one mechanism.
- **Tokens** — Stored in the OS keychain (macOS Keychain, Windows Credential
  Manager). Falls back to a file in the config directory if the keychain
  isn't reachable.

## Setup

Requirements:

- Node.js 18+
- Rust toolchain (`rustup`)
- **Windows:** WebView2 Runtime (comes pre-installed on Windows 11) and the
  C++ component of Visual Studio Build Tools
- **macOS:** Xcode Command Line Tools (`xcode-select --install`)

If you don't have Rust:

```bash
winget install Rustlang.Rustup
```

On macOS/Linux:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then install dependencies:

```bash
npm install
```

## Running it

Dev mode (Vite + Tauri together):

```bash
npm run app
```

A distributable package (`.msi`/`.exe` on Windows, `.app`/`.dmg` on macOS):

```bash
npm run app:build
```

If you just want to run the web side, `npm run dev` — but `invoke` calls
don't work without Tauri, so the window shows up empty.

### Two gotchas when running it

**"Port 1420 is already in use"** — a vite server left over from a previous
run. Tauri doesn't always take the vite process it spawned down with it when
it closes:

```bash
npx kill-port 1420
```

**"failed to remove file ... vitaline.exe / Access is denied"** — the app is
still running, so cargo can't overwrite the binary. Click Quit from the tray
menu, or:

```bash
taskkill /IM vitaline.exe /F
```

## First-time setup

On first launch there's no watched project yet, so the Settings window opens
on its own. Enter tokens for the providers you'll use (each is stored
separately in the OS keychain), then add projects by picking a provider.

**Token scopes:**

- **GitLab** — Preferences → Access tokens. `read_api` to watch;
  `api` to retry/cancel/start. With only `read_api`, action buttons return
  **403** — the error shows up as a persistent banner above the panel.
- **GitHub** — Settings → Developer settings → Personal access tokens.
  Fine-grained: `Actions (read/write)` + `Pull requests (read)`;
  for a classic token, the `repo` scope.
- **Azure DevOps** — User settings → Personal access tokens.
  `Build (Read & execute)` + `Code (Read)`. Also enter the organization
  address: `https://dev.azure.com/organization`.

If the branch field is left empty, the project's most recent pipeline is watched.

## Usage

| What | How |
|---|---|
| Open | Hover over the pill |
| Keep it open | Click the pill (or "Pin") |
| Hide | "Hide" in the panel, bring it back from the tray menu |
| Filter by stage | Click a segment in the stage bar |
| Job log | "Log" on the job row |
| Open in GitLab | Click the project name or the ↗ icon on a job row |
| See open MRs | "Merge request N" on the card |

In the tray menu: show/hide, refresh now, settings, quit.

## Platform notes

**macOS** — `alwaysOnTop` alone stays below the menu bar. For the notch to
actually sit inside the physical notch, the window level is raised to
`NSStatusWindowLevel`; this is the project's only Objective-C touchpoint,
isolated in the `raise_above_menu_bar` function in
[`src-tauri/src/notch.rs`](src-tauri/src/notch.rs). If it fails to compile,
emptying its body is enough — the notch keeps working, it just stays below
the menu bar.

The packaged `.app` doesn't show a Dock icon (`Info.plist` → `LSUIElement`).
The Dock icon does show up during `npm run app`; that's expected.

**Windows** — Since there's no physical notch, the panel just looks like a
bar stuck to the top center of the screen. Bumping the "offset from the top
edge" setting up a bit (e.g. 4-8 px) usually looks better.

## Troubleshooting

Since the app lives in the tray, release builds have no console; everything
is written to a log file instead:

```
%APPDATA%\dev.vitaline.desktop\vitaline.log
```

The same path is also shown at the bottom of the Settings window. If
something goes wrong, check there first.

Only a **single instance** of the app runs at a time; launching it a second
time brings the existing notch to the front. To quit, use the **Quit** button
in the notch panel or **Quit** in the tray menu — closing windows doesn't
terminate the app, that's intentional.

## Known limits

- Only the first 100 jobs per pipeline and the first 20 open MR/PRs per
  project are fetched; larger lists get truncated.
- On GitHub, run duration is approximated from `run_started_at → updated_at`
  (the API doesn't return a duration directly).
- On Azure, a job's numeric id is its log id; a job whose log doesn't exist
  yet (hasn't started) can't have its log opened.
- Stage order is derived from each job's **first attempt's** id (older
  retries are fetched too). Since jobs are created in stage order, this
  reflects the pipeline's real flow, and retrying a job doesn't change the
  ordering. This is a heuristic because GitLab's official stage order doesn't
  come from a separate REST endpoint.
- While the notch is open, the window grows, so the apps underneath can't be
  clicked in that area. While closed, it only takes up as much space as the pill.

## Tests

```bash
cd src-tauri && cargo test
```

There are unit tests for the status aggregation logic (`aggregate`), log
tail truncation, and project path encoding.

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).

## About

Built by [Yağız Küçükkambak](https://github.com/yagizkambak). Claude Code
was a pair-programming collaborator throughout — from the provider
integrations and the notch animation work down to this README.
