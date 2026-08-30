import { statusMeta } from "../lib/status";
import type { StageInfo } from "../types";

interface Props {
  stages: StageInfo[];
  onPick?: (stage: string) => void;
  active?: string | null;
}

/** Draws the pipeline's stages as colored segments, left to right. */
export function StageBar({ stages, onPick, active }: Props) {
  if (stages.length === 0) return null;

  return (
    <div className="stagebar">
      {stages.map((stage) => {
        const meta = statusMeta(stage.status);
        const failed = stage.jobs.filter((j) => j.status === "failed").length;
        return (
          <button
            key={stage.name}
            type="button"
            className={`stagebar__seg stagebar__seg--${meta.tone} ${
              active === stage.name ? "is-active" : ""
            }`}
            title={`${stage.name} — ${meta.label}${failed ? ` (${failed} failed job)` : ""}`}
            onClick={() => onPick?.(stage.name)}
          >
            <span className="stagebar__name">{stage.name}</span>
            <span className="stagebar__count">{stage.jobs.length}</span>
          </button>
        );
      })}
    </div>
  );
}
