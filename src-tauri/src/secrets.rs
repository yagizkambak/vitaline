//! Where provider tokens are stored.
//!
//! The OS keychain is tried first (macOS Keychain, Windows Credential
//! Manager). If it's not reachable, we fall back to a file in the config
//! directory -- not ideal, but better than losing the token entirely.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tauri::AppHandle;

use crate::model::ProviderKind;

const SERVICE: &str = "vitaline";

/// Keychain account name. The old name is kept for GitLab so the token
/// already saved on existing installs isn't lost.
fn account(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Gitlab => "gitlab-token",
        ProviderKind::Github => "github-token",
        ProviderKind::Azure => "azure-token",
    }
}

fn fallback_file(kind: ProviderKind) -> &'static str {
    match kind {
        // File name from an older version; kept as-is for backward compatibility.
        ProviderKind::Gitlab => "token.txt",
        ProviderKind::Github => "token-github.txt",
        ProviderKind::Azure => "token-azure.txt",
    }
}

fn entry(kind: ProviderKind) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(SERVICE, account(kind))
}

fn fallback_path(app: &AppHandle, kind: ProviderKind) -> Option<PathBuf> {
    crate::config::dir(app)
        .ok()
        .map(|d| d.join(fallback_file(kind)))
}

pub fn get(app: &AppHandle, kind: ProviderKind) -> Option<String> {
    if let Ok(e) = entry(kind) {
        match e.get_password() {
            Ok(token) if !token.is_empty() => return Some(token),
            // NoEntry is normal; for other errors we fall back to the file.
            _ => {}
        }
    }
    let path = fallback_path(app, kind)?;
    let token = fs::read_to_string(path).ok()?.trim().to_string();
    (!token.is_empty()).then_some(token)
}

pub fn set(app: &AppHandle, kind: ProviderKind, token: &str) -> Result<()> {
    if let Ok(e) = entry(kind) {
        if e.set_password(token).is_ok() {
            // If the keychain works, don't leave a stale file fallback behind.
            if let Some(path) = fallback_path(app, kind) {
                let _ = fs::remove_file(path);
            }
            return Ok(());
        }
    }
    let path = fallback_path(app, kind)
        .ok_or_else(|| anyhow::anyhow!("No directory found to save the token to"))?;
    fs::write(&path, token)?;
    restrict(&path);
    Ok(())
}

pub fn clear(app: &AppHandle, kind: ProviderKind) -> Result<()> {
    if let Ok(e) = entry(kind) {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => return Err(err.into()),
        }
    }
    if let Some(path) = fallback_path(app, kind) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// Keep the file fallback readable only by the current user, as far as possible.
#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

/// Windows: even though the file sits under the user's profile by default,
/// it still inherits the parent directory's NTFS ACL -- other accounts on
/// the same machine, or a misconfigured share, could read it. `icacls`
/// strips the inheritance and grants access to only the current user; if the
/// command isn't found or fails, this is skipped silently since the token
/// was already written (the keychain is the primary path, this is just
/// defense in depth).
#[cfg(windows)]
fn restrict(path: &Path) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // `%USERNAME%` only expands when cmd.exe parses it itself; since we call
    // icacls directly (no shell), we resolve the actual username ourselves
    // here -- more reliable, and it never touches a shell, so there's no
    // extra escaping/injection surface.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(path) = path.to_str() else { return };
    let Ok(user) = std::env::var("USERNAME") else {
        return;
    };
    let grant = format!("{user}:(R,W)");
    let _ = Command::new("icacls")
        .args([path, "/inheritance:r", "/grant:r", &grant])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(any(unix, windows)))]
fn restrict(_path: &Path) {}
