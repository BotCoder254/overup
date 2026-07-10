import { useEffect, useState } from 'react';

/**
 * A ticking "now" for live-updating durations (queue waits, run timers).
 * Pass `active: false` to freeze it once nothing on screen is live.
 */
export function useNow(active = true, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    const handle = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(handle);
  }, [active, intervalMs]);
  return now;
}
