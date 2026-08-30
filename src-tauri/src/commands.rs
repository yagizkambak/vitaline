//! Commands the frontend calls via `invoke`.
//!
//! All of them return the error as a `String`; on the JS side `errorText()`
//! shows it directly.

use tauri::{AppHandle, Manager, State};

use crate::model::{AppConfig, ProviderKind, Snapshot, TokenState, TokenStates};
use crate::providers::{self, Client};
use crate::state::AppState;
use crate::{config, notch, refresh, secrets};

/// Writes the error to both the terminal and the frontend. While running
/// `npm run app` we want a failed action to leave a trace in the terminal;
/// otherwise the only evidence is a small red line that vanishes in the panel.
fn fail<E: std::fmt::Display>(context: &str, err: E) -> String {
    let message = format!("{context}: {err}");
    crate::log::line(&message);
    message
}

fn parse_provider(raw: &str) -> Result<ProviderKind, String> {
    ProviderKind::parse(raw).ok_or_else(|| format!("Unknown provider: {raw}"))
}

/// Client ready for the watched project's provider. Returns an error if the
/// project isn't in the list (e.g. removed from settings).
fn client_for_project(app: &AppHandle, project_id: &str) -> Result<Client, String> {
    let state = app.state::<AppState>();
    let kind = state
        .config
        .read()
        .watched
        .iter()
        .find(|p| p.id == project_id)
        .map(|p| p.provider)
        .ok_or_else(|| format!("Project isn't in the watched list: {project_id}"))?;
    providers::client_for(&state, kind).map_err(|e| fail("Client could not be prepared", e))
}

// ------------------------------------------------------------------ settings --

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.config.read().clone()
}

#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<AppConfig, String> {
    let clean = config.sanitized();
    crate::log::line(&format!(
        "save_config started: gitlab_url={}, watched={}",
        clean.gitlab_url,
        clean.watched.len()
    ));

    {
        let state = app.state::<AppState>();
        *state.config.write() = clean.clone();
    }
    config::save(&app, &clean).map_err(|e| fail("Settings could not be saved", e))?;

    // Window behavior depends on the settings; apply it right away.
    if let Some(window) = notch::window(&app) {
        notch::apply_behaviour(&window, clean.show_on_all_spaces);
        let size = window.inner_size().ok();
        let scale = window.scale_factor().unwrap_or(1.0);
        if let Some(size) = size {
            notch::place(
                &window,
                (size.width as f64 / scale).round() as u32,
                (size.height as f64 / scale).round() as u32,
                clean.top_offset,
            );
        }
    }

    app.state::<AppState>().wake();
    crate::log::line("save_config finished");
    Ok(clean)
}

// -------------------------------------------------------------------- token --

fn token_state_of(state: &AppState, kind: ProviderKind) -> TokenState {
    TokenState {
        present: state.tokens.read().contains_key(&kind),
        username: state.usernames.read().get(&kind).cloned(),
    }
}

#[tauri::command]
pub fn get_token_states(state: State<'_, AppState>) -> TokenStates {
    TokenStates {
        gitlab: token_state_of(&state, ProviderKind::Gitlab),
        github: token_state_of(&state, ProviderKind::Github),
        azure: token_state_of(&state, ProviderKind::Azure),
    }
}

#[tauri::command]
pub async fn set_token(
    app: AppHandle,
    provider: String,
    token: String,
) -> Result<TokenState, String> {
    let kind = parse_provider(&provider)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Token cannot be empty.".to_string());
    }
    crate::log::line(&format!("set_token started: provider={}", kind.label()));

    // Verify before saving: don't silently store a wrong token. The token is
    // placed in state temporarily; it's rolled back if verification fails.
    let previous = {
        let state = app.state::<AppState>();
        // Drop the result into an intermediate variable to release the guard
        // before `state` goes out of scope.
        let previous = state.tokens.write().insert(kind, token.clone());
        previous
    };

    let verification = {
        let state = app.state::<AppState>();
        match providers::client_for(&state, kind) {
            Ok(client) => client.current_user().await.map_err(|e| e.to_string()),
            Err(message) => Err(message),
        }
    };

    let username = match verification {
        Ok(username) => username,
        Err(err) => {
            // Roll back the token that failed verification.
            let state = app.state::<AppState>();
            let mut tokens = state.tokens.write();
            match previous {
                Some(old) => {
                    tokens.insert(kind, old);
                }
                None => {
                    tokens.remove(&kind);
                }
            }
            return Err(fail("Token could not be verified", err));
        }
    };

    secrets::set(&app, kind, &token).map_err(|e| fail("Token could not be saved", e))?;

    let state = app.state::<AppState>();
    state.usernames.write().insert(kind, username.clone());
    state.wake();
    crate::log::line(&format!("set_token finished: provider={}", kind.label()));

    Ok(TokenState {
        present: true,
        username: Some(username),
    })
}

#[tauri::command]
pub fn clear_token(app: AppHandle, provider: String) -> Result<TokenState, String> {
    let kind = parse_provider(&provider)?;
    secrets::clear(&app, kind).map_err(|e| fail("Token could not be removed", e))?;
    let state = app.state::<AppState>();
    state.tokens.write().remove(&kind);
    state.usernames.write().remove(&kind);
    state.wake();
    Ok(TokenState {
        present: false,
        username: None,
    })
}

// ----------------------------------------------------------------- snapshot --

#[tauri::command]
pub fn get_snapshot(state: State<'_, AppState>) -> Snapshot {
    state.snapshot.read().clone()
}

#[tauri::command]
pub async fn refresh_now(app: AppHandle) -> Result<Snapshot, String> {
    Ok(refresh::refresh_and_publish(&app).await)
}

// ---------------------------------------------------------------- actions --

#[tauri::command]
pub async fn retry_pipeline(
    app: AppHandle,
    project_id: String,
    pipeline_id: u64,
) -> Result<(), String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .retry_pipeline(&project_id, pipeline_id)
        .await
        .map_err(|e| fail("Pipeline could not be retried", e))
}

#[tauri::command]
pub async fn cancel_pipeline(
    app: AppHandle,
    project_id: String,
    pipeline_id: u64,
) -> Result<(), String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .cancel_pipeline(&project_id, pipeline_id)
        .await
        .map_err(|e| fail("Pipeline could not be canceled", e))
}

#[tauri::command]
pub async fn retry_job(app: AppHandle, project_id: String, job_id: u64) -> Result<(), String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .retry_job(&project_id, job_id)
        .await
        .map_err(|e| fail("Job could not be retried", e))
}

#[tauri::command]
pub async fn cancel_job(app: AppHandle, project_id: String, job_id: u64) -> Result<(), String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .cancel_job(&project_id, job_id)
        .await
        .map_err(|e| fail("Job could not be canceled", e))
}

#[tauri::command]
pub async fn play_job(app: AppHandle, project_id: String, job_id: u64) -> Result<(), String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .play_job(&project_id, job_id)
        .await
        .map_err(|e| fail("Job could not be started", e))
}

#[tauri::command]
pub async fn job_trace(
    app: AppHandle,
    project_id: String,
    pipeline_id: u64,
    job_id: u64,
) -> Result<String, String> {
    let client = client_for_project(&app, &project_id)?;
    client
        .job_trace(&project_id, pipeline_id, job_id)
        .await
        .map_err(|e| fail("Job log could not be fetched", e))
}

// ------------------------------------------------------------------ window --

#[tauri::command]
pub fn set_notch_size(app: AppHandle, width: u32, height: u32) -> Result<(), String> {
    let Some(window) = notch::window(&app) else {
        return Ok(());
    };
    let top_offset = app.state::<AppState>().config.read().top_offset;
    notch::place(&window, width.max(1), height.max(1), top_offset);

    // Windows/Linux only: macOS already calls show() directly in setup().
    // First size report: the frontend being able to call this IPC is proof
    // the webview has actually loaded. See `AppState::notch_revealed`.
    #[cfg(not(target_os = "macos"))]
    {
        let state = app.state::<AppState>();
        if !state
            .notch_revealed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let _ = window.show();
        }
    }
    Ok(())
}

/// The screen's real notch dimensions. The frontend sizes the pill from
/// this: the area behind the notch is a physical hole, nothing drawn there
/// is visible.
#[tauri::command]
pub fn notch_metrics(app: AppHandle) -> notch::NotchMetrics {
    let Some(window) = notch::window(&app) else {
        return notch::NotchMetrics::default();
    };
    notch::metrics(&window)
}

#[tauri::command]
pub fn set_notch_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    let Some(window) = notch::window(&app) else {
        return Ok(());
    };
    // Neither is a plain show()/hide(): the window's membership in the
    // special notch space has to be managed by hand, otherwise it either
    // drops below the menu bar when it comes back, or stops receiving mouse
    // events.
    let _ = window;
    if visible {
        notch::reveal(&app);
    } else {
        notch::conceal(&app);
    }
    Ok(())
}

/// MUST BE ASYNC. Sync commands run on Tauri's main thread, and creating a
/// window requires the message loop to keep spinning. When this was sync,
/// clicking "Settings" from the notch panel had the IPC call block the main
/// thread, the webview couldn't start, and the window opened pure white.
/// Called from the tray menu it looked fine, because there the IPC call
/// doesn't get in the way.
#[tauri::command]
pub async fn open_settings(app: AppHandle) -> Result<(), String> {
    notch::open_settings(&app).map_err(|e| fail("Settings window could not be opened", e))
}

/// Opens a link in the user's default browser.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    // http(s) only; don't let an arbitrary string from the webview launch
    // something else.
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only http/https addresses can be opened.".to_string());
    }
    opener::open_browser(&url).map_err(|e| fail("Link could not be opened", e))
}

/// Full path to the log file; shown on the settings screen.
#[tauri::command]
pub fn log_path(app: AppHandle) -> String {
    crate::log::path(&app)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(not found)".to_string())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}
