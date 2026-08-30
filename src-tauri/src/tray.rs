//! Menu bar / system tray icon.
//!
//! The icon isn't loaded from a file; it's drawn at runtime based on the
//! status color. That way there's no need to ship a separate PNG per status,
//! and the color looks the same on both platforms.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

use crate::model::{tone_of, Snapshot, Tone};
use crate::notch;

pub const TRAY_ID: &str = "vitaline";
pub const TOGGLE_EVENT: &str = "notch://toggle";

const ICON_SIZE: usize = 32;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / hide notch", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&toggle, &refresh, &settings, &separator, &quit])?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(dot_icon(Tone::Idle))
        .tooltip("Vitaline")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_notch(app),
            "refresh" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    crate::refresh::refresh_and_publish(&app).await;
                });
            }
            "settings" => {
                let _ = notch::open_settings(app);
            }
            "quit" => {
                crate::log::line("tray Quit selected");
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Shows the notch if it's hidden; tells the frontend to "open then close" if it's visible.
fn toggle_notch(app: &AppHandle) {
    let Some(window) = notch::window(app) else {
        return;
    };
    match window.is_visible() {
        Ok(true) => {
            let _ = app.emit(TOGGLE_EVENT, ());
        }
        _ => {
            // Plain show() isn't enough: it drops out of the notch space while hidden.
            notch::reveal(app);
        }
    }
}

/// Refreshes the tray icon and tooltip text to match the current status.
pub fn update(app: &AppHandle, snapshot: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    let tone = tone_of(&snapshot.overall);
    let _ = tray.set_icon(Some(dot_icon(tone)));
    let _ = tray.set_tooltip(Some(tooltip(snapshot)));

    // On macOS we can write a short summary next to the icon; on other
    // platforms the tray title isn't shown.
    #[cfg(target_os = "macos")]
    {
        let failed = count(snapshot, Tone::Bad);
        let running = count(snapshot, Tone::Busy);
        let title = if failed > 0 {
            format!("{failed} ✗")
        } else if running > 0 {
            format!("{running} ●")
        } else {
            String::new()
        };
        let _ = tray.set_title(Some(title));
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn count(snapshot: &Snapshot, want: Tone) -> usize {
    snapshot
        .projects
        .iter()
        .filter(|p| {
            let status = if p.error.is_some() {
                "failed"
            } else {
                p.pipeline.as_ref().map_or("none", |pl| pl.status.as_str())
            };
            tone_of(status) == want
        })
        .count()
}

fn tooltip(snapshot: &Snapshot) -> String {
    if snapshot.projects.is_empty() {
        return "Vitaline — no watched projects".to_string();
    }

    let mut lines = vec![format!("Vitaline — {}", snapshot.overall)];
    for entry in snapshot.projects.iter().take(8) {
        let name = entry
            .project
            .label
            .clone()
            .or_else(|| entry.pipeline.as_ref().map(|p| p.project_name.clone()))
            .unwrap_or_else(|| entry.project.id.clone());
        let status = entry
            .error
            .as_deref()
            .map(|_| "error")
            .or(entry.pipeline.as_ref().map(|p| p.status.as_str()))
            .unwrap_or("no pipeline");
        lines.push(format!("{name}: {status}"));
    }
    lines.join("\n")
}

fn rgb(tone: Tone) -> [u8; 3] {
    match tone {
        Tone::Ok => [63, 185, 80],
        Tone::Bad => [248, 81, 73],
        Tone::Busy => [88, 166, 255],
        Tone::Warn => [210, 153, 34],
        Tone::Idle => [125, 133, 144],
    }
}

/// Produces a filled circle in the given color, with softened edges, plus a
/// thin "ping" ring around it echoing the app icon's orbit-comet head glyph.
/// The ring is the same status color at much lower opacity, so it never
/// competes with the dot for the color read -- the tray icon's entire job
/// is that color, at a glance.
fn dot_icon(tone: Tone) -> Image<'static> {
    let [r, g, b] = rgb(tone);
    let size = ICON_SIZE;
    let center = (size as f32 - 1.0) / 2.0;
    let radius = size as f32 * 0.40;
    let ring_radius = size as f32 * 0.46;
    let ring_half_width = size as f32 * 0.045;

    let mut buffer = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();

            // Simple edge smoothing with a 1px transition band.
            let dot_alpha = ((radius - distance).clamp(0.0, 1.0) * 255.0).round() as u8;
            let ring_dist = (distance - ring_radius).abs() - ring_half_width;
            let ring_alpha = ((0.5 - ring_dist).clamp(0.0, 1.0) * 110.0).round() as u8;
            let alpha = dot_alpha.max(ring_alpha);

            let i = (y * size + x) * 4;
            buffer[i] = r;
            buffer[i + 1] = g;
            buffer[i + 2] = b;
            buffer[i + 3] = alpha;
        }
    }

    Image::new_owned(buffer, size as u32, size as u32)
}

/// Writes a temporary tooltip to the tray while the app is starting.
pub fn set_starting(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some("Vitaline — connecting…"));
    }
}
