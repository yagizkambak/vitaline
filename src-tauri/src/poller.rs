//! Background refresh loop.
//!
//! Runs even when the notch window is hidden; the tray icon and notifications
//! depend on it. If `AppState::wake()` is called while waiting, the loop
//! repeats immediately (a setting changed, a token was entered, "refresh
//! now" was selected).

use tauri::{AppHandle, Manager};

use crate::refresh;
use crate::state::AppState;

pub async fn run(app: AppHandle) {
    loop {
        refresh::refresh_and_publish(&app).await;

        let (interval, wake) = {
            let state = app.state::<AppState>();
            (state.poll_interval(), state.wake.clone())
        };

        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = wake.notified() => {}
        }
    }
}
