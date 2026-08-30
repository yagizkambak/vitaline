import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, getSnapshot, onSnapshot, refreshNow } from "../lib/api";
import type { Snapshot } from "../types";

interface SnapshotState {
  snapshot: Snapshot | null;
  error: string | null;
  refreshing: boolean;
  refresh: () => Promise<void>;
}

/**
 * Holds the current pipeline status. Fetches the initial value via a
 * command, then updates it from events coming from Rust's poll loop.
 */
export function useSnapshot(): SnapshotState {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    getSnapshot()
      .then((s) => alive.current && setSnapshot(s))
      .catch((e) => alive.current && setError(errorText(e)));

    const unlisten = onSnapshot((s) => {
      if (!alive.current) return;
      setSnapshot(s);
      setError(null);
    });

    return () => {
      alive.current = false;
      void unlisten.then((fn) => fn());
    };
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const s = await refreshNow();
      if (alive.current) {
        setSnapshot(s);
        setError(null);
      }
    } catch (e) {
      if (alive.current) setError(errorText(e));
    } finally {
      if (alive.current) setRefreshing(false);
    }
  }, []);

  return { snapshot, error, refreshing, refresh };
}
