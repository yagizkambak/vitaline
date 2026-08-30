# Cutting a release

Packages are produced by `.github/workflows/release.yml`: pushing a tag
starting with `v` builds the macOS (universal DMG) and Windows (NSIS `.exe`)
packages and uploads them to GitHub Releases as a **draft**. The Windows
`.msi` (WiX) build is intentionally not produced by CI — `light.exe` failed
consistently on the GitHub Actions Windows runner image, reproduced twice in
a row. `.exe` is what winget and the release notes point users at anyway; a
local `npm run app:build` still produces both if you ever need the `.msi`.

## Steps

1. **Bump the version.** Must match in three places:
   - `package.json` > `version`
   - `src-tauri/Cargo.toml` > `version`
   - `src-tauri/tauri.conf.json` > `version`

2. **Push the tag.**

   ```
   git tag v0.2.0 && git push origin v0.2.0
   ```

3. **Check the draft release.** Download the packages and try installing on
   both platforms, then publish the release. The Homebrew cask and winget
   manifest use the download URLs of the published release — don't skip this
   ordering.

4. **Update the Homebrew tap** (`github.com/yagizkambak/homebrew-tap`):

   ```
   shasum -a 256 "Vitaline_0.2.0_universal.dmg"
   ```

   Update the `version` and `sha256` values in `packaging/homebrew/vitaline.rb`
   and copy it over `Casks/vitaline.rb` in the tap repo.

5. **Update the winget manifest** — see `packaging/winget/README.md`.

## Signing status

The app ships **unsigned**. Consequences:

| platform | what the user sees |
|---|---|
| macOS | Gatekeeper says "damaged, move to Trash"; needs `xattr -cr` |
| Windows | SmartScreen shows an "unrecognized app" warning |

The only way to fully fix this on macOS is signing and notarizing with an
**Apple Developer ID** ($99/yr). Homebrew also removes unsigned casks from
the official tap (deadline September 2026) — that's why we can't get into
the official tap and use our own instead.

If a certificate is obtained later: give `tauri-action` the
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` secrets and add a signing +
notarization step to the workflow; nothing else needs to change.

## Setting up the Homebrew tap for the first time

One-time: create a repo named `homebrew-tap`, add a `Casks/` folder in it,
and copy `packaging/homebrew/vitaline.rb` there. From the user's side:

```
brew tap yagizkambak/tap
brew trust --cask yagizkambak/tap/vitaline
brew install --cask vitaline
```

Since Homebrew 6, third-party taps must be explicitly **trusted** —
`brew trust --cask` above does that. The app is unsigned and Homebrew
removed `--no-quarantine` in 4.7, so it won't open right after install;
run the `xattr -cr` command from the Signing status table above.
