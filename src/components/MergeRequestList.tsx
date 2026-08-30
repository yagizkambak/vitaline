import { openExternal } from "../lib/api";
import { timeAgo } from "../lib/status";
import type { MergeRequestInfo } from "../types";

interface Props {
  items: MergeRequestInfo[];
}

export function MergeRequestList({ items }: Props) {
  if (items.length === 0) {
    return <div className="card__sub">No open merge requests.</div>;
  }

  return (
    <div className="mrs">
      {items.map((mr) => (
        <button
          key={mr.iid}
          type="button"
          className="mr"
          title={`${mr.sourceBranch} → ${mr.targetBranch}`}
          onClick={() => void openExternal(mr.webUrl)}
        >
          <span className="mr__iid">!{mr.iid}</span>
          {mr.draft && <span className="mr__draft">Draft</span>}
          <span className="mr__title">{mr.title}</span>
          <span className="mr__branch">{mr.sourceBranch}</span>
          <span className="mr__meta">
            {mr.author ?? "—"} · {timeAgo(mr.createdAt)}
          </span>
        </button>
      ))}
    </div>
  );
}
