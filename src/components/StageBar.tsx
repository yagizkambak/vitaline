import { statusMeta } from "../lib/status";
import type { StageInfo } from "../types";

interface Props {
  stages: StageInfo[];
  onPick?: (stage: string) => void;
  active?: string | null;
  /**
   * Segments only, no stage names or job counts — for the widget's collapsed
   * rows, where the bar has to read as a progress strip inside a ~52px slot.
   * The tooltip still carries everything the labels would have said.
   */
  compact?: boolean;
}

/** Draws the pipeline's stages as colored segments, left to right. */
export function StageBar({ stages, onPick, active, compact = false }: Props) {
  if (stages.length === 0) return null;

  /**
   * Without `onPick` the bar is a read-only graphic, and it renders as spans
   * rather than buttons.
   *
   * Not cosmetic: the widget's collapsed row is ITSELF a button (it expands
   * the project), and nested buttons are invalid HTML — the inner ones swallow
   * clicks meant for the row, and React warns about it.
   */
  const interactive = Boolean(onPick);

  return (
    <div className={`stagebar ${compact ? "stagebar--compact" : ""}`}>
      {stages.map((stage) => {
        const meta = statusMeta(stage.status);
        const failed = stage.jobs.filter((j) => j.status === "failed").length;
        const className = `stagebar__seg stagebar__seg--${meta.tone} ${
          active === stage.name ? "is-active" : ""
        }`;
        const title = `${stage.name} — ${meta.label}${
          failed ? ` (${failed} failed job)` : ""
        }`;
        const labels = compact ? null : (
          <>
            <span className="stagebar__name">{stage.name}</span>
            <span className="stagebar__count">{stage.jobs.length}</span>
          </>
        );

        return interactive ? (
          <button
            key={stage.name}
            type="button"
            className={className}
            title={title}
            onClick={() => onPick?.(stage.name)}
          >
            {labels}
          </button>
        ) : (
          <span key={stage.name} className={className} title={title}>
            {labels}
          </span>
        );
      })}
    </div>
  );
}
