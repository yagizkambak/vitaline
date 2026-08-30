import { statusMeta } from "../lib/status";
import type { PipelineStatus } from "../types";

interface Props {
  status: PipelineStatus;
  size?: "sm" | "md";
  title?: string;
}

export function StatusDot({ status, size = "md", title }: Props) {
  const meta = statusMeta(status);
  return (
    <span
      className={`dot dot--${meta.tone} dot--${size}`}
      title={title ?? meta.label}
      aria-label={meta.label}
      role="img"
    />
  );
}
