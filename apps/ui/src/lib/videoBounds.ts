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
}

/** Coalesces any number of `schedule()` calls within a frame into one measure+send, skipping no-ops. */
export function createBoundsReporter({ measure, send, raf = requestAnimationFrame, caf = cancelAnimationFrame }: BoundsReporterDeps) {
  let pending = 0;
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
    dispose() {
      if (pending) caf(pending);
      pending = 0;
    },
    reset() {
      last = null;
    },
  };
}
