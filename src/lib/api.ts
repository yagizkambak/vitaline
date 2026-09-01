import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  DisplayMode,
  NotchMetrics,
  ProviderKind,
  Snapshot,
  TickerItem,
  TokenState,
  TokenStates,
} from "../types";

/** Rust emits this event on every poll loop iteration in the background. */
export const SNAPSHOT_EVENT = "pipelines://updated";
/** When "Show/hide notch" is selected from the tray menu. */
export const TOGGLE_EVENT = "notch://toggle";
/** When Rust announces a new MR / pipeline change; becomes a scrolling ticker in the notch. */
export const TICKER_EVENT = "notch://ticker";
/**
 * Emitted by Rust when the cursor enters/leaves the notch.
 *
 * We can't rely on the DOM's `onMouseEnter`: WKWebView only sets up its mouse
 * tracking area "while the app is active"; while the user is in another app,
 * the webview sees no movement at all and hovering over the notch did nothing.
 */
export const HOVER_EVENT = "notch://hover";
/**
 * Emitted by Rust after the config changes, from wherever it changed — the
 * settings window, the tray's "Widget mode" item, either surface's own mode
 * button. Surfaces that render FROM the config (the widget's opacity, the
 * settings window's mode radio) follow this instead of only seeing the value
 * they were opened with.
 */
export const CONFIG_EVENT = "config://updated";

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");
export const refreshNow = () => invoke<Snapshot>("refresh_now");

export const getConfig = () => invoke<AppConfig>("get_config");
export const saveConfig = (config: AppConfig) =>
  invoke<AppConfig>("save_config", { config });

export const getTokenStates = () => invoke<TokenStates>("get_token_states");
export const setToken = (provider: ProviderKind, token: string) =>
  invoke<TokenState>("set_token", { provider, token });
export const clearToken = (provider: ProviderKind) =>
  invoke<TokenState>("clear_token", { provider });

export const retryPipeline = (projectId: string, pipelineId: number) =>
  invoke<void>("retry_pipeline", { projectId, pipelineId });
export const cancelPipeline = (projectId: string, pipelineId: number) =>
  invoke<void>("cancel_pipeline", { projectId, pipelineId });
export const retryJob = (projectId: string, jobId: number) =>
  invoke<void>("retry_job", { projectId, jobId });
export const cancelJob = (projectId: string, jobId: number) =>
  invoke<void>("cancel_job", { projectId, jobId });
export const playJob = (projectId: string, jobId: number) =>
  invoke<void>("play_job", { projectId, jobId });

/** The job's log tail (last ~200 lines). pipelineId is required for Azure. */
export const jobTrace = (
  projectId: string,
  pipelineId: number,
  jobId: number,
) => invoke<string>("job_trace", { projectId, pipelineId, jobId });

/**
 * The panel's visible rectangle inside the window, in CSS px relative to the
 * window's top-left. The window is intentionally larger than the panel (spring
 * slack + lagging shrink); Rust's cursor watcher needs the panel, not the frame.
 */
export interface HoverRect {
  left: number;
  width: number;
  height: number;
}

export const setNotchSize = (
  width: number,
  height: number,
  hover?: HoverRect,
) => invoke<void>("set_notch_size", { width, height, hover });
export const setNotchVisible = (visible: boolean) =>
  invoke<void>("set_notch_visible", { visible });

/** The screen's real notch dimensions; the pill is sized from these. */
export const notchMetrics = () => invoke<NotchMetrics>("notch_metrics");

/** Switches surfaces: hides the notch and opens the widget, or the reverse. */
export const setDisplayMode = (mode: DisplayMode) =>
  invoke<void>("set_display_mode", { mode });

/** Hides the widget (its own Hide button) or brings it back (the tray menu). */
export const setWidgetVisible = (visible: boolean) =>
  invoke<void>("set_widget_visible", { visible });

/** Hover state coming from Rust's cursor watcher. */
export const onHover = (fn: (inside: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>(HOVER_EVENT, (event) => fn(event.payload));

export const openSettings = () => invoke<void>("open_settings");

/** Opens a link in the default browser (on the Rust side). */
export const openExternal = (url: string) =>
  invoke<void>("open_external", { url });
export const quitApp = () => invoke<void>("quit_app");

/** Path to the log file; shown on the settings screen. */
export const logPath = () => invoke<string>("log_path");

export function onSnapshot(
  handler: (snapshot: Snapshot) => void,
): Promise<UnlistenFn> {
  return listen<Snapshot>(SNAPSHOT_EVENT, (event) => handler(event.payload));
}

// A payload-less event, but kept with the same signature as the others so
// it can be used with `useTauriEvent`.
export function onToggle(
  handler: (payload: unknown) => void,
): Promise<UnlistenFn> {
  return listen(TOGGLE_EVENT, (event) => handler(event.payload));
}

export function onTicker(
  handler: (item: TickerItem) => void,
): Promise<UnlistenFn> {
  return listen<TickerItem>(TICKER_EVENT, (event) => handler(event.payload));
}

export function onConfig(
  handler: (config: AppConfig) => void,
): Promise<UnlistenFn> {
  return listen<AppConfig>(CONFIG_EVENT, (event) => handler(event.payload));
}

/** Errors from Rust come back as strings; read them the same way everywhere. */
export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
