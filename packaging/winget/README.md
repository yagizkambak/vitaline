# winget package

The manifest files don't live in THIS repo: they're submitted as a PR to the
`microsoft/winget-pkgs` repo, and the file path changes with the version
(`manifests/y/yagizkambak/Vitaline/<version>/`). Use `wingetcreate` instead of
hand-writing YAML -- it updates the manifest itself as the schema versions change.

## Package identity

    yagizkambak.Vitaline

Chosen once, then it CANNOT be changed (changing it means a new package).

## First submission

After the release is published, on Windows:

```powershell
winget install Microsoft.WingetCreate
wingetcreate new https://github.com/yagizkambak/vitaline/releases/download/v0.1.0/Vitaline_0.1.0_x64-setup.exe
```

`wingetcreate` downloads the installer, computes its hash, detects the
installer type (NSIS), and asks for any missing fields. At the end it opens
the PR itself; it asks for a GitHub token.

Fields to fill in:

| field | value |
|---|---|
| PackageIdentifier | `yagizkambak.Vitaline` |
| PackageName | Vitaline |
| Publisher | yagizkambak |
| License | (the repo's license) |
| ShortDescription | Shows GitLab/GitHub/Azure pipeline status at the top of the screen |
| PackageUrl | https://github.com/yagizkambak/vitaline |

## Later releases

```powershell
wingetcreate update yagizkambak.Vitaline `
  --version 0.2.0 `
  --urls https://github.com/yagizkambak/vitaline/releases/download/v0.2.0/Vitaline_0.2.0_x64-setup.exe `
  --submit
```

## Things worth knowing

- **Signing isn't required.** winget accepts unsigned installers, but the
  user gets a SmartScreen warning ("More info" > "Run anyway"). The only way
  to remove the warning is getting an Authenticode code signing certificate.
- **There's automated verification.** Once the PR opens, Microsoft's pipeline
  actually runs and tests the installer; the PR is rejected if silent install
  (`/S`) isn't supported or the installer requires user interaction. The NSIS
  package Tauri produces supports silent install.
- **The first PR is reviewed by hand**, later version updates usually go through automatically.
- CI only produces the `.exe` (NSIS) — the `.msi` (WiX) build was dropped
  from `.github/workflows/release.yml` because `light.exe` fails
  consistently on the GitHub Actions Windows runner image. NSIS was already
  the better fit for winget anyway: it installs per-user and doesn't
  require administrator privileges.
