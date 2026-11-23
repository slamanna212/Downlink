// Reports the video rect in PHYSICAL pixels to Rust so libmpv's native surface can sit under it.
import type { VideoBounds } from '../api/types';

export function toPhysicalBounds(rect: { left: number; top: number; width: number; height: number }, scale: number): VideoBounds {
  return {
    x: Math.round(rect.left * scale),
    y: Math.round(rect.top * scale),
    w: Math.round(rect.width * scale),
    h: Math.round(rect.height * scale),
    scale,
  };
}

const same = (a: VideoBounds | null, b: VideoBounds) =>
  a !== null && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h && a.scale === b.scale;

export interface BoundsReporterDeps {
  measure: () => VideoBounds | null;
  send: (b: VideoBounds) => void;
  raf?: (cb: () => void) => number;
  caf?: (id: number) => void;
  setTimer?: (cb: () => void, ms: number) => number;
  clearTimer?: (id: number) => void;
}

/** Trailing re-measures after a window transition (fullscreen, maximise, DPI change). Free when nothing changed. */
export const SETTLE_DELAYS_MS = [60, 160, 400, 900];

let seqCounter = 0;
/** Increasing stamp, also across page reloads, so Rust can drop reports that arrive out of order. */
export function nextBoundsSeq(now = () => performance.timeOrigin + performance.now()): number {
  seqCounter = Math.max(seqCounter + 1, Math.round(now() * 1000));
  return seqCounter;
}

/** Coalesces any number of `schedule()` calls within a frame into one measure+send, skipping no-ops. */
export function createBoundsReporter({
  measure,
  send,
  raf = requestAnimationFrame,
  caf = cancelAnimationFrame,
  setTimer = (cb, ms) => window.setTimeout(cb, ms),
  clearTimer = (id) => window.clearTimeout(id),
}: BoundsReporterDeps) {
  let pending = 0;
  let timers: number[] = [];
  const clearTimers = () => {
    timers.forEach(clearTimer);
    timers = [];
  };
  let last: VideoBounds | null = null;
  const flush = () => {
    pending = 0;
    const b = measure();
    if (b && b.w > 0 && b.h > 0 && !same(last, b)) {
      last = b;
      send(b);
    }
  };
  return {
    schedule() {
      if (!pending) pending = raf(flush);
    },
    /** Measure now and again a few times as a window transition settles; only changes are sent. */
    settle() {
      this.schedule();
      clearTimers();
      timers = SETTLE_DELAYS_MS.map((ms) => setTimer(() => this.schedule(), ms));
    },
    dispose() {
      if (pending) caf(pending);
      pending = 0;
      clearTimers();
    },
    reset() {
      last = null;
    },
  };
}
