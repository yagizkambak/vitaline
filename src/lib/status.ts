import type { PipelineStatus } from "../types";

type Tone = "ok" | "bad" | "busy" | "idle" | "warn";

interface StatusMeta {
  tone: Tone;
  label: string;
  /** Single-character glyph; used in the pill and job rows. */
  glyph: string;
}

const META: Record<PipelineStatus, StatusMeta> = {
  success: { tone: "ok", label: "Success", glyph: "✓" },
  failed: { tone: "bad", label: "Failed", glyph: "✕" },
  running: { tone: "busy", label: "Running", glyph: "●" },
  pending: { tone: "warn", label: "Pending", glyph: "○" },
  preparing: { tone: "warn", label: "Preparing", glyph: "○" },
  created: { tone: "warn", label: "Created", glyph: "○" },
  waiting_for_resource: { tone: "warn", label: "Waiting for resource", glyph: "○" },
  scheduled: { tone: "warn", label: "Scheduled", glyph: "◷" },
  manual: { tone: "idle", label: "Manual", glyph: "▸" },
  canceling: { tone: "idle", label: "Canceling", glyph: "■" },
  canceled: { tone: "idle", label: "Canceled", glyph: "■" },
  skipped: { tone: "idle", label: "Skipped", glyph: "»" },
  none: { tone: "idle", label: "No pipeline", glyph: "—" },
  unknown: { tone: "idle", label: "Unknown", glyph: "?" },
};

const FALLBACK: StatusMeta = META.unknown;

export function statusMeta(status: PipelineStatus): StatusMeta {
  return META[status] ?? FALLBACK;
}

export function statusTone(status: PipelineStatus): Tone {
  return statusMeta(status).tone;
}

/** Running/pending states, i.e. not yet resolved. */
export function isActive(status: PipelineStatus): boolean {
  const tone = statusTone(status);
  return tone === "busy" || tone === "warn";
}

/** Whether the user can act on this status (retry/cancel). */
export function canRetry(status: PipelineStatus): boolean {
  return status === "failed" || status === "canceled" || status === "success";
}

export function canCancel(status: PipelineStatus): boolean {
  return isActive(status) || status === "canceling";
}

/** 4210 -> "1h 10m", 95 -> "1m 35s" */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return "—";
  const total = Math.round(seconds);
  if (total < 60) return `${total}s`;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h}h ${m}m`;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

/** Produces a short relative expression like "3m ago" from an ISO date. */
export function timeAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "—";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "—";
  const diff = Math.max(0, Math.round((now - then) / 1000));
  if (diff < 45) return "just now";
  if (diff < 3600) return `${Math.round(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.round(diff / 3600)}h ago`;
  return `${Math.round(diff / 86400)}d ago`;
}

export function shortSha(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 8) : "—";
}
