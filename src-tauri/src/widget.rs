//! The widget window: the app's second surface.
//!
//! Where the notch is pinned to the top center of the screen and opens on
//! hover, the widget is a panel the user parks anywhere and leaves open. It
//! shows the same snapshot the notch does -- one row per watched project,
//! expandable down to jobs -- so nothing here fetches or computes anything;
//! this module only owns the WINDOW.
//!
//! Only one surface is on screen at a time (`AppConfig::display_mode`);
//! `apply_mode` is the single place that switches between them.
//!
//! Unlike the notch, there is no platform-specific code below: the widget is
//! an ordinary window, so `always_on_top` / `always_on_bottom` are enough and
//! the same code runs on macOS and Windows. The notch's Objective-C
//! machinery exists only because it has to sit ABOVE the menu bar, which the
//! widget never does.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::model::{DisplayMode, WidgetConfig, WidgetLayer};
use crate::state::AppState;
use crate::{config, notch};

pub const LABEL: &str = "widget";

/// Gap from the work area's edges used the FIRST time the widget is placed.
const FIRST_MARGIN: f64 = 18.0;

/// How much of the widget's top-left corner has to stay inside a monitor's
/// work area for a saved position to be considered usable. Guards against the
/// widget coming back on a monitor that has since been unplugged.
const KEEP_VISIBLE: f64 = 48.0;

/// Geometry changes are written to disk this long after the last one. Dragging
/// a window emits `Moved` on every frame; without a debounce, one drag across
/// the screen would rewrite config.json a hundred times.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(700);

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Bumped on every geometry change; a pending save only writes if its token is
/// still the newest one.
static SAVE_TOKEN: AtomicU64 = AtomicU64::new(0);

/// Creates the widget window if it doesn't exist yet, and returns it.
///
/// The window is built at its saved size and position rather than being moved
/// after the fact: a window that appears at the default spot and then jumps to
/// its real one is visible as a flash on every launch.
pub fn ensure(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = window(app) {
        return Ok(window);
    }

    let (widget, show_on_all_spaces) = {
        let state = app.state::<AppState>();
        let config = state.config.read();
        (config.widget, config.show_on_all_spaces)
    };

    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("widget.html".into()))
        .title("Vitaline — Widget")
        .inner_size(widget.width as f64, widget.height as f64)
        // The OS enforces this floor while the user drags the grip, so the
        // header tools and the rows never get squeezed past being readable.
        .min_inner_size(
            crate::model::WIDGET_MIN_WIDTH as f64,
            crate::model::WIDGET_MIN_HEIGHT as f64,
        )
        .resizable(true)
        // No title bar: the widget draws its own header, and that header is
        // also the drag handle (`startDragging` in Widget.tsx).
        .decorations(false)
        // For the rounded corners.
        .transparent(true)
        // The notch turns its shadow OFF and draws one in CSS, because its
        // window is deliberately larger than its panel and has room for it.
        // The widget's panel fills its window exactly, so a CSS shadow would
        // be clipped away at the frame -- the platform's own shadow is the one
        // that can actually be seen here.
        .shadow(true)
        .skip_taskbar(true)
        // Taking focus would pull this app forward over whatever the user is
        // working in. The widget is a display; it's never the reason someone
        // switched apps.
        .focused(false)
        // REQUIRED, not a nicety. WKWebView reports `acceptsFirstMouse:` from
        // this attribute, and it defaults to false -- so while the user is in
        // another app (which is the normal state for a widget), the first
        // click on it would only activate Vitaline and be swallowed. Every
        // row would need clicking twice.
        .accept_first_mouse(true);

    if let Some((x, y)) = placement(app, &widget) {
        builder = builder.position(x, y);
    }

    let window = builder.build()?;
    apply_layer(&window, widget.layer, show_on_all_spaces);
    watch(app, &window);
    crate::log::line(&format!(
        "widget: window created {}x{} layer={:?}",
        widget.width, widget.height, widget.layer
    ));
    Ok(window)
}

/// Shows or hides the widget. Showing it creates the window on first use.
pub fn set_visible(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    if !visible {
        if let Some(window) = window(app) {
            let _ = window.hide();
        }
        return Ok(());
    }

    let window = ensure(app)?;
    let _ = window.show();
    // Re-apply after showing: hiding a window can drop its level and
    // workspace flags, so they can't be set once at creation and forgotten
    // (the notch has to do the same thing -- see `notch::reveal`).
    let (layer, show_on_all_spaces) = {
        let state = app.state::<AppState>();
        let config = state.config.read();
        (config.widget.layer, config.show_on_all_spaces)
    };
    apply_layer(&window, layer, show_on_all_spaces);
    Ok(())
}

/// Puts the surface the config names on screen and takes the other one off.
///
/// Called when the settings are saved and when the mode is switched from the
/// tray or from either surface's own button -- NOT during startup, where the
/// notch has a platform-specific first-show path (see `lib.rs`). Calling it
/// repeatedly is safe: showing what's already shown does nothing.
pub fn apply_mode(app: &AppHandle) {
    let mode = app.state::<AppState>().config.read().display_mode;

    match mode {
        DisplayMode::Notch => {
            if let Some(window) = window(app) {
                let _ = window.hide();
            }
            notch::reveal(app);
        }
        DisplayMode::Widget => {
            notch::conceal(app);
            if let Err(err) = set_visible(app, true) {
                crate::log::line(&format!("widget: window could not be opened: {err}"));
            }
        }
    }
    crate::tray::sync_mode(mode);
    crate::log::line(&format!("display mode applied: {mode:?}"));
}

/// Applies the config's layer choice to an existing window.
pub fn apply_layer(window: &WebviewWindow, layer: WidgetLayer, show_on_all_spaces: bool) {
    // ORDER MATTERS. On macOS both calls set the same NSWindow level (tao maps
    // them to NSFloatingWindowLevel and BelowNormalWindowLevel), so whichever
    // runs last decides. Clearing the opposite flag first also means switching
    // layers back and forth can't leave the window stuck at the old level.
    match layer {
        WidgetLayer::Front => {
            let _ = window.set_always_on_bottom(false);
            let _ = window.set_always_on_top(true);
        }
        WidgetLayer::Desktop => {
            let _ = window.set_always_on_top(false);
            let _ = window.set_always_on_bottom(true);
        }
    }
    let _ = window.set_visible_on_all_workspaces(show_on_all_spaces);
    let _ = window.set_skip_taskbar(true);
}

/// Re-applies everything in the config that affects the window itself.
/// The frontend picks up the rest (opacity) from the `config://updated` event.
pub fn refresh_from_config(app: &AppHandle) {
    let Some(window) = window(app) else {
        return;
    };
    let (layer, show_on_all_spaces) = {
        let state = app.state::<AppState>();
        let config = state.config.read();
        (config.widget.layer, config.show_on_all_spaces)
    };
    apply_layer(&window, layer, show_on_all_spaces);
}

/// Where the widget should open: its saved spot if that's still on a screen,
/// otherwise a fresh one. `None` means "let the window manager decide", which
/// is only reachable if no monitor can be read at all.
fn placement(app: &AppHandle, widget: &WidgetConfig) -> Option<(f64, f64)> {
    if let (Some(x), Some(y)) = (widget.x, widget.y) {
        let (x, y) = (x as f64, y as f64);
        if on_some_monitor(app, x, y) {
            return Some((x, y));
        }
        crate::log::line(&format!(
            "widget: saved position ({x},{y}) is on no current monitor, placing it fresh"
        ));
    }
    first_position(app, widget)
}

/// Top-right of the primary monitor's work area -- below the menu bar, clear
/// of the notch, and out of the way of most windows' title bars.
fn first_position(app: &AppHandle, widget: &WidgetConfig) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let left = area.position.x as f64 / scale;
    let top = area.position.y as f64 / scale;
    let width = area.size.width as f64 / scale;

    let x = left + width - widget.width as f64 - FIRST_MARGIN;
    let y = top + FIRST_MARGIN;
    Some((x.max(left), y))
}

/// Is this logical point inside some monitor's work area?
///
/// The comparison is done per monitor, each in its own scale factor. On a
/// mixed-DPI setup that isn't exactly the global logical space, but this is a
/// coarse "did the screen this was saved on disappear" check, not placement
/// math -- a monitor's worth of slack either way changes nothing.
fn on_some_monitor(app: &AppHandle, x: f64, y: f64) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };

    monitors.iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let left = area.position.x as f64 / scale;
        let top = area.position.y as f64 / scale;
        let right = left + area.size.width as f64 / scale;
        let bottom = top + area.size.height as f64 / scale;

        // The point is the window's TOP-LEFT corner: requiring a margin of it
        // to be inside means a widget dragged mostly off the edge still comes
        // back where it was, but one saved on an unplugged monitor doesn't.
        (left..=right).contains(&(x + KEEP_VISIBLE)) && (top..=bottom).contains(&(y + KEEP_VISIBLE))
    })
}

/// Keeps the config in step with what the user does to the window, and keeps
/// the window from being destroyed behind their back.
fn watch(app: &AppHandle, window: &WebviewWindow) {
    let handle = app.clone();
    let watched = window.clone();

    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            // The payloads are physical, and `Moved` reports the OUTER
            // position while `Resized` reports the inner size. Rather than
            // convert two different payload types, both values are read back
            // off the window in one place: the window is borderless, so outer
            // and inner are the same rect anyway.
            let Ok(scale) = watched.scale_factor() else {
                return;
            };
            let Ok(position) = watched.outer_position() else {
                return;
            };
            let Ok(size) = watched.inner_size() else {
                return;
            };
            let position = position.to_logical::<f64>(scale);
            let size = size.to_logical::<f64>(scale);
            remember_geometry(
                &handle,
                position.x.round() as i32,
                position.y.round() as i32,
                size.width.round() as u32,
                size.height.round() as u32,
            );
        }
        // `..` is required, not optional: the variant is `#[non_exhaustive]`,
        // so a pattern that names only `api` doesn't compile (E0638).
        WindowEvent::CloseRequested { api, .. } => {
            // Cmd+W / Alt+F4 would DESTROY the window. The widget has no
            // title bar, so from the user's side that reads as "it's gone" --
            // and it would come back only after switching modes twice. Hide
            // it instead, like the notch's Hide button, so the tray menu can
            // bring it back.
            api.prevent_close();
            let _ = watched.hide();
            crate::log::line("widget: close requested, hidden instead");
        }
        _ => {}
    });
}

/// Records the widget's geometry in the live config and schedules a save.
fn remember_geometry(app: &AppHandle, x: i32, y: i32, width: u32, height: u32) {
    let changed = {
        // `try_state` rather than `state`: window events can in principle
        // arrive before `setup` has managed the state, and panicking inside an
        // event handler would take the app down.
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let mut config = state.config.write();
        let widget = &mut config.widget;
        let changed = widget.x != Some(x)
            || widget.y != Some(y)
            || widget.width != width
            || widget.height != height;
        if changed {
            widget.x = Some(x);
            widget.y = Some(y);
            widget.width = width;
            widget.height = height;
        }
        changed
    };

    if changed {
        schedule_save(app);
    }
}

fn schedule_save(app: &AppHandle) {
    let token = SAVE_TOKEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SAVE_DEBOUNCE).await;
        // A newer change came in while we were waiting; that one owns the write.
        if SAVE_TOKEN.load(Ordering::SeqCst) != token {
            return;
        }

        let Some(config) = app
            .try_state::<AppState>()
            .map(|state| state.config.read().clone())
        else {
            return;
        };
        if let Err(err) = config::save(&app, &config) {
            crate::log::line(&format!("widget: geometry could not be saved: {err}"));
        }
    });
}
