import { useEffect, useState } from 'react';
import { nowEpoch } from './time';

/** Current UTC epoch seconds, refreshed every `everyMs` (progress bars, "x min left"). */
export function useNowEpoch(everyMs = 30_000): number {
  const [now, setNow] = useState(nowEpoch);
  useEffect(() => {
    const id = setInterval(() => setNow(nowEpoch()), everyMs);
    return () => clearInterval(id);
  }, [everyMs]);
  return now;
}
