mod azure;
mod commands;
mod config;
mod github;
mod gitlab;
mod log;
mod model;
mod notch;
mod poller;
mod providers;
mod refresh;
mod secrets;
mod state;
mod tray;
mod widget;

use tauri::Manager;

use crate::model::DisplayMode;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // The single-instance plugin MUST come before the others.
        //
        // Without this the app could be launched more than once: two notches
        // would overlap at the same screen position, "Quit" on one would
        // leave the other running, and the app would look like it hadn't
        // quit at all.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::line("second instance blocked, bringing the existing surface to front");
            // Whichever surface is the active one -- launching the app a
            // second time in widget mode used to reveal the notch on top of
            // the widget, leaving both on screen.
            widget::apply_mode(app);
            // Copied out first, not matched on the guard -- see the note in
            // `tray::toggle_surface`.
            let mode = app.state::<AppState>().config.read().display_mode;
            let window = match mode {
                DisplayMode::Notch => notch::window(app),
                DisplayMode::Widget => widget::window(app),
            };
            if let Some(window) = window {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_token_states,
            commands::set_token,
            commands::clear_token,
            commands::get_snapshot,
            commands::refresh_now,
            commands::retry_pipeline,
            commands::cancel_pipeline,
            commands::retry_job,
            commands::cancel_job,
            commands::play_job,
            commands::job_trace,
            commands::set_notch_size,
            commands::set_notch_visible,
            commands::notch_metrics,
            commands::set_display_mode,
            commands::set_widget_visible,
            commands::open_settings,
            commands::open_external,
            commands::quit_app,
            commands::log_path,
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            log::init(&handle);
            log::line("app starting");

            let config = config::load(&handle);
            let mut tokens = std::collections::HashMap::new();
            for kind in [
                crate::model::ProviderKind::Gitlab,
                crate::model::ProviderKind::Github,
                crate::model::ProviderKind::Azure,
            ] {
                if let Some(token) = secrets::get(&handle, kind) {
                    tokens.insert(kind, token);
                }
            }
            let show_on_all_spaces = config.show_on_all_spaces;
            let top_offset = config.top_offset;
            let start_collapsed = config.start_collapsed;
            let display_mode = config.display_mode;

            app.manage(AppState::new(config, tokens));

            tray::build(&handle)?;
            tray::set_starting(&handle);

            if let Some(window) = notch::window(&handle) {
                notch::apply_behaviour(&window, show_on_all_spaces);
                notch::place(
                    &window,
                    notch::INITIAL_WIDTH,
                    notch::INITIAL_HEIGHT,
                    top_offset,
                    // The panel rect isn't known until the frontend renders;
                    // the window is the pill's exact size at this point anyway.
                    None,
                );
                // On macOS, calling show() directly here has always worked
                // fine, so it's left as-is. On Windows the SAME call runs
                // before the WebView2 controller has attached and leaves the
                // window permanently blank; there, first visibility is given
                // when `set_notch_size` is called, in commands.rs (see
                // AppState::notch_revealed).
                //
                // In widget mode the window is still built, placed and
                // configured -- just never shown. Keeping its webview loaded
                // is what makes switching modes instant, and it costs nothing
                // on screen.
                #[cfg(target_os = "macos")]
                if display_mode == DisplayMode::Notch {
                    let _ = window.show();
                }
            }

            // The widget is created on demand, so in notch mode the window
            // never exists at all. `apply_mode` is deliberately NOT used
            // here: it would reveal the notch through `notch::reveal`, and on
            // Windows that show() has to wait for the webview (see above).
            if display_mode == DisplayMode::Widget {
                if let Err(err) = widget::ensure(&handle) {
                    log::line(&format!("widget: startup failed: {err}"));
                }
            }

            // Hover is driven from here rather than the webview's own mouse
            // events; see `notch::watch_cursor` for why.
            #[cfg(target_os = "macos")]
            notch::watch_cursor(handle.clone());

            // If there's no project yet, take the user straight to settings.
            if !start_collapsed || handle.state::<AppState>().config.read().watched.is_empty() {
                if let Err(err) = notch::open_settings(&handle) {
                    log::line(&format!("open_settings error: {err}"));
                }
            }

            tauri::async_runtime::spawn(poller::run(handle));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tauri app failed to start")
        .run(|_app, event| {
            // Tray app: must stay running in the background even if every
            // window is closed. `code` is populated when `app.exit(0)` is
            // called; only then do we actually quit.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                log::line(&format!("ExitRequested code={code:?}"));
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
