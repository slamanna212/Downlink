import { useEffect } from 'react';
import { invoke } from '../api/ipc';

let reported = false;

/** First list rendered with data: the "interactive" point of PLAN §8.8 (start -> interactive < 800 ms cached). */
export function markInteractive(): void {
  if (reported) return;
  reported = true;
  performance.mark('dl:interactive');
  const ms = Math.round(performance.now());
  // Debug hook so the desktop perf harness can read it; never blocks or fails the UI.
  void invoke('report_ui_timing', { name: 'dl:interactive', ms }).catch(() => undefined);
}

export function useInteractive(ready: boolean): void {
  useEffect(() => {
    if (ready) markInteractive();
  }, [ready]);
}

/** Run `fn` when the browser is idle (falls back to a short timeout). Returns a cancel function. */
export function whenIdle(fn: () => void, timeout = 2000): () => void {
  if (typeof window.requestIdleCallback === 'function') {
    const id = window.requestIdleCallback(fn, { timeout });
    return () => window.cancelIdleCallback(id);
  }
  const id = window.setTimeout(fn, 300);
  return () => window.clearTimeout(id);
}
