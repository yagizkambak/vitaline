import { useEffect } from "react";
import { setNotchSize } from "../lib/api";

/**
 * Resizes the window to the given TARGET size.
 *
 * We used to measure the panel's real height with a ResizeObserver. Once the
 * panel's height itself started animating, that broke: every frame produced
 * a new measurement, every measurement meant an IPC call + `setFrame`, and
 * since the window trailed a frame or two behind the CSS, the panel's bottom
 * edge clipped throughout the growth.
 *
 * Now the caller computes the target size and it stays FIXED for the whole
 * animation: the window jumps to its final size once, and the visible growth
 * happens entirely in CSS. The window's excess is transparent, so it's invisible.
 */
export function useNotchAutosize(width: number, height: number) {
  useEffect(() => {
    if (width <= 0 || height <= 0) return;
    void setNotchSize(Math.round(width), Math.round(height)).catch(() => {
      // The window may be closing; this gets retried on the next measurement.
    });
  }, [width, height]);
}
