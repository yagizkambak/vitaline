import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { ProjectCard } from "./components/ProjectCard";
import { StatusDot } from "./components/StatusDot";
import { useNotchAutosize } from "./hooks/useNotchAutosize";
import { useTauriEvent } from "./hooks/useTauriEvent";
import { useSnapshot } from "./hooks/useSnapshot";
import {
  notchMetrics,
  onHover,
  onTicker,
  onToggle,
  openExternal,
  openSettings,
  quitApp,
  setNotchVisible,
} from "./lib/api";
import { statusTone, timeAgo } from "./lib/status";
import type { NotchMetrics, Snapshot, TickerItem } from "./types";

/**
 * Size used on notch-less screens (external monitor, Windows, Linux).
 *
 * On a notched Mac the pill is physically embedded in the notch, so it still
 * reads as "anchored" even though it's small. There's no such reference
 * point on a notch-less screen; the same small size just got lost on the
 * desktop. So it's a bit bigger and more prominent here.
 */
const PLAIN_COLLAPSED_WIDTH = 560;
const PLAIN_PILL_HEIGHT = 40;
/**
 * Width of the "ears" the pill carries on either side of the notch on a
 * notched Mac. Content only appears here: the area behind the notch is a
 * physical hole.
 *
 * Asymmetric because what they carry is asymmetric too: only the status dot
 * on the left, counters on the right. Making them equal left the left side
 * empty and needlessly long.
 */
/**
 * Collapsed, the pill's content all sits to the LEFT of the notch -- the strip
 * to the right belongs to macOS's own menu bar extras, and anything drawn
 * there lands on top of them. See `.pill__group` in styles.css.
 *
 * Not quite 0: this sliver carries no content, it only keeps a few points of
 * hover target on the notch's right side so approaching from that direction
 * still opens the panel. Small enough to read as part of the notch's own edge
 * rather than as a tab hanging off it.
 */
const COLLAPSED_RIGHT_EAR = 6;
/**
 * MINIMUM width of the left ear. Its real width is MEASURED from the content
 * (see `groupWidth`), because that content is variable length: the status
 * dot, one tone group per status, and the merge-request badge. Pinned to a
 * constant it fit three single digits and clipped everything past that.
 */
const EAR_MIN = 78;
/** The left ear's own padding: 10 toward the notch + 14 outward (styles.css). */
const EAR_PADDING = 24;
const EXPANDED_WIDTH = 700;
/**
 * How far the opening spring overshoots its target. If the window were
 * exactly the target size, the overshoot frames would get clipped and the
 * spring's whole effect would be lost (see `--spring-open` in styles.css).
 */
const SPRING_SLACK = 26;
/** Delay before closing once the mouse leaves the panel. */
const CLOSE_DELAY_MS = 220;
/**
 * How long the window stays large while the panel shrinks.
 *
 * Must be LONGER than both the `--grow-close` (600ms) and `bodyOut` (380ms)
 * animations in styles.css: if the window shrinks first while the panel is
 * still shrinking, it clips. Not an arbitrary number -- that's the only
 * reason it's 660.
 */
const CLOSE_ANIM_MS = 660;
/** Duration of the failure flash; matches `pillFlash` in styles.css. */
const FLASH_MS = 1300;
/** Height of the announcement ticker that opens under the notch, and the panel width. */
const TICKER_HEIGHT = 26;
const TICKER_WIDTH = 420;
/** Max number of ticker announcements held in the queue. */
const TICKER_QUEUE_MAX = 8;

/** Color tone based on the marker at the start of the announcement. */
function tickerTone(text: string): string {
  if (text.startsWith("✗") || text.startsWith("✕")) return "t-bad";
  if (text.startsWith("✓")) return "t-ok";
  return "t-busy";
}

/** How long the scrolling text stays on screen, based on its length. */
function tickerDuration(text: string): number {
  return Math.min(14000, 4500 + text.length * 90);
}

/**
 * For the window size: apply GROWTH immediately, delay SHRINKING until the
 * animation finishes.
 *
 * The window can't be smaller than the panel, or the panel clips; but there's
 * no harm in it being bigger than the panel, since the excess is transparent.
 * So we grow immediately, and wait for the CSS animation to finish before shrinking.
 */
function useLaggingShrink(value: number, delay: number): number {
  const [held, setHeld] = useState(value);

  // Growth is applied during render: even one late frame clips the panel.
  if (value > held) setHeld(value);

  useEffect(() => {
    if (value >= held) return;
    const timer = window.setTimeout(() => setHeld(value), delay);
    return () => window.clearTimeout(timer);
  }, [value, held, delay]);

  return held;
}

interface Tally {
  ok: number;
  bad: number;
  busy: number;
  other: number;
  mrs: number;
  total: number;
}

function tally(snapshot: Snapshot | null): Tally {
  const t: Tally = { ok: 0, bad: 0, busy: 0, other: 0, mrs: 0, total: 0 };
  if (!snapshot) return t;
  for (const entry of snapshot.projects) {
    t.total += 1;
    t.mrs += entry.mergeRequests?.length ?? 0;
    const tone = entry.error
      ? "bad"
      : statusTone(entry.pipeline?.status ?? "none");
    if (tone === "ok") t.ok += 1;
    else if (tone === "bad") t.bad += 1;
    else if (tone === "busy" || tone === "warn") t.busy += 1;
    else t.other += 1;
  }
  return t;
}

export function Notch() {
  const { snapshot, error, refreshing, refresh } = useSnapshot();
  const [hover, setHover] = useState(false);
  const [pinned, setPinned] = useState(false);
  /**
   * Action errors live here, not in the card components. Cards unmount when
   * the panel closes, and local error state used to disappear along with
   * them -- the error vanished before anyone saw it.
   */
  const [actionError, setActionError] = useState<string | null>(null);
  /**
   * The panel stays mounted a bit longer for the closing animation. The
   * window only shrinks once the animation finishes, so the exit animation
   * doesn't clip.
   */
  const [closing, setClosing] = useState(false);
  /** The pill flashes red briefly when the overall status newly turns bad. */
  const [flash, setFlash] = useState(false);
  /**
   * Announcement queue; the front one is shown, the rest wait their turn.
   *
   * Deliberately STATE, not a ref. It used to be a ref, and advancing to the
   * next item was done with `setTicker((cur) => cur ?? queue.current.shift() ?? null)`
   * -- i.e. the updater had a `shift()` side effect INSIDE it.
   * React.StrictMode calls updaters twice specifically to catch this: the
   * first call removes the item from the queue (and its result gets
   * discarded), the second call sees an empty queue and returns null, which
   * React then uses. The result: announcements were silently swallowed.
   */
  const [tickerQueue, setTickerQueue] = useState<TickerItem[]>([]);
  const ticker = tickerQueue[0] ?? null;
  /** The screen's real notch dimensions; assumed notch-less if unmeasurable. */
  const [metrics, setMetrics] = useState<NotchMetrics | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  /**
   * The body's NATURAL height. We can't measure the panel's own height since
   * it's animated -- that would produce intermediate values throughout the
   * growth. The body, on the other hand, flows at its full height even when
   * the panel clips it, so it's the right thing to measure.
   */
  const bodyRef = useRef<HTMLDivElement>(null);
  const [bodyHeight, setBodyHeight] = useState(0);
  /**
   * The pill content's NATURAL width (dot + counters), which sets the left
   * ear's width.
   *
   * Measurable only because `.pill__inner` is `flex: 0 0 auto` (styles.css):
   * it keeps its full width and overflows the ear rather than being squeezed
   * into it, so what we read here is what it actually NEEDS, not what it
   * currently has. No feedback loop -- the content's width doesn't depend on
   * the ear's, so this settles after one pass.
   */
  const groupRef = useRef<HTMLSpanElement>(null);
  const [groupWidth, setGroupWidth] = useState(0);
  const closeTimer = useRef<number | null>(null);
  const prevTone = useRef<string>("idle");

  // Don't let the panel close on its own while there's an unread error.
  const expanded = hover || pinned || actionError !== null;
  // The window stays large while closing; it shrinks once the animation ends.
  const bodyVisible = expanded || closing;

  const notched = metrics?.hasNotch ?? false;
  const notchWidth = notched ? Math.round(metrics!.notchWidth) : 0;
  // The pill is the same height as the notch so its bottom edge lines up
  // with the notch's bottom edge and it reads as one piece.
  const pillHeight = notched
    ? Math.round(metrics!.notchHeight)
    : PLAIN_PILL_HEIGHT;
  /**
   * VISUAL dimensions depend on `expanded`, WINDOW dimensions on `bodyVisible`.
   *
   * Closing looked terrible when both depended on `bodyVisible`: the panel
   * didn't move at all during the closing delay, then started shrinking
   * exactly on the frame the window shrank, and clipped instantly. The
   * correct order is the opposite -- the panel starts shrinking IMMEDIATELY,
   * the window stays large until the animation finishes. The window's excess
   * is transparent, so it's invisible.
   */
  /** The announcement ticker is only shown while the panel is closed; open, there's already a body. */
  const showTicker = ticker !== null && !expanded;

  const openEar = (EXPANDED_WIDTH - notchWidth) / 2;
  const tickerEar = (TICKER_WIDTH - notchWidth) / 2;
  const ear = (collapsed: number) =>
    !notched ? 0 : expanded ? openEar : showTicker ? tickerEar : collapsed;
  // Notched and collapsed: the whole pill is left of the notch, nothing to
  // the right of it. See `.pill__group` in styles.css for why.
  const leftEar = ear(Math.max(EAR_MIN, groupWidth + EAR_PADDING));
  const rightEar = ear(COLLAPSED_RIGHT_EAR);

  /**
   * The window is always centered on screen (see notch::place); the panel is
   * aligned to the notch and asymmetric. For the panel to fit inside the
   * window, the window has to be wide enough for the wider ear on both sides.
   *
   * `SPRING_SLACK`: the opening spring overshoots its target a bit (see
   * `--spring-open` in styles.css). If the window were exactly the target
   * size, the overshoot moments would clip and the spring's whole effect
   * would be lost.
   */
  const targetWindowWidth = notched
    ? notchWidth + (Math.max(leftEar, rightEar) + SPRING_SLACK) * 2
    : expanded
      ? EXPANDED_WIDTH
      : showTicker
        // The ear system is disabled on the notch-less path (leftEar/rightEar
        // are always 0), so the ticker would also get squeezed down to
        // PLAIN_COLLAPSED_WIDTH and clip. We apply the same TICKER_WIDTH used
        // on the Mac path here too, without touching its own calc (tickerEar).
        ? TICKER_WIDTH
        : PLAIN_COLLAPSED_WIDTH;

  useEffect(() => {
    const el = groupRef.current;
    if (!el) return;
    const measure = () =>
      setGroupWidth(Math.ceil(el.getBoundingClientRect().width));
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    measure();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const el = bodyRef.current;
    if (!el) return;
    const measure = () =>
      setBodyHeight(Math.ceil(el.getBoundingClientRect().height));
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    measure();
    return () => observer.disconnect();
  }, [bodyVisible]);

  /** The panel's target height; CSS SPRINGS toward this value (styles.css). */
  const panelHeight =
    pillHeight + (expanded ? bodyHeight : showTicker ? TICKER_HEIGHT : 0);
  const targetWindowHeight = panelHeight + SPRING_SLACK;

  // Grow the window immediately, wait for the animation to finish before shrinking.
  const windowWidth = useLaggingShrink(targetWindowWidth, CLOSE_ANIM_MS);
  const windowHeight = useLaggingShrink(targetWindowHeight, CLOSE_ANIM_MS);

  /**
   * The panel's VISIBLE rectangle inside the window -- the hover target.
   *
   * The window is deliberately bigger than the panel (SPRING_SLACK on every
   * side, plus it stays large for the whole closing animation) and all of
   * that excess is transparent. Rust used to take the window frame as the
   * hover zone, so the panel opened while the cursor was still well below
   * the notch, over empty desktop. See `notch::HoverRect`.
   *
   * Notched: the panel is aligned to the NOTCH, not the window -- `left: 50%`
   * of the window, shifted left by `notchWidth / 2 + leftEar` (styles.css,
   * `.panel--notched`). Notch-less: the panel is `width: 100%` at `left: 0`.
   */
  const panelWidth = notched ? leftEar + notchWidth + rightEar : windowWidth;
  const panelLeft = notched ? windowWidth / 2 - (notchWidth / 2 + leftEar) : 0;

  useNotchAutosize(windowWidth, windowHeight, {
    left: panelLeft,
    width: panelWidth,
    height: panelHeight,
  });

  // Re-read the notch dimensions when the screen changes too (an external monitor got plugged in).
  useEffect(() => {
    let alive = true;
    const read = () => {
      void notchMetrics()
        .then((m) => alive && setMetrics(m))
        .catch(() => alive && setMetrics(null));
    };
    read();
    window.addEventListener("resize", read);
    return () => {
      alive = false;
      window.removeEventListener("resize", read);
    };
  }, []);

  useTauriEvent(onToggle, () => setPinned((v) => !v));

  // Queue announcements coming from Rust; the front one is shown.
  useTauriEvent(onTicker, (item: TickerItem) => {
    setTickerQueue((queue) => [...queue, item].slice(-TICKER_QUEUE_MAX));
  });

  // Move to the next announcement once the current one's time is up.
  useEffect(() => {
    if (!ticker) return;
    const timer = window.setTimeout(() => {
      setTickerQueue((queue) => queue.slice(1));
    }, tickerDuration(ticker.text));
    return () => window.clearTimeout(timer);
  }, [ticker]);

  useEffect(() => {
    return () => {
      if (closeTimer.current) window.clearTimeout(closeTimer.current);
    };
  }, []);

  /**
   * Kick off the exit animation on the open -> closed transition.
   *
   * Doing this in an EFFECT doesn't work: the effect runs after the render
   * has been committed, so there's a frame in between where both `expanded`
   * and `closing` are false. In that frame `bodyVisible` is false and the
   * window shrinks, then the effect sets `closing` and it grows back
   * immediately after -- a visible flicker (measured: on a single close the
   * window jumped 376 -> 752 -> 376).
   *
   * Setting state during render is React's recommended way to handle this:
   * React re-renders right away before committing the output, so no
   * committed frame ever sits in between.
   */
  const [prevExpanded, setPrevExpanded] = useState(expanded);
  if (prevExpanded !== expanded) {
    setPrevExpanded(expanded);
    setClosing(!expanded);
  }

  // Actually remove the body once the closing animation finishes.
  useEffect(() => {
    if (!closing) return;
    const timer = window.setTimeout(() => setClosing(false), CLOSE_ANIM_MS);
    return () => window.clearTimeout(timer);
  }, [closing]);

  const counts = useMemo(() => tally(snapshot), [snapshot]);
  const overall = error ? "failed" : (snapshot?.overall ?? "none");

  // Flash the pill when the status newly turns "bad"; don't repeat if it's already bad.
  useEffect(() => {
    const tone = statusTone(overall);
    const was = prevTone.current;
    prevTone.current = tone;
    if (tone === "bad" && was !== "bad") {
      setFlash(true);
      const timer = window.setTimeout(() => setFlash(false), FLASH_MS);
      return () => window.clearTimeout(timer);
    }
  }, [overall]);

  const enter = useCallback(() => {
    if (closeTimer.current) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    setHover(true);
  }, []);

  const leave = useCallback(() => {
    if (closeTimer.current) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(
      () => setHover(false),
      CLOSE_DELAY_MS,
    );
  }, []);

  /**
   * On macOS, hover is driven by Rust's cursor watcher.
   *
   * WKWebView only sets up its mouse tracking area "while the app is
   * active": while in another app (which is the whole point of the notch),
   * the panel saw no mouseenter at all and hovering over the notch did
   * nothing. Once a single event arrives from Rust, we drop DOM events
   * entirely: running both at once lets the DOM's delayed "leave" close the
   * panel while the cursor is still inside.
   */
  const nativeHover = useRef(false);
  useTauriEvent(onHover, (inside: boolean) => {
    nativeHover.current = true;
    if (inside) enter();
    else leave();
  });

  return (
    <div
      ref={panelRef}
      className={`panel ${expanded ? "panel--open" : "panel--pill"} ${
        showTicker ? "panel--ticker" : ""
      } ${notched ? "panel--notched" : ""}`}
      style={
        {
          // The panel aligns itself to the notch in CSS (styles.css); here
          // we only provide dimensions. On a notch-less screen the width is
          // given directly.
          "--notch-w": `${notchWidth}px`,
          "--pill-h": `${pillHeight}px`,
          // We ONLY provide a numeric value on a notched screen. `.pill`'s
          // grid-template-columns falls back to CSS's `var(--left-ear, 1fr)`
          // (see styles.css) when these are undefined, and the content
          // collapses to the center. Even a concrete value like "0px" here
          // stops that fallback from kicking in -- on notch-less
          // Windows/Linux the columns pinned to 0px and squeezed the status
          // dot and counters down to invisible width (the notch looked like
          // it shrank to a line).
          ...(notched && {
            "--left-ear": `${leftEar}px`,
            "--right-ear": `${rightEar}px`,
          }),
          "--panel-h": `${panelHeight}px`,
          "--open-w": `${EXPANDED_WIDTH}px`,
          "--ticker-h": `${TICKER_HEIGHT}px`,
        } as CSSProperties
      }
      onMouseEnter={() => !nativeHover.current && enter()}
      onMouseLeave={() => !nativeHover.current && leave()}
    >
      <div
        className={`pill ${flash ? "pill--flash" : ""}`}
        title={pinned ? "Click to release" : "Click to keep open"}
        onClick={() => setPinned((v) => !v)}
      >
        {/* Notched: the dot and the counters share the single ear left of
            the notch, inside `.pill__inner` (the measured box). Notch-less:
            both wrappers are `display: contents`, so these two become direct
            children of `.pill` again and spread to its edges as before. */}
        <span className="pill__group">
          <span ref={groupRef} className="pill__inner">
            <span className="pill__status">
              <StatusDot status={overall} />
              {pinned && (
                <span className="pill__pin" title="Pinned">
                  {"\u25c9"}
                </span>
              )}
            </span>

            <span className="pill__meta">
              <span className="pill__counts">
                {counts.total === 0 ? (
                  <span className="pill__empty">no projects</span>
                ) : (
                  <>
                    {/* key=value: the element remounts when the count changes and plays the pop animation */}
                    {counts.ok > 0 && (
                      <b key={`ok-${counts.ok}`} className="t-ok">
                        {counts.ok}
                      </b>
                    )}
                    {counts.busy > 0 && (
                      <b key={`busy-${counts.busy}`} className="t-busy">
                        {counts.busy}
                      </b>
                    )}
                    {counts.bad > 0 && (
                      <b key={`bad-${counts.bad}`} className="t-bad">
                        {counts.bad}
                      </b>
                    )}
                    {counts.other > 0 && (
                      <b key={`idle-${counts.other}`} className="t-idle">
                        {counts.other}
                      </b>
                    )}
                    {counts.mrs > 0 && (
                      <span
                        key={`mrs-${counts.mrs}`}
                        className="pill__mrs"
                        title={`${counts.mrs} open merge request(s)`}
                      >
                        {"\u21c4"} {counts.mrs}
                      </span>
                    )}
                  </>
                )}
              </span>
            </span>
          </span>
        </span>

        {/* The physical notch itself: no pixels here, left empty. */}
        <span className="pill__notch" aria-hidden="true" />
      </div>

      {/* Announcement ticker: a subtitle-like strip that opens BELOW the
          notch. The panel grows for this (see panelHeight). */}
      {showTicker && ticker && (
        <div
          className={`ticker ${tickerTone(ticker.text)}`}
          title={ticker.url ? "Click to open in browser" : ticker.text}
          onClick={(e) => {
            e.stopPropagation();
            if (ticker.url) void openExternal(ticker.url);
            // Jump to the next announcement right away when clicked.
            setTickerQueue((queue) => queue.slice(1));
          }}
        >
          <span
            className="ticker__text"
            style={{ animationDuration: `${tickerDuration(ticker.text)}ms` }}
          >
            {ticker.text}
          </span>
        </div>
      )}

      {bodyVisible && (
        <div ref={bodyRef} className={`body ${closing ? "body--out" : ""}`}>
          <div className="body__head">
            <span className="body__title">Vitaline</span>
            <span className="body__when">
              {snapshot
                ? `updated ${timeAgo(snapshot.fetchedAt)}`
                : "loading…"}
            </span>
            <span className="body__tools">
              <button
                type="button"
                disabled={refreshing}
                onClick={() => void refresh()}
              >
                {refreshing ? "Refreshing…" : "Refresh"}
              </button>
              <button type="button" onClick={() => setPinned((v) => !v)}>
                {pinned ? "Release" : "Pin"}
              </button>
              <button type="button" onClick={() => void openSettings()}>
                Settings
              </button>
              <button type="button" onClick={() => void setNotchVisible(false)}>
                Hide
              </button>
              {/* The tray icon was easy to miss and there was no other
                  visible way to quit the app. */}
              <button
                type="button"
                className="danger"
                onClick={() => void quitApp()}
              >
                Quit
              </button>
            </span>
          </div>

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
              First enter your GitLab URL, token, and the projects you want to watch.{" "}
              <button
                type="button"
                className="linkish"
                onClick={() => void openSettings()}
              >
                Open settings
              </button>
            </div>
          )}

          <div className="cards">
            {snapshot?.projects.map((entry) => (
              <ProjectCard
                key={entry.project.id}
                entry={entry}
                onAction={() => void refresh()}
                onError={setActionError}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
