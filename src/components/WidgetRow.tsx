import { useMemo, useState } from "react";
import {
  cancelPipeline,
  errorText,
  openExternal,
  retryPipeline,
} from "../lib/api";
import {
  canCancel,
  canRetry,
  formatDuration,
  shortSha,
  statusMeta,
  statusTone,
  timeAgo,
} from "../lib/status";
import type { JobInfo, ProjectSnapshot, StageInfo } from "../types";
import { JobRow } from "./JobRow";
import { MergeRequestList } from "./MergeRequestList";
import { StageBar } from "./StageBar";
import { StatusDot } from "./StatusDot";
import { TraceModal } from "./TraceModal";

interface Props {
  entry: ProjectSnapshot;
  /** Which row is open is owned by `Widget`: only one at a time. */
  expanded: boolean;
  onToggle: () => void;
  onAction: () => void;
  /** Carries action errors to the banner at the top of the widget. */
  onError: (message: string) => void;
}

/**
 * The stage worth naming in a single collapsed line.
 *
 * A widget row has space for exactly one stage name, so it should be the one
 * that explains the row's status: what broke, or what's running now. Falling
 * back to the last stage means a finished pipeline reads as the stage it
 * finished on rather than the first one it started with.
 */
function focusStage(stages: StageInfo[]): StageInfo | null {
  if (stages.length === 0) return null;
  return (
    stages.find((s) => statusTone(s.status) === "bad") ??
    stages.find((s) => statusTone(s.status) === "busy") ??
    stages.find((s) => statusTone(s.status) === "warn") ??
    stages[stages.length - 1]
  );
}

/**
 * One watched project as a single line, expandable down to its jobs.
 *
 * This is the notch's `ProjectCard` compressed into a widget's width: the same
 * data, the same actions, and the same job/MR/log components underneath — only
 * the collapsed summary is new.
 */
export function WidgetRow({
  entry,
  expanded,
  onToggle,
  onAction,
  onError,
}: Props) {
  const [stageFilter, setStageFilter] = useState<string | null>(null);
  const [trace, setTrace] = useState<JobInfo | null>(null);
  const [showMrs, setShowMrs] = useState(false);
  const [busy, setBusy] = useState(false);

  const { project, pipeline, mergeRequests } = entry;
  const title = project.label || pipeline?.projectName || project.id;
  const mrCount = mergeRequests?.length ?? 0;
  const status = entry.error ? "unknown" : (pipeline?.status ?? "none");
  const meta = statusMeta(status);

  const jobs = useMemo(() => {
    if (!pipeline) return [];
    const stages = stageFilter
      ? pipeline.stages.filter((s) => s.name === stageFilter)
      : pipeline.stages;
    return stages.flatMap((s) => s.jobs);
  }, [pipeline, stageFilter]);

  const stage = pipeline ? focusStage(pipeline.stages) : null;
  // Nothing to open for a project that has no pipeline and no MRs.
  const expandable = Boolean(pipeline) || mrCount > 0;

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

  return (
    <div className={`wrow wrow--${meta.tone} ${busy ? "is-busy" : ""}`}>
      <button
        type="button"
        className="wrow__line"
        title={
          entry.error
            ? entry.error
            : `${title} — ${meta.label}${stage ? ` (${stage.name})` : ""}`
        }
        disabled={!expandable}
        onClick={onToggle}
      >
        <span className={`wrow__caret ${expanded ? "is-open" : ""}`}>
          {expandable ? (expanded ? "▾" : "▸") : ""}
        </span>
        <StatusDot status={status} size="sm" />
        <span className="wrow__name">{title}</span>

        {entry.error ? (
          <span className="wrow__error">{entry.error}</span>
        ) : pipeline ? (
          <>
            <span className="wrow__stage">{stage?.name ?? ""}</span>
            <span className="wrow__bar">
              <StageBar stages={pipeline.stages} compact />
            </span>
            <span className="wrow__when">
              {/* A finished pipeline is best described by when it ran, a live
                  one by how long it has been going. */}
              {statusTone(pipeline.status) === "busy"
                ? formatDuration(pipeline.duration)
                : timeAgo(pipeline.createdAt)}
            </span>
          </>
        ) : (
          <span className="wrow__stage wrow__stage--empty">
            {project.gitRef ? `no pipeline (${project.gitRef})` : "no pipeline"}
          </span>
        )}

        {mrCount > 0 && (
          <span
            className="wrow__mrs"
            title={`${mrCount} open merge request(s)`}
          >
            {"⇄"} {mrCount}
          </span>
        )}
      </button>

      {expanded && (
        <div className="wrow__detail">
          {pipeline && (
            <>
              <div className="wrow__meta">
                <button
                  type="button"
                  className="linkish wrow__pipeline"
                  title="Open the pipeline in the browser"
                  onClick={() => void openExternal(pipeline.webUrl)}
                >
                  #{pipeline.id} {"↗"}
                </button>
                <span className="wrow__branch">{pipeline.gitRef}</span>
                <span className="wrow__sha">{shortSha(pipeline.sha)}</span>
                <span className={`wrow__state t-${meta.tone}`}>
                  {meta.label}
                </span>
                <span>{formatDuration(pipeline.duration)}</span>
              </div>

              {pipeline.commitTitle && (
                <div className="wrow__commit">{pipeline.commitTitle}</div>
              )}

              <StageBar
                stages={pipeline.stages}
                active={stageFilter}
                onPick={(name) =>
                  setStageFilter((cur) => (cur === name ? null : name))
                }
              />

              <div className="wrow__tools">
                {canRetry(pipeline.status) && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void run("pipeline could not be retried", () =>
                        retryPipeline(project.id, pipeline.id),
                      )
                    }
                  >
                    Retry
                  </button>
                )}
                {canCancel(pipeline.status) && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void run("pipeline could not be canceled", () =>
                        cancelPipeline(project.id, pipeline.id),
                      )
                    }
                  >
                    Cancel
                  </button>
                )}
                {stageFilter && (
                  <button type="button" onClick={() => setStageFilter(null)}>
                    Clear filter ({stageFilter})
                  </button>
                )}
                {mrCount > 0 && (
                  <button
                    type="button"
                    className="has-badge"
                    onClick={() => setShowMrs((v) => !v)}
                  >
                    {showMrs ? (
                      "Hide MRs"
                    ) : (
                      <>
                        Merge request <b>{mrCount}</b>
                      </>
                    )}
                  </button>
                )}
              </div>

              <div className="wrow__jobs">
                {jobs.length === 0 ? (
                  <div className="wrow__empty">No jobs found.</div>
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

              {trace && (
                <TraceModal
                  projectId={project.id}
                  pipelineId={pipeline.id}
                  job={trace}
                  onClose={() => setTrace(null)}
                />
              )}
            </>
          )}

          {/* With no pipeline the MR list is all there is to show, so it opens
              straight away instead of behind a button. */}
          {(showMrs || !pipeline) && mrCount > 0 && (
            <div className="wrow__jobs">
              <MergeRequestList items={mergeRequests} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
