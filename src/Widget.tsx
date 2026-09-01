import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StatusDot } from "./components/StatusDot";
import { WidgetRow } from "./components/WidgetRow";
import { useSnapshot } from "./hooks/useSnapshot";
import { useTauriEvent } from "./hooks/useTauriEvent";
import {
  getConfig,
  onConfig,
  onTicker,
  openExternal,
  openSettings,
  quitApp,
  setDisplayMode,
  setWidgetVisible,
} from "./lib/api";
import { timeAgo } from "./lib/status";
import type { AppConfig, TickerItem } from "./types";

/**
 * Used until the real config arrives — a frame or two. Matches
 * `default_widget_opacity` in model.rs; if that changes, change this too.
 */
const FALLBACK_OPACITY = 0.94;

/** How long an announcement stays in the footer strip; matches Notch.tsx. */
function tickerDuration(text: string): number {
  return Math.min(14000, 4500 + text.length * 90);
}

function tickerTone(text: string): string {
  if (text.startsWith("✗") || text.startsWith("✕")) return "t-bad";
  if (text.startsWith("✓")) return "t-ok";
  return "t-busy";
}

/**
 * The widget surface: the same snapshot the notch shows, in a window the user
 * parks wherever they like and leaves open.
 *
 * The window itself is owned by Rust (`widget.rs`) — position, size, layer and
 * visibility all live there, so this component never resizes anything. The two
 * exceptions are the drag handle and the resize grip: both have to be started
 * from inside the mouse-down gesture, which only the webview is in.
 */
export function Widget() {
  const { snapshot, error, refreshing, refresh } = useSnapshot();
  /** Which project row is open. Only one at a time — the window is small. */
  const [openId, setOpenId] = useState<string | null>(null);
  /**
   * Action errors live here rather than in the rows: a row that collapses (or
   * a project that drops out of the next snapshot) used to take its own error
   * message down with it before anyone had read it.
   */
  const [actionError, setActionError] = useState<string | null>(null);
  const [opacity, setOpacity] = useState(FALLBACK_OPACITY);
  const [ticker, setTicker] = useState<TickerItem | null>(null);

  useEffect(() => {
    let alive = true;
    getConfig()
      .then((config) => alive && setOpacity(config.widget.opacity))
      .catch(() => {
        // Keep the fallback; a widget with no background would be worse.
      });
    return () => {
      alive = false;
    };
  }, []);

  // The opacity slider in the settings window applies live.
  useTauriEvent(onConfig, (config: AppConfig) =>
    setOpacity(config.widget.opacity),
  );

  /**
   * Announcements (a new MR, a pipeline that broke or recovered) come from the
   * same event the notch's ticker uses — Rust broadcasts it to every window.
   *
   * Unlike the notch, the widget does NOT queue them: it shows the newest one
   * and lets older ones go. The notch has to queue because its pill is the
   * only place the news ever appears; here every announcement is about a row
   * that's already on screen, so the strip is a nudge, not the record.
   */
  useTauriEvent(onTicker, (item: TickerItem) => setTicker(item));

  useEffect(() => {
    if (!ticker) return;
    const timer = window.setTimeout(
      () => setTicker(null),
      tickerDuration(ticker.text),
    );
    return () => window.clearTimeout(timer);
  }, [ticker]);

  /**
   * The header is the title bar the window doesn't have. Buttons inside it are
   * excluded, or pressing Refresh would start a window drag instead.
   */
  const startDrag = useCallback((event: React.MouseEvent) => {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }, []);

  const startResize = useCallback((event: React.MouseEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    void getCurrentWindow().startResizeDragging("SouthEast");
  }, []);

  const overall = error ? "failed" : (snapshot?.overall ?? "none");
  const projects = snapshot?.projects ?? [];

  return (
    <div
      className="widget"
      style={{ "--widget-opacity": opacity } as CSSProperties}
    >
      <header className="widget__head" onMouseDown={startDrag}>
        {/* The grip is drawn in CSS, not a glyph — see `.widget__handle`. */}
        <span className="widget__handle" aria-hidden="true" />
        <StatusDot status={overall} size="sm" />
        <span className="widget__title">Vitaline</span>
        <span className="widget__when">
          {snapshot ? timeAgo(snapshot.fetchedAt) : "loading…"}
        </span>
        <span className="widget__tools">
          <button
            type="button"
            className="icon-btn"
            title="Refresh now"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {"⟳"}
          </button>
          <button
            type="button"
            className="icon-btn"
            title="Settings"
            onClick={() => void openSettings()}
          >
            {"⚙"}
          </button>
          <button
            type="button"
            className="icon-btn"
            title="Hide — bring it back from the tray menu"
            onClick={() => void setWidgetVisible(false)}
          >
            {"✕"}
          </button>
        </span>
      </header>

      {actionError && (
        <div className="banner banner--bad banner--action">
          <span>{actionError}</span>
          <button type="button" onClick={() => setActionError(null)}>
            OK
          </button>
        </div>
      )}

      {error && <div className="banner banner--bad">{error}</div>}

      {snapshot && !snapshot.configured && (
        <div className="banner">
          No token or project yet.{" "}
          <button
            type="button"
            className="linkish"
            onClick={() => void openSettings()}
          >
            Open settings
          </button>
        </div>
      )}

      <div className="widget__list">
        {projects.map((entry) => (
          <WidgetRow
            key={entry.project.id}
            entry={entry}
            expanded={openId === entry.project.id}
            onToggle={() =>
              setOpenId((cur) =>
                cur === entry.project.id ? null : entry.project.id,
              )
            }
            onAction={() => void refresh()}
            onError={setActionError}
          />
        ))}
        {snapshot && projects.length === 0 && (
          <div className="widget__empty">No watched projects.</div>
        )}
      </div>

      {ticker && (
        <button
          type="button"
          className={`widget__ticker ${tickerTone(ticker.text)}`}
          title={ticker.url ? "Click to open in browser" : ticker.text}
          onClick={() => {
            if (ticker.url) void openExternal(ticker.url);
            setTicker(null);
          }}
        >
          {ticker.text}
        </button>
      )}

      <footer className="widget__foot">
        <button
          type="button"
          className="linkish widget__mode"
          title="Show the notch at the top of the screen instead"
          onClick={() => void setDisplayMode("notch")}
        >
          Notch mode
        </button>
        {/* The tray icon is easy to miss, and in widget mode the notch panel's
            Quit button isn't reachable at all. */}
        <button
          type="button"
          className="linkish widget__quit"
          onClick={() => void quitApp()}
        >
          Quit
        </button>
        {/* Borderless windows have no resize edges of their own. */}
        <span
          className="widget__resize"
          title="Drag to resize"
          onMouseDown={startResize}
        />
      </footer>
    </div>
  );
}
