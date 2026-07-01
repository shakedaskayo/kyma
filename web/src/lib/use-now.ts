import { useEffect, useState } from "react";

/**
 * Ticks a `Date.now()` value on `intervalMs` while `active`, otherwise returns
 * a static snapshot. Shared by every component that renders a live-ticking
 * relative time or duration (run rows, run detail header, activity feed) so
 * each one isn't hand-rolling its own `setInterval`.
 */
export function useNow(active: boolean, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(t);
  }, [active, intervalMs]);
  return now;
}
