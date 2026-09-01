//! The notch window's position and platform-specific behavior.
//!
//! The window is borderless and transparent; it sticks to the middle of the
//! top edge of the screen. The frontend measures the content's real height
//! and reports it here via `set_notch_size`, and we re-center it.

#[cfg(not(target_os = "macos"))]
use tauri::LogicalSize;
#[cfg(not(target_os = "macos"))]
use tauri::PhysicalPosition;
use tauri::{AppHandle, Manager, WebviewWindow};

pub const LABEL: &str = "notch";
pub const SETTINGS_LABEL: &str = "settings";

/// Pill dimensions at startup; must match COLLAPSED_WIDTH in the frontend.
pub const INITIAL_WIDTH: u32 = 268;
pub const INITIAL_HEIGHT: u32 = 34;

/// Event emitted to the frontend when the cursor enters/leaves the notch (bool).
/// Only emitted on macOS (see `watch_cursor`); on other platforms hover is
/// driven by the webview's own mouse events.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const HOVER_EVENT: &str = "notch://hover";

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// The parts of the config that decide WHERE the window goes, as opposed to
/// how big it is. Grouped so `place` doesn't grow another positional `i32`
/// every time a placement setting is added -- two adjacent bare integers at a
/// call site is exactly how they end up swapped.
///
/// `horizontal_offset` is only read off a screen without a physical notch, so
/// on macOS the field is dead in the same way `HoverRect`'s are on Windows;
/// the lint is gated for the same reason.
#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub top_offset: i32,
    pub horizontal_offset: i32,
}

impl Placement {
    pub fn of(config: &crate::model::AppConfig) -> Self {
        Self {
            top_offset: config.top_offset,
            horizontal_offset: config.horizontal_offset,
        }
    }
}

/// The notch's location on screen, relative to the primary screen's TOP-LEFT
/// corner (Quartz orientation, y increases downward) -- the same space as
/// `CGEventGetLocation`. `place_macos` updates this on every placement, the
/// cursor watcher reads it.
#[cfg(target_os = "macos")]
static HOT_ZONE: parking_lot::Mutex<Option<(f64, f64, f64, f64)>> = parking_lot::Mutex::new(None);

/// The panel's VISIBLE rectangle inside the window, in logical points
/// relative to the window's TOP-LEFT corner.
///
/// The window is deliberately bigger than the panel: it carries `SPRING_SLACK`
/// on every side so the opening spring's overshoot doesn't clip, and it stays
/// large for the whole closing animation (`useLaggingShrink`). None of that
/// excess is painted -- it's transparent. So the hover zone has to be derived
/// from the panel, not from the window frame; using the frame made the panel
/// open while the cursor was still tens of points BELOW the notch.
///
/// The panel's top edge is always the window's top edge (`.panel { top: 0 }`),
/// so there's no `top` field.
///
/// Only `place_macos` reads these fields; the hover zone they describe exists
/// for the cursor watcher, which is macOS-only (elsewhere the webview's own
/// mouse events drive hover). Everywhere else the value is still carried
/// around -- stored in `LAST_GEOMETRY`, handed back to `place` -- but never
/// looked into, which is dead code as far as rustc is concerned. Gated rather
/// than allowed outright so a field that stops being read on macOS too still
/// gets reported.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverRect {
    /// Distance from the window's left edge to the panel's left edge.
    pub left: f64,
    pub width: f64,
    pub height: f64,
}

/// The last geometry the FRONTEND reported: logical (width, height) plus the
/// panel rect inside it.
///
/// `reveal` and the settings path have to re-place the window without being
/// able to ask anyone what should currently be on screen. They used to fall
/// back to `INITIAL_*`, which CLOBBERED the real pill: after Hide -> Show the
/// window came back 268x34, so the ears were clipped off and the hover zone
/// was the startup placeholder. The notch sat there dead until something else
/// happened to resize it -- which is exactly what a second click on the tray
/// item did (the window is visible by then, so `toggle_notch` pins the panel
/// open instead of revealing it, and THAT re-reports the real size).
static LAST_GEOMETRY: parking_lot::Mutex<Option<(u32, u32, HoverRect)>> =
    parking_lot::Mutex::new(None);

/// Watches the cursor and emits `HOVER_EVENT` when it enters the notch.
///
/// WHY THIS IS NEEDED: the panel's `onMouseEnter` relies on WKWebView's
/// NSTrackingArea, and WebKit sets that up with `activeInActiveApp` -- meaning
/// while the user is in ANOTHER app (which is the whole point of the notch),
/// the webview sees no mouse movement at all, and hovering over the notch
/// does nothing. (BoringNotch works because SwiftUI's `.onHover` sets up its
/// tracking area with `activeAlways`; there's no way for us to change that in
/// WKWebView.)
///
/// The fix is to watch the cursor externally. `CGEventCreate`/`CGEventGetLocation`
/// are safe from any thread, need no extra permission, and work even when the
/// app isn't active. Since `place_macos` caches the window's frame, we don't
/// need to hit the main thread on every tick.
#[cfg(target_os = "macos")]
pub fn watch_cursor(app: AppHandle) {
    use std::ffi::c_void;
    use std::time::Duration;
    use tauri::Emitter;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    extern "C" {
        fn CGEventCreate(source: *mut c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CFRelease(cf: *mut c_void);
    }

    /// The cursor's position relative to the primary screen's top-left.
    fn cursor() -> Option<CGPoint> {
        // SAFETY: CGEventCreate(NULL) produces a standalone event; we read
        // the returned pointer and release it right away. Both are
        // thread-safe Quartz APIs.
        unsafe {
            let event = CGEventCreate(std::ptr::null_mut());
            if event.is_null() {
                return None;
            }
            let point = CGEventGetLocation(event);
            CFRelease(event);
            Some(point)
        }
    }

    tauri::async_runtime::spawn(async move {
        // ~16 Hz: smooth enough for hover, negligible CPU cost.
        let mut ticker = tokio::time::interval(Duration::from_millis(60));
        let mut was_inside = false;

        loop {
            ticker.tick().await;

            // In widget mode the notch is off screen. Its webview keeps
            // running (so switching back is instant), so a hover event here
            // would still open and resize a window nobody can see.
            let notch_mode = app
                .try_state::<crate::state::AppState>()
                .map(|state| state.config.read().display_mode == crate::model::DisplayMode::Notch)
                .unwrap_or(true);
            if !notch_mode {
                // Emit the leave we owe the frontend. Without it, a panel
                // that was open at the moment the user switched to widget
                // mode would still be open when they switch back -- there's
                // no DOM mouseleave to close it (see the note above).
                if was_inside {
                    was_inside = false;
                    let _ = app.emit(HOVER_EVENT, false);
                }
                continue;
            }

            let Some(zone) = *HOT_ZONE.lock() else {
                continue;
            };
            let Some(point) = cursor() else { continue };

            let (x, y, w, h) = zone;
            let inside = point.x >= x && point.x <= x + w && point.y >= y && point.y <= y + h;

            if inside != was_inside {
                was_inside = inside;
                let _ = app.emit(HOVER_EVENT, inside);
            }
        }
    });
}

/// Resizes the window to the given logical size and places it at the top
/// center of the screen.
///
/// The window is ALWAYS centered horizontally on screen; since the notch is
/// also centered on screen, the window's exact center equals the notch's
/// exact center. The panel's own alignment (asymmetric ears, expand
/// animation) is done in CSS relative to this fixed center -- so there's only
/// ever one thing moving during the animation.
///
/// `hover` is the panel's visible rectangle inside the window; it defines the
/// cursor watcher's target zone on macOS. `None` means "the whole window",
/// which is only right before the frontend has reported a real panel size.
// See `metrics` for why the `return` inside the macOS block stays.
#[allow(clippy::needless_return)]
pub fn place(
    window: &WebviewWindow,
    width: u32,
    height: u32,
    placement: Placement,
    hover: Option<HoverRect>,
) {
    // Only a report that carries a panel rect comes from the frontend, and
    // only those describe a real on-screen state worth replaying later.
    if let Some(hover) = hover {
        *LAST_GEOMETRY.lock() = Some((width, height, hover));
    }

    // On macOS, size and position are given together in a SINGLE `setFrame:`
    // call; see `place_macos`. A separate `set_size` call causes a race there.
    #[cfg(target_os = "macos")]
    {
        place_macos(
            window,
            width as f64,
            height as f64,
            placement.top_offset as f64,
            hover,
        );
        return;
    }

    #[cfg(not(target_os = "macos"))]
    let _ = hover;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.set_size(LogicalSize::new(width as f64, height as f64));

        // The notch always stays on the main screen; that's where the menu
        // bar / taskbar live.
        let monitor = window
            .primary_monitor()
            .ok()
            .flatten()
            .or_else(|| window.current_monitor().ok().flatten());

        let Some(monitor) = monitor else {
            crate::log::line("place: no monitor found, skipping placement");
            return;
        };

        let scale = monitor.scale_factor();
        // The WORK AREA, not the full monitor: a taskbar docked to the left
        // or right edge shifts where "the left edge of the screen" is, and a
        // left-aligned notch would otherwise start underneath it.
        let area = monitor.work_area();
        let origin = area.position;
        let size = area.size;

        let physical_width = (width as f64 * scale).round() as i32;
        // How much room is left over once the window is laid down. Negative
        // if the window is wider than the work area, which `clamp` below then
        // resolves in favor of the left edge.
        let free = size.width as i32 - physical_width;
        let offset = (placement.horizontal_offset as f64 * scale).round() as i32;

        // Centered, then nudged. The offset is measured from the center
        // rather than from an edge so that `0` means what the app has always
        // done; see `AppConfig::horizontal_offset`.
        //
        // Clamped into the work area afterwards, which is what makes an
        // extreme value usable rather than dangerous: the user doesn't have
        // to work out the exact pixel that puts the bar against the right
        // edge, they can ask for far more than the screen has and land
        // flush. It also keeps the bar on screen while the panel GROWS --
        // opening near an edge widens the window by 140px, and without this
        // the far side would slide off.
        let x = origin.x + (free / 2 + offset).clamp(0, free.max(0));
        let y = origin.y + (placement.top_offset as f64 * scale).round() as i32;

        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// Re-applies the last geometry the frontend reported.
///
/// For callers that need to re-place the window (revealing it, a changed top
/// offset) but have no business changing its SIZE. Falls back to the startup
/// pill only if the frontend has never reported -- i.e. only before its first
/// render, where that placeholder is in fact correct.
pub fn replace(window: &WebviewWindow, placement: Placement) {
    // Copy the value out into its OWN statement first. Matching on
    // `*LAST_GEOMETRY.lock()` directly keeps the guard alive across the whole
    // match -- arms included -- and `place` locks the same mutex, so a
    // parking_lot (non-reentrant) lock deadlocks the caller. It froze the app
    // on Hide -> Show.
    let last = *LAST_GEOMETRY.lock();
    match last {
        Some((width, height, hover)) => place(window, width, height, placement, Some(hover)),
        None => place(window, INITIAL_WIDTH, INITIAL_HEIGHT, placement, None),
    }
}

/// macOS: sizes and positions the window directly in Cocoa's (NSScreen/NSWindow,
/// point-based) coordinate system, in a SINGLE `setFrame:` call.
///
/// This solves three separate problems at once:
///
/// 1) tao's `set_position` (see `util::window_position`) mixes up
///    `CGDisplay::main().pixels_high()` (Quartz, PIXELS) with the logical
///    (point) y value we send it; on scaled ("more space") resolutions the
///    result drifts from the real screen height.
///
/// 2) A race condition appears if size and position are given in SEPARATE
///    calls. `set_size` and the positioning here each go through the main
///    thread queue separately; if the size is processed AFTER, AppKit's
///    `setContentSize:` keeps the top-LEFT corner fixed and pushes the window
///    up. As the panel closed from 214pt down to 38pt, the notch would jump
///    exactly 176pt (=214-38) upward, off the top of the screen: "opened,
///    expanded, then vanished but still running". Giving the frame in one
///    atomic call ends this.
///
/// 3) `-setFrameTopLeftPoint:` (the method tao uses, and the first one we
///    tried) squeezes the window under the menu bar. `setFrame:` also goes
///    through `constrainFrameRect:toScreen:`; `allow_frames_over_menu_bar`
///    neutralizes that too.
///
/// NOTE: since the window is borderless, the frame size equals the content size.
#[cfg(target_os = "macos")]
fn place_macos(
    window: &WebviewWindow,
    width: f64,
    height: f64,
    top_offset: f64,
    hover: Option<HoverRect>,
) {
    let main_thread_window = window.clone();
    let _ = window.run_on_main_thread(move || {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let Ok(ptr) = main_thread_window.ns_window() else {
            crate::log::line("place_macos: ns_window() failed");
            return;
        };
        if ptr.is_null() {
            crate::log::line("place_macos: ns_window() returned a null pointer");
            return;
        }

        // SAFETY: run_on_main_thread guarantees we're on the main thread.
        // `screen`, `frame`, and `setFrame:display:` exist on every NSWindow.
        unsafe {
            let ns_window = ptr as *mut AnyObject;

            // Index 0 of `NSScreen.screens` is always the screen physically
            // carrying the menu bar (the one listed first in System Settings
            // > Displays) -- that's the screen the notch can be on.
            // `NSScreen.mainScreen` is something DIFFERENT: "the screen
            // holding the key window"; since our window is never key, this
            // usually returned the wrong (external) monitor and moved the
            // notch there. `.screen` is also unreliable when `ns_window`
            // hasn't been placed on any screen yet; so we try screen 0
            // first, and only fall back to the window's own screen if that's
            // unavailable too.
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            let mut screen: *mut AnyObject = if !screens.is_null() {
                msg_send![screens, objectAtIndex: 0usize]
            } else {
                std::ptr::null_mut()
            };
            if screen.is_null() {
                screen = msg_send![ns_window, screen];
            }
            if screen.is_null() {
                crate::log::line("place_macos: no screen found");
                return;
            }

            let screen_frame: NSRect = msg_send![screen, frame];
            let x = screen_frame.origin.x + (screen_frame.size.width - width) / 2.0;
            // Bottom-left corner (Cocoa's origin): top edge = top of screen -
            // top_offset, bottom-left = top edge - height.
            let y = screen_frame.origin.y + screen_frame.size.height - top_offset - height;

            let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
            let _: () = msg_send![ns_window, setFrame: frame, display: true];

            // The cursor watcher's target zone (see `watch_cursor`). Converted
            // to Quartz orientation: primary screen's top-left is (0,0), y
            // increases downward.
            //
            // The zone tracks the PANEL, not the window: the window carries
            // transparent slack on every side (see `HoverRect`), and hovering
            // empty air used to open the panel.
            //
            // We extend it downward by HOVER_SKIRT. The notch itself is a
            // physical hole: the cursor can never enter it FROM ABOVE, it
            // always approaches from below, and it's invisible while it's
            // behind the notch. Without this skirt, targeting the notch is
            // nearly impossible. It's kept small on purpose -- every point
            // here is a point of screen where the panel opens without the
            // cursor having reached the notch.
            const HOVER_SKIRT: f64 = 8.0;
            let panel = hover.unwrap_or(HoverRect {
                left: 0.0,
                width,
                height,
            });
            let top_down_y = screen_frame.size.height - (y + height);
            *HOT_ZONE.lock() = Some((
                x + panel.left,
                top_down_y,
                panel.width,
                panel.height + HOVER_SKIRT,
            ));

            // Did it actually land where we wanted? If the constraint comes
            // back (e.g. an AppKit update), log it -- this should never fire.
            let applied: NSRect = msg_send![ns_window, frame];
            if (applied.origin.x - x).abs() > 0.5 || (applied.origin.y - y).abs() > 0.5 {
                crate::log::line(&format!(
                    "place_macos: constraint override detected, wanted=({x},{y}) got=({},{})",
                    applied.origin.x, applied.origin.y
                ));
            }
        }
    });
}

/// The screen's actual notch dimensions (logical points).
///
/// On notch-less screens `has_notch` is false and width/height fall back to
/// reasonable defaults derived from the menu bar.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotchMetrics {
    pub has_notch: bool,
    /// Width of the physical notch (a hole, not pixels).
    pub notch_width: f64,
    /// Height of the physical notch.
    pub notch_height: f64,
    /// Height of the menu bar strip.
    pub menu_bar_height: f64,
}

impl Default for NotchMetrics {
    fn default() -> Self {
        Self {
            has_notch: false,
            notch_width: 0.0,
            notch_height: 0.0,
            menu_bar_height: 24.0,
        }
    }
}

/// Reads the primary screen's notch dimensions. Always returns the default outside macOS.
// There's no `else` for `#[cfg]`, so a function that does one thing on macOS
// and another everywhere else is written as two blocks, and the first one ends
// in `return`. On macOS the second block is compiled out, which is why clippy
// sees a `return` as the last thing in the function and calls it needless --
// it is right about that, and it would still be right if the two blocks ever
// drifted apart. The `return` stays because it is what makes "the macOS path
// ends here" true no matter what gets added after these blocks later, and
// because reshaping macOS-only control flow is not something the Windows side
// of this project can compile, let alone test.
#[allow(clippy::needless_return)]
pub fn metrics(window: &WebviewWindow) -> NotchMetrics {
    #[cfg(target_os = "macos")]
    {
        return metrics_macos(window);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        NotchMetrics::default()
    }
}

#[cfg(target_os = "macos")]
fn metrics_macos(window: &WebviewWindow) -> NotchMetrics {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel();
    let main_thread_window = window.clone();
    let dispatched = window.run_on_main_thread(move || {
        let _ = main_thread_window;
        // SAFETY: run_on_main_thread guarantees we're on the main thread.
        let value = unsafe { read_metrics() };
        let _ = tx.send(value);
    });

    if dispatched.is_err() {
        return NotchMetrics::default();
    }
    // Don't lock up the UI if the main thread fails to process the message for some reason.
    rx.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
unsafe fn read_metrics() -> NotchMetrics {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::NSRect;

    let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
    if screens.is_null() {
        return NotchMetrics::default();
    }
    let screen: *mut AnyObject = msg_send![screens, objectAtIndex: 0usize];
    if screen.is_null() {
        return NotchMetrics::default();
    }

    let frame: NSRect = msg_send![screen, frame];
    let (safe_top, left_w, right_w, menu_bar_height) = notch_geometry(screen, frame);

    let notch_width = if left_w > 0.0 && right_w > 0.0 {
        frame.size.width - left_w - right_w
    } else {
        0.0
    };

    NotchMetrics {
        has_notch: safe_top > 0.0 && notch_width > 0.0,
        notch_width,
        notch_height: safe_top.max(0.0),
        menu_bar_height,
    }
}

/// Reads the screen's raw notch dimensions:
/// `(safeAreaInsets.top, left ear width, right ear width, menu bar height)`.
///
/// `safeAreaInsets.top` is the notch's height on notched Macs (0 on
/// notch-less ones); `auxiliaryTopLeftArea` / `auxiliaryTopRightArea` are the
/// "ear" regions of the menu bar to the LEFT and RIGHT of the notch. The
/// notch's actual width = screen width - left ear - right ear. BoringNotch
/// uses exactly this same approach (see sizing/matters.swift). Both APIs are
/// macOS 12.1+, so they're guarded with `responds_to`; we return 0 otherwise.
#[cfg(target_os = "macos")]
unsafe fn notch_geometry(
    screen: *mut objc2::runtime::AnyObject,
    screen_frame: objc2_foundation::NSRect,
) -> (f64, f64, f64, f64) {
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, sel};
    use objc2_foundation::NSRect;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSEdgeInsets {
        top: f64,
        left: f64,
        bottom: f64,
        right: f64,
    }

    // SAFETY: NSEdgeInsets is a plain C struct made of four CGFloats.
    unsafe impl Encode for NSEdgeInsets {
        const ENCODING: Encoding = Encoding::Struct(
            "NSEdgeInsets",
            &[
                Encoding::Double,
                Encoding::Double,
                Encoding::Double,
                Encoding::Double,
            ],
        );
    }

    let cls = (&*(screen as *const AnyObject)).class();

    let safe_top = if cls.responds_to(sel!(safeAreaInsets)) {
        let insets: NSEdgeInsets = msg_send![screen, safeAreaInsets];
        insets.top
    } else {
        0.0
    };

    let (left_w, right_w) = if cls.responds_to(sel!(auxiliaryTopLeftArea))
        && cls.responds_to(sel!(auxiliaryTopRightArea))
    {
        let left: NSRect = msg_send![screen, auxiliaryTopLeftArea];
        let right: NSRect = msg_send![screen, auxiliaryTopRightArea];
        (left.size.width, right.size.width)
    } else {
        (0.0, 0.0)
    };

    let visible: NSRect = msg_send![screen, visibleFrame];
    let menu_bar_height = (screen_frame.origin.y + screen_frame.size.height)
        - (visible.origin.y + visible.size.height);

    (safe_top, left_w, right_w, menu_bar_height)
}

/// BoringNotch's core trick: moving the window into a **dedicated CGS Space**.
///
/// No matter how high `NSWindow.level` is raised, AppKit still keeps the
/// window below the menu bar/notch. BoringNotch gets around this by opening a
/// hidden space with `CGSSpaceCreate` at absolute level `i32::MAX` and adding
/// the window to it (see boringNotch/private/CGSSpace.swift and
/// managers/NotchSpaceManager.swift). Since that space's level sits above the
/// layer the menu bar is drawn on, the window genuinely sits inside the notch
/// and can receive mouse events.
///
/// The symbols live in SkyLight.framework (also visible via CoreGraphics) and
/// are a private API, so they're resolved at runtime via `dlsym` rather than
/// at link time: if they disappear in some future OS version, the app won't
/// crash, this trick just gets disabled.
#[cfg(target_os = "macos")]
mod cgs {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::sync::OnceLock;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    }
    const RTLD_NOW: c_int = 2;
    const SKYLIGHT: &str =
        "/System/Library/PrivateFrameworks/SkyLight.framework/Versions/A/SkyLight";

    /// Exactly matches BoringNotch's Swift signatures (proven to work there):
    /// connection and level are pointer-sized, the space id is a u64.
    type DefaultConnection = unsafe extern "C" fn() -> u64;
    type SpaceCreate = unsafe extern "C" fn(u64, isize, *const c_void) -> u64;
    type SpaceSetAbsoluteLevel = unsafe extern "C" fn(u64, u64, isize);
    type ShowSpaces = unsafe extern "C" fn(u64, *mut AnyObject);
    type WindowsToSpaces = unsafe extern "C" fn(u64, *mut AnyObject, *mut AnyObject);

    unsafe fn lookup(name: &str) -> *mut c_void {
        let sym = CString::new(name).unwrap();
        // First check symbols already loaded in the process; otherwise load
        // SkyLight explicitly.
        let found = dlsym(std::ptr::null_mut(), sym.as_ptr());
        if !found.is_null() {
            return found;
        }
        let path = CString::new(SKYLIGHT).unwrap();
        let handle = dlopen(path.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return std::ptr::null_mut();
        }
        dlsym(handle, sym.as_ptr())
    }

    /// (connection, space id) -- created once per process.
    static SPACE: OnceLock<Option<(u64, u64)>> = OnceLock::new();

    unsafe fn space() -> Option<(u64, u64)> {
        *SPACE.get_or_init(|| {
            let conn_fn = lookup("_CGSDefaultConnection");
            let create_fn = lookup("CGSSpaceCreate");
            let level_fn = lookup("CGSSpaceSetAbsoluteLevel");
            let show_fn = lookup("CGSShowSpaces");
            if conn_fn.is_null() || create_fn.is_null() || level_fn.is_null() || show_fn.is_null() {
                crate::log::line("cgs: symbols not found, skipping the notch space trick");
                return None;
            }

            let conn = std::mem::transmute::<*mut c_void, DefaultConnection>(conn_fn)();
            // Flag must be 1; passing 0 makes Finder draw desktop icons in this space.
            let space = std::mem::transmute::<*mut c_void, SpaceCreate>(create_fn)(
                conn,
                1,
                std::ptr::null(),
            );
            if space == 0 {
                crate::log::line("cgs: CGSSpaceCreate returned 0");
                return None;
            }
            std::mem::transmute::<*mut c_void, SpaceSetAbsoluteLevel>(level_fn)(
                conn,
                space,
                i32::MAX as isize,
            );
            std::mem::transmute::<*mut c_void, ShowSpaces>(show_fn)(conn, number_array(space));

            crate::log::line(&format!(
                "cgs: space created id={space} (level={})",
                i32::MAX
            ));
            Some((conn, space))
        })
    }

    /// A single-element `NSArray<NSNumber>` -- the CGS functions expect
    /// CFArray, and NSArray is toll-free bridged.
    unsafe fn number_array(value: u64) -> *mut AnyObject {
        let number: *mut AnyObject = msg_send![class!(NSNumber), numberWithUnsignedLongLong: value];
        msg_send![class!(NSArray), arrayWithObject: number]
    }

    unsafe fn move_window(ns_window: *mut AnyObject, symbol: &str, verb: &str) {
        let Some((conn, space)) = space() else { return };
        let func = lookup(symbol);
        if func.is_null() {
            crate::log::line(&format!("cgs: {symbol} not found"));
            return;
        }
        let window_number: isize = msg_send![ns_window, windowNumber];
        std::mem::transmute::<*mut c_void, WindowsToSpaces>(func)(
            conn,
            number_array(window_number as u64),
            number_array(space),
        );
        crate::log::line(&format!("cgs: window {window_number} space {space} {verb}"));
    }

    /// Adds the window to the notch space.
    pub unsafe fn join(ns_window: *mut AnyObject) {
        move_window(ns_window, "CGSAddWindowsToSpaces", "joined");
    }

    /// Removes the window from the notch space.
    ///
    /// Must be called BEFORE hiding. `orderOut:` drops the window's space
    /// membership on its own accord; if that membership is left half-done,
    /// showing the window again leaves it visible in both the normal space
    /// and our space, and it stops receiving mouse events ("doesn't expand
    /// after show/hide"). BoringNotch explicitly removes it too, with
    /// `undelegateWindow`, for the same reason.
    pub unsafe fn leave(ns_window: *mut AnyObject) {
        move_window(ns_window, "CGSRemoveWindowsFromSpaces", "left");
    }
}

/// Hides the notch.
///
/// A plain `hide()` is NOT enough: `orderOut:` leaves the window's membership
/// in the dedicated notch space half-finished, so when it's shown again it's
/// visible but doesn't receive mouse events ("doesn't expand after
/// show/hide"). We explicitly leave the space before hiding.
// See `metrics` for why the `return` inside the macOS block stays.
#[allow(clippy::needless_return)]
pub fn conceal(app: &AppHandle) {
    let Some(window) = window(app) else {
        return;
    };

    #[cfg(target_os = "macos")]
    {
        let main_thread_window = window.clone();
        let _ = window.run_on_main_thread(move || {
            let Ok(ptr) = main_thread_window.ns_window() else {
                return;
            };
            if ptr.is_null() {
                return;
            }
            // SAFETY: run_on_main_thread guarantees we're on the main thread.
            unsafe { cgs::leave(ptr as *mut objc2::runtime::AnyObject) };
            let _ = main_thread_window.hide();
        });
        return;
    }

    #[cfg(not(target_os = "macos"))]
    let _ = window.hide();
}

/// Makes the notch visible.
///
/// A plain `show()` is NOT enough: while hidden, the window drops out of the
/// dedicated CGS space, and showing it again brought it back below the menu
/// bar, unstyled ("after clicking Hide, the notch never showed up again").
/// So after showing it, the level/space/position are re-applied.
pub fn reveal(app: &AppHandle) {
    let Some(window) = window(app) else {
        return;
    };
    let state = app.state::<crate::state::AppState>();
    let (show_on_all_spaces, placement) = {
        let config = state.config.read();
        (config.show_on_all_spaces, Placement::of(&config))
    };

    let _ = window.show();
    apply_behaviour(&window, show_on_all_spaces);

    // Restore the size it had before it was hidden. Hiding does NOT unmount
    // the webview, so the frontend has no reason to re-report anything on the
    // way back -- its own state didn't change. Placing the startup pill here
    // was therefore permanent, not "only the first frame".
    replace(&window, placement);
}

/// Window behaviors applied on every startup and settings change.
pub fn apply_behaviour(window: &WebviewWindow, show_on_all_spaces: bool) {
    let _ = window.set_always_on_top(true);
    let _ = window.set_visible_on_all_workspaces(show_on_all_spaces);
    let _ = window.set_skip_taskbar(true);

    #[cfg(target_os = "macos")]
    raise_above_menu_bar(window);
}

/// On macOS, `alwaysOnTop` still stays below the menu bar. To actually sit
/// inside the notch, we raise the window's level to NSStatusWindowLevel (25).
///
/// This is the app's only Objective-C touchpoint; if it fails to compile,
/// emptying this function's body is enough for the rest of the app to keep
/// working (the notch just stays below the menu bar).
///
/// IMPORTANT: AppKit calls can only be made from the main thread.
/// `apply_behaviour` is also called from async commands like `save_config` /
/// `set_token`, which run in Tauri's background thread pool; calling
/// `msg_send` directly from there brought the whole app down with "not
/// called from main thread" (SIGABRT). `run_on_main_thread` hands the work
/// off to the event loop instead.
#[cfg(target_os = "macos")]
fn raise_above_menu_bar(window: &WebviewWindow) {
    let main_thread_window = window.clone();
    let _ = window.run_on_main_thread(move || {
        let window = main_thread_window;
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        // Same as BoringNotch: NSMainMenuWindowLevel (24) + 3.
        const NOTCH_WINDOW_LEVEL: isize = 27;

        let Ok(ptr) = window.ns_window() else {
            crate::log::line("raise_above_menu_bar: ns_window() failed");
            return;
        };
        if ptr.is_null() {
            crate::log::line("raise_above_menu_bar: ns_window() returned a null pointer");
            return;
        }

        // SAFETY: run_on_main_thread guarantees we're on the main thread;
        // ns_window() returns a valid NSWindow pointer, and setLevel: exists
        // on every NSWindow.
        unsafe {
            let ns_window = ptr as *mut AnyObject;
            let _: () = msg_send![ns_window, setLevel: NOTCH_WINDOW_LEVEL];
            allow_frames_over_menu_bar(&*ns_window);
            cgs::join(ns_window);
        }
    });
}

/// On `setFrame:`/`setContentSize:` calls, AppKit pushes the window BELOW the
/// menu bar via `-constrainFrameRect:toScreen:` (exactly 39pt down, on our
/// setup). Neither the window level nor the CGS space changes this; the
/// constraint lives inside NSWindow itself. BoringNotch never runs into this
/// because it uses `NSPanel` + `.nonactivatingPanel`; since tao hands us a
/// plain `NSWindow`, we get to the same result by neutralizing the method.
///
/// We use `class_replaceMethod`: it replaces the class's method table in
/// place, without touching any object's `isa`. (isa-swizzling, tried earlier,
/// collided with the isa swap tao/wry already does for KVO and sent the app
/// into a crash loop.)
#[cfg(target_os = "macos")]
unsafe fn allow_frames_over_menu_bar(ns_window: &objc2::runtime::AnyObject) {
    use objc2::encode::Encode;
    use objc2::ffi;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::{AnyClass, AnyObject, MethodImplementation, Sel};
    use objc2::sel;
    use objc2_foundation::NSRect;
    use std::ffi::CString;
    use std::sync::Once;

    /// Returns the given rect unchanged: no constraint applied at all.
    extern "C-unwind" fn unconstrained(
        _this: &AnyObject,
        _cmd: Sel,
        frame: NSRect,
        _screen: *mut AnyObject,
    ) -> NSRect {
        frame
    }

    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        autoreleasepool(|_| {
            // The object's class can be `NSKVONotifying_TaoWindow` because of
            // KVO; the method has to be written onto the real base class (TaoWindow).
            let mut cls: &AnyClass = ns_window.class();
            while cls.name().to_string_lossy().starts_with("NSKVONotifying_") {
                match cls.superclass() {
                    Some(sup) => cls = sup,
                    None => break,
                }
            }

            let types = CString::new(format!(
                "{}{}{}{}{}",
                <NSRect as Encode>::ENCODING,
                <*mut AnyObject as Encode>::ENCODING,
                <Sel as Encode>::ENCODING,
                <NSRect as Encode>::ENCODING,
                <*mut AnyObject as Encode>::ENCODING,
            ))
            .unwrap();

            let imp = (unconstrained as extern "C-unwind" fn(_, _, _, _) -> _).__imp();

            // SAFETY: `cls` is a valid class obtained at runtime, and
            // `unconstrained`'s signature exactly matches
            // `-constrainFrameRect:toScreen:`.
            unsafe {
                ffi::class_replaceMethod(
                    cls as *const AnyClass as *mut AnyClass,
                    sel!(constrainFrameRect:toScreen:),
                    imp,
                    types.as_ptr(),
                );
            }

            crate::log::line(&format!(
                "allow_frames_over_menu_bar: constrainFrameRect neutralized on class {}",
                cls.name().to_string_lossy()
            ));
        });
    });
}

/// Opens the settings window; brings it to front if it's already open.
pub fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        // If the window once loaded blank (got stuck on about:blank), it
        // used to stay white forever: we used to unconditionally reuse it.
        // Now we destroy the broken window and rebuild it instead.
        let healthy = window
            .url()
            .map(|u| {
                let u = u.as_str();
                u.contains("settings.html")
            })
            .unwrap_or(false);

        if healthy {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(());
        }

        crate::log::line("settings window loaded blank, rebuilding it");
        let _ = window.destroy();
    }

    let settings = tauri::WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("Vitaline — Settings")
    .inner_size(780.0, 760.0)
    .min_inner_size(560.0, 420.0)
    .resizable(true)
    .build()?;

    // The horizontal slider moves the notch live, without saving anything
    // (`commands::preview_notch_offset`). If this window goes away without a
    // Save, the bar is left wherever the slider was last dragged and only a
    // restart would put it back -- so the saved position is restored here.
    //
    // Registered per BUILD, not per open: the reuse path above returns before
    // reaching this, and closing the settings window destroys it, so each
    // live window gets exactly one handler.
    let handle = app.clone();
    settings.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::Destroyed) {
            return;
        }
        // `try_state` and a guard that ends with the statement: this runs on
        // a window event, where a panic would take the app down and a lock
        // held across `replace` would be one more chance to deadlock.
        let Some(state) = handle.try_state::<crate::state::AppState>() else {
            return;
        };
        let placement = Placement::of(&state.config.read());
        if let Some(window) = window(&handle) {
            replace(&window, placement);
        }
    });

    Ok(())
}
