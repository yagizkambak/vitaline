import { useState } from "react";
import {
  cancelJob,
  errorText,
  openExternal,
  playJob,
  retryJob,
} from "../lib/api";
import { formatDuration, statusMeta } from "../lib/status";
import type { JobInfo, ProviderKind } from "../types";
import { StatusDot } from "./StatusDot";

interface Props {
  projectId: string;
  provider: ProviderKind;
  job: JobInfo;
  onAction: () => void;
  /** Send the error to the persistent banner above the panel, not into the row. */
  onError: (message: string) => void;
  onShowTrace: (job: JobInfo) => void;
}

export function JobRow({
  projectId,
  provider,
  job,
  onAction,
  onError,
  onShowTrace,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [showChildren, setShowChildren] = useState(false);
  const meta = statusMeta(job.status);

  const run = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
      onAction();
    } catch (e) {
      onError(`${job.name}: ${label} — ${errorText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const isFinished = ["success", "failed", "canceled", "skipped"].includes(
    job.status,
  );
  const isRunning = [
    "running",
    "pending",
    "preparing",
    "created",
    "waiting_for_resource",
  ].includes(job.status);

  // Provider capabilities: don't show buttons the API doesn't support.
  const canPlay = provider === "gitlab";
  const canRetryJob = provider === "gitlab" || provider === "github";
  const canCancelJob = provider === "gitlab";

  const childJobs = job.downstream?.stages.flatMap((s) => s.jobs) ?? [];

  return (
    <>
      <div className={`job ${busy ? "is-busy" : ""}`}>
        <StatusDot status={job.status} size="sm" />
        <span
          className="job__name"
          title={`${job.stage} / ${job.name} — ${meta.label}`}
        >
          {job.name}
        </span>
        {job.allowFailure && <span className="job__tag">allow_failure</span>}
        {/* Bridge job: the downstream pipeline it triggered shows up here. */}
        {job.downstream && (
          <span
            className="job__downstream"
            title={`Downstream pipeline #${job.downstream.id}${
              job.downstream.gitRef ? ` (${job.downstream.gitRef})` : ""
            } — ${statusMeta(job.downstream.status).label}`}
            onClick={(e) => {
              e.stopPropagation();
              // If the downstream pipeline's jobs could be fetched, expand/collapse
              // them; if not (no permission, empty pipeline), open it in the browser.
              if (childJobs.length > 0) setShowChildren((v) => !v);
              else if (job.downstream?.webUrl)
                void openExternal(job.downstream.webUrl);
            }}
          >
            {showChildren ? "▾" : "↳"}
            <StatusDot status={job.downstream.status} size="sm" />
            <span className="job__downstream-ref">
              {job.downstream.gitRef ?? `#${job.downstream.id}`}
            </span>
            {childJobs.length > 0 && (
              <span className="job__downstream-count">{childJobs.length}</span>
            )}
          </span>
        )}
        <span className="job__dur">{formatDuration(job.duration)}</span>

        <span className="job__actions">
          {canPlay && job.status === "manual" && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                run("could not be started", () => playJob(projectId, job.id))
              }
            >
              Start
            </button>
          )}
          {canRetryJob && isFinished && job.status !== "skipped" && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                run("could not be retried", () => retryJob(projectId, job.id))
              }
            >
              Retry
            </button>
          )}
          {canCancelJob && isRunning && (
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                run("could not be canceled", () => cancelJob(projectId, job.id))
              }
            >
              Cancel
            </button>
          )}
          {isFinished && (
            <button type="button" onClick={() => onShowTrace(job)}>
              Log
            </button>
          )}
          {job.webUrl.length > 0 && (
            <button
              type="button"
              title="Open in browser"
              onClick={() => void openExternal(job.webUrl)}
            >
              {"↗"}
            </button>
          )}
        </span>
      </div>

      {/* The downstream pipeline's jobs — read-only (see DownstreamInfo in types.ts). */}
      {showChildren && job.downstream && (
        <div className="job__children">
          {job.downstream.stages.map((stage) => (
            <div key={stage.name} className="job__child-stage">
              <span className="job__child-stage-name">{stage.name}</span>
              {stage.jobs.map((child) => (
                <div key={child.id} className="job job--child">
                  <StatusDot status={child.status} size="sm" />
                  <span
                    className="job__name"
                    title={`${child.stage} / ${child.name} — ${statusMeta(child.status).label}`}
                  >
                    {child.name}
                  </span>
                  <span className="job__dur">
                    {formatDuration(child.duration)}
                  </span>
                  {child.webUrl.length > 0 && (
                    <span className="job__actions">
                      <button
                        type="button"
                        title="Open in browser"
                        onClick={() => void openExternal(child.webUrl)}
                      >
                        {"↗"}
                      </button>
                    </span>
                  )}
                </div>
              ))}
            </div>
          ))}
        </div>
      )}
    </>
  );
}
