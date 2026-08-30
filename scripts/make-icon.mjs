// Generates the app icon's source PNG (dependency-free, plain Node).
// Run `npx tauri icon` on the output to convert it to every platform size.
//
//   node scripts/make-icon.mjs
//
// Drawing: a dark rounded square with a notch cut into the top edge, and an
// "orbit comet" inside it -- a trail of shrinking, dimming dots sweeping
// around the center with a soft ping ring on the lead dot. Reads as a live,
// continuously-polling status watch. Color matches --busy in styles.css
// (the app's own "actively running" blue), picked over the status-green
// used everywhere else in the UI because a mark that sits in a menu bar all
// day reads calmer in blue than in alert-flavored green.

import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const OUT = fileURLToPath(new URL("../src-tauri/icons/app-icon.png", import.meta.url));

const BG = [17, 20, 26];
const NOTCH = [5, 6, 9];
const ACCENT = [88, 166, 255]; // matches --busy in src/styles.css
const ACCENT_DIM = [46, 78, 120];

/** Signed distance to a rounded rectangle (negative = inside). */
function roundedRect(x, y, cx, cy, halfW, halfH, r) {
  const dx = Math.abs(x - cx) - (halfW - r);
  const dy = Math.abs(y - cy) - (halfH - r);
  const outside = Math.hypot(Math.max(dx, 0), Math.max(dy, 0));
  return outside + Math.min(Math.max(dx, dy), 0) - r;
}

function circleDist(x, y, cx, cy, r) {
  return Math.hypot(x - cx, y - cy) - r;
}

/** Edge antialiasing: turns a signed distance into 0..1 coverage. */
function coverage(distance) {
  return Math.min(Math.max(0.5 - distance, 0), 1);
}

function mix(dst, src, alpha) {
  for (let i = 0; i < 3; i++) dst[i] = Math.round(dst[i] * (1 - alpha) + src[i] * alpha);
}

function render() {
  const px = Buffer.alloc(SIZE * SIZE * 4);
  const c = SIZE / 2;
  const bodyHalf = SIZE * 0.42;
  const bodyRadius = SIZE * 0.22;

  const notchHalfW = SIZE * 0.17;
  const notchHalfH = SIZE * 0.055;
  const notchCy = c - bodyHalf + notchHalfH * 0.4;
  const notchRadius = SIZE * 0.045;

  // Orbit comet: a ring of dots shrinking and dimming along ~290 degrees of
  // sweep, plus a faint ping ring around the lead dot.
  const orbitRx = SIZE * 0.165, orbitRy = SIZE * 0.155;
  const orbitCy = c + SIZE * 0.045;
  const dots = 9;
  const sweep = Math.PI * 1.6;
  const headAngle = -Math.PI * 0.5;
  const headX = c + Math.cos(headAngle) * orbitRx;
  const headY = orbitCy + Math.sin(headAngle) * orbitRy;

  for (let y = 0; y < SIZE; y++) {
    for (let x = 0; x < SIZE; x++) {
      const rgba = [0, 0, 0, 0];

      const body = coverage(roundedRect(x, y, c, c, bodyHalf, bodyHalf, bodyRadius));
      if (body > 0) {
        rgba[0] = BG[0];
        rgba[1] = BG[1];
        rgba[2] = BG[2];
        rgba[3] = Math.round(body * 255);
      }

      const notch = coverage(
        roundedRect(x, y, c, notchCy, notchHalfW, notchHalfH, notchRadius),
      );
      if (notch > 0 && rgba[3] > 0) mix(rgba, NOTCH, notch);

      let best = { d: Infinity, i: 0 };
      for (let i = 0; i < dots; i++) {
        const a = headAngle - i * (sweep / (dots - 1));
        const dx = c + Math.cos(a) * orbitRx;
        const dy = orbitCy + Math.sin(a) * orbitRy;
        const r = SIZE * (0.05 - i * 0.0037);
        const dd = circleDist(x, y, dx, dy, r);
        if (dd < best.d) best = { d: dd, i };
      }
      const dotCov = coverage(best.d);
      if (dotCov > 0 && rgba[3] > 0) {
        const t = best.i / (dots - 1);
        const col = [
          Math.round(ACCENT[0] * (1 - t) + ACCENT_DIM[0] * t),
          Math.round(ACCENT[1] * (1 - t) + ACCENT_DIM[1] * t),
          Math.round(ACCENT[2] * (1 - t) + ACCENT_DIM[2] * t),
        ];
        mix(rgba, col, dotCov * (1 - t * 0.5));
      }
      if (best.i === 0) {
        const ring = Math.abs(circleDist(x, y, headX, headY, SIZE * 0.082)) - SIZE * 0.011;
        const ringCov = coverage(ring);
        if (ringCov > 0 && rgba[3] > 0) mix(rgba, ACCENT, ringCov * 0.55);
      }

      const i = (y * SIZE + x) * 4;
      px[i] = rgba[0];
      px[i + 1] = rgba[1];
      px[i + 2] = rgba[2];
      px[i + 3] = rgba[3];
    }
  }
  return px;
}

// --- minimal PNG encoder -----------------------------------------------------

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let c = -1;
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}

function encodePng(pixels, size) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  // 10..12: compression / filter / interlace = 0

  // Each row is prefixed with a filter byte (0 = None).
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    const from = y * size * 4;
    raw[y * (size * 4 + 1)] = 0;
    pixels.copy(raw, y * (size * 4 + 1) + 1, from, from + size * 4);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, encodePng(render(), SIZE));
console.log(`wrote: ${OUT} (${SIZE}x${SIZE})`);
