# Contributing to Vitaline

Thanks for considering a contribution. This is a solo side project, but
issues and pull requests are genuinely welcome.

## Getting set up

See the [Setup](README.md#setup) and [Running it](README.md#running-it)
sections of the README — `npm install`, then `npm run app` for a dev build
with hot reload.

## Before opening a PR

```bash
cd src-tauri && cargo test && cargo clippy -- -D warnings
cd .. && npx tsc --noEmit
```

All three should be clean. There's no separate JS/TS test suite yet — the
Rust side (where all the provider/HTTP logic lives) is what's covered.

## Code style

- Comments and user-facing strings (errors, notifications, UI copy) are in
  English — including test names and inline test data. This project doesn't
  do partial translations; keep it that way.
- Rust code follows `rustfmt` defaults (`cargo fmt`); there's a
  `rustfmt.toml` at the repo root of `src-tauri/`.
- Windows-specific fixes should be `#[cfg]`-gated or CSS-scoped so they
  don't change macOS behavior, and vice versa — the app shares one codebase
  across both platforms on purpose.

## Adding a provider

Each provider (GitLab, GitHub, Azure DevOps) is one module in
`src-tauri/src/` that maps its API's shape onto the shared model in
`model.rs` (`PipelineInfo`, `StageInfo`, `JobInfo`, `MergeRequestInfo`).
`providers.rs` is the dispatch layer that routes a call to the right client
and returns a user-facing error for operations a provider doesn't support.
Look at `github.rs` for the smallest example to copy from.

## Regenerating the app icon

`scripts/make-icon.mjs` is a dependency-free PNG generator that draws the
app icon (`src-tauri/icons/app-icon.png`) from scratch — no image editor
needed. Run it with `node scripts/make-icon.mjs`, then run `npx tauri icon
src-tauri/icons/app-icon.png` to regenerate every platform size from the
new 1024×1024 source.

## Reporting a bug

Please include your OS, the app version, and — if it's provider-related —
which of GitLab/GitHub/Azure DevOps. The log file
(`%APPDATA%\dev.vitaline.desktop\vitaline.log` on Windows, shown at the
bottom of the Settings window on either platform) usually has the actual
error.

## A note on how this project is built

Vitaline is developed with [Claude Code](https://claude.com/claude-code) as
a regular part of the workflow, not just for one-off snippets. If you send a
PR that was also written with AI assistance, that's fine — just make sure
you've actually run it and understand what it does before opening the PR.
