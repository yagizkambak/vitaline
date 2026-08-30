import { useMemo, useState } from "react";
import { cancelPipeline, errorText, openExternal, retryPipeline } from "../lib/api";
import { canCancel, canRetry, formatDuration, shortSha, statusMeta, timeAgo } from "../lib/status";
import type { JobInfo, ProjectSnapshot } from "../types";
import { JobRow } from "./JobRow";
import { MergeRequestList } from "./MergeRequestList";
import { StageBar } from "./StageBar";
import { StatusDot } from "./StatusDot";
import { TraceModal } from "./TraceModal";

interface Props {
  entry: ProjectSnapshot;
  onAction: () => void;
  /** Carries action errors to the persistent banner above the panel. */
  onError: (message: string) => void;
}

type Panel = "none" | "jobs" | "mrs";

export function ProjectCard({ entry, onAction, onError }: Props) {
  const [open, setOpen] = useState<Panel>("none");
  const [stageFilter, setStageFilter] = useState<string | null>(null);
  const [trace, setTrace] = useState<JobInfo | null>(null);
  const [busy, setBusy] = useState(false);

  const { project, pipeline, mergeRequests } = entry;
  const title = project.label || pipeline?.projectName || project.id;
  const mrCount = mergeRequests?.length ?? 0;

  const jobs = useMemo(() => {
    if (!pipeline) return [];
    const stages = stageFilter
      ? pipeline.stages.filter((s) => s.name === stageFilter)
      : pipeline.stages;
    return stages.flatMap((s) => s.jobs);
  }, [pipeline, stageFilter]);

  if (entry.error) {
    return (
      <div className="card card--error">
        <div className="card__row">
          <StatusDot status="unknown" />
          <div className="card__main">
            <div className="card__title">{title}</div>
            <div className="card__sub card__sub--error">{entry.error}</div>
          </div>
        </div>
      </div>
    );
  }

  const toggle = (panel: Panel) => setOpen((cur) => (cur === panel ? "none" : panel));

  const run = async (label: string, fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
      onAction();
    } catch (e) {
      onError(`${title}: ${label} — ${errorText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const mrButton = mrCount > 0 && (
    <button type="button" className="has-badge" onClick={() => toggle("mrs")}>
      Merge request <b>{mrCount}</b>
    </button>
  );

  if (!pipeline) {
    return (
      <div className="card">
        <div className="card__row">
          <StatusDot status="none" />
          <div className="card__main">
            <div className="card__title">{title}</div>
            <div className="card__sub">
              {project.gitRef ? `No pipeline for ${project.gitRef}` : "No pipeline yet"}
            </div>
          </div>
        </div>
        {mrCount > 0 && <div className="card__tools">{mrButton}</div>}
        {open === "mrs" && (
          <div className="card__jobs">
            <MergeRequestList items={mergeRequests} />
          </div>
        )}
      </div>
    );
  }

  const meta = statusMeta(pipeline.status);
  const jobCount = pipeline.stages.reduce((n, s) => n + s.jobs.length, 0);

  return (
    <div className={`card card--${meta.tone} ${busy ? "is-busy" : ""}`}>
      <div className="card__row">
        <StatusDot status={pipeline.status} />
        <div className="card__main">
          <div className="card__title">
            <button
              type="button"
              className="linkish"
              onClick={() => void openExternal(pipeline.webUrl)}
            >
              {title}
            </button>
            <span className="card__branch">{pipeline.gitRef}</span>
            <span className="card__sha">{shortSha(pipeline.sha)}</span>
          </div>
          <div className="card__sub">{pipeline.commitTitle ?? `#${pipeline.id}`}</div>
        </div>
        <div className="card__meta">
          <span className={`card__state card__state--${meta.tone}`}>{meta.label}</span>
          <span>{formatDuration(pipeline.duration)}</span>
          <span>{timeAgo(pipeline.createdAt)}</span>
        </div>
      </div>

      <StageBar
        stages={pipeline.stages}
        active={stageFilter}
        onPick={(name) => {
          setStageFilter((cur) => (cur === name ? null : name));
          setOpen("jobs");
        }}
      />

      <div className="card__tools">
        <button type="button" onClick={() => toggle("jobs")}>
          {open === "jobs" ? "Hide jobs" : `Jobs (${jobCount})`}
        </button>
        {mrButton}
        {canRetry(pipeline.status) && (
          <button
            type="button"
            disabled={busy}
            onClick={() => run("pipeline could not be retried", () => retryPipeline(project.id, pipeline.id))}
          >
            Retry pipeline
          </button>
        )}
        {canCancel(pipeline.status) && (
          <button
            type="button"
            disabled={busy}
            onClick={() => run("pipeline could not be canceled", () => cancelPipeline(project.id, pipeline.id))}
          >
            Cancel
          </button>
        )}
        {stageFilter && (
          <button type="button" onClick={() => setStageFilter(null)}>
            Clear filter ({stageFilter})
          </button>
        )}
      </div>

      {open === "jobs" && (
        <div className="card__jobs">
          {jobs.length === 0 ? (
            <div className="card__sub">No jobs found.</div>
          ) : (
            jobs.map((job) => (
              <JobRow
                key={job.id}
                projectId={project.id}
                provider={project.provider}
                job={job}
                onAction={onAction}
                onError={onError}
                onShowTrace={setTrace}
              />
            ))
          )}
        </div>
      )}

      {open === "mrs" && (
        <div className="card__jobs">
          <MergeRequestList items={mergeRequests} />
        </div>
      )}

      {trace && (
        <TraceModal
          projectId={project.id}
          pipelineId={pipeline.id}
          job={trace}
          onClose={() => setTrace(null)}
        />
      )}
    </div>
  );
}
