use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use tokio::sync::Notify;

use crate::model::{AppConfig, ProviderKind, Snapshot};

/// Shared state that lives for the whole app lifetime. Held by Tauri via `manage`.
pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub snapshot: RwLock<Snapshot>,
    /// Provider -> token. A provider with no token entered is absent from the map.
    pub tokens: RwLock<HashMap<ProviderKind, String>>,
    /// Provider -> verified username.
    pub usernames: RwLock<HashMap<ProviderKind, String>>,
    /// Project id -> last reported status. Avoids repeating notifications.
    pub last_status: Mutex<HashMap<String, String>>,
    /// Project id -> previously seen MR iids. Only newly opened ones are notified.
    pub seen_merge_requests: Mutex<HashMap<String, HashSet<u64>>>,
    pub http: reqwest::Client,
    /// Wakes the poll loop without waiting (a setting changed, manual refresh).
    pub wake: Arc<Notify>,
    /// Has the notch window been shown once at startup yet? Windows/Linux only;
    /// see `commands::set_notch_size`. macOS keeps calling `window.show()`
    /// directly in `setup()`, this field is never read there.
    ///
    /// On Windows, calling `window.show()` right in `setup()` runs before the
    /// WebView2 controller has attached: the HWND gets marked "visible" but
    /// the webview surface never picks that up, leaving the panel permanently
    /// blank. The first real reveal happens once the frontend has loaded and
    /// calls `set_notch_size` -- the IPC call succeeding is proof the webview
    /// is actually ready.
    #[cfg(not(target_os = "macos"))]
    pub notch_revealed: AtomicBool,
}

impl AppState {
    pub fn new(config: AppConfig, tokens: HashMap<ProviderKind, String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default();

        Self {
            config: RwLock::new(config),
            snapshot: RwLock::new(Snapshot::default()),
            tokens: RwLock::new(tokens),
            usernames: RwLock::new(HashMap::new()),
            last_status: Mutex::new(HashMap::new()),
            seen_merge_requests: Mutex::new(HashMap::new()),
            http,
            wake: Arc::new(Notify::new()),
            #[cfg(not(target_os = "macos"))]
            notch_revealed: AtomicBool::new(false),
        }
    }

    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.config.read().poll_seconds.clamp(5, 3600))
    }

    /// Re-run the poll loop immediately.
    pub fn wake(&self) {
        self.wake.notify_waiters();
    }
}
