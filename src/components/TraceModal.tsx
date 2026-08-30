import { useEffect, useState } from "react";
import { errorText, jobTrace } from "../lib/api";
import type { JobInfo } from "../types";

interface Props {
  projectId: string;
  /** Azure's log endpoint needs the build id; the other providers ignore it. */
  pipelineId: number;
  job: JobInfo;
  onClose: () => void;
}

/** Shows the tail of the job log inside the notch. */
export function TraceModal({ projectId, pipelineId, job, onClose }: Props) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setText(null);
    setError(null);
    jobTrace(projectId, pipelineId, job.id)
      .then((t) => alive && setText(t.trimEnd() || "(empty log)"))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [projectId, pipelineId, job.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="trace">
      <div className="trace__head">
        <strong>{job.name}</strong>
        <span className="trace__stage">{job.stage}</span>
        <button type="button" className="trace__close" onClick={onClose}>
          Close
        </button>
      </div>
      <pre className="trace__body">
        {error ? `Log could not be fetched: ${error}` : (text ?? "Loading…")}
      </pre>
    </div>
  );
}
