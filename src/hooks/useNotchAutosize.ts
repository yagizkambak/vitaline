import { useEffect } from "react";
import { setNotchSize, type HoverRect } from "../lib/api";

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
export function useNotchAutosize(
  width: number,
  height: number,
  hover: HoverRect,
) {
  const { left, width: hoverWidth, height: hoverHeight } = hover;
  useEffect(() => {
    if (width <= 0 || height <= 0) return;
    void setNotchSize(Math.round(width), Math.round(height), {
      left: Math.round(left),
      width: Math.round(hoverWidth),
      height: Math.round(hoverHeight),
    }).catch(() => {
      // The window may be closing; this gets retried on the next measurement.
    });
    // `hover` is rebuilt on every render, so we depend on its FIELDS -- an
    // object identity dep would fire an IPC call per render.
  }, [width, height, left, hoverWidth, hoverHeight]);
}
