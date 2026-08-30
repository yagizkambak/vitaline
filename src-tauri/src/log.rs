//! Simple file logger.
//!
//! In release builds nobody sees stderr because of
//! `windows_subsystem = "windows"`. And since this is a tray app, running it
//! from a terminal isn't the usual way to launch it either. So everything
//! also gets written to a file in the config directory; if a problem gets
//! reported, we have evidence.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;

use parking_lot::Mutex;
use tauri::AppHandle;

const FILE: &str = "vitaline.log";
/// The file is truncated once it exceeds this size; it shouldn't grow unbounded.
const MAX_BYTES: u64 = 512 * 1024;

static HANDLE: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<AppHandle>> {
    HANDLE.get_or_init(|| Mutex::new(None))
}

/// Called once during app startup; entries before this only go to stderr.
pub fn init(app: &AppHandle) {
    *slot().lock() = Some(app.clone());
}

pub fn line(message: &str) {
    eprintln!("[vitaline] {message}");

    let Some(app) = slot().lock().clone() else {
        return;
    };
    let Ok(dir) = crate::config::dir(&app) else {
        return;
    };
    let path = dir.join(FILE);

    if std::fs::metadata(&path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(&path);
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(
            file,
            "{} {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            message
        );
    }
}

/// Path to the log file; shown on the settings screen.
pub fn path(app: &AppHandle) -> Option<std::path::PathBuf> {
    crate::config::dir(app).ok().map(|d| d.join(FILE))
}
