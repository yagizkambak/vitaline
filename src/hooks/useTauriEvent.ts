import { useEffect, useRef } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * Subscribes to a Tauri event and unsubscribes when the component unmounts.
 *
 * WHY A SEPARATE HELPER: `listen()` isn't synchronous, it returns a Promise.
 * `React.StrictMode` runs every effect twice in development (mount ->
 * unmount -> mount); since cleanup runs before the first `listen()` promise
 * resolves, a naive implementation lets the second subscription get set up
 * before the first one is torn down, and every event ends up handled TWICE.
 *
 * This caused a real bug: a single MR added two entries to the announcement
 * queue, and when the first one's timer expired, the second one was left
 * behind and the ticker under the notch never closed again. With `onToggle`
 * it looked like it toggled twice and did nothing.
 *
 * The `alive` flag both ignores events that arrive after cleanup and closes
 * the subscription right away if the promise resolves late.
 */
export function useTauriEvent<T>(
  subscribe: (handler: (payload: T) => void) => Promise<UnlistenFn>,
  handler: (payload: T) => void,
) {
  // The handler can be re-created on every render; don't resubscribe because of that.
  const latest = useRef(handler);
  latest.current = handler;

  useEffect(() => {
    let alive = true;
    let off: UnlistenFn | undefined;

    void subscribe((payload) => {
      if (alive) latest.current(payload);
    }).then((fn) => {
      if (alive) off = fn;
      else fn();
    });

    return () => {
      alive = false;
      off?.();
    };
  }, [subscribe]);
}
