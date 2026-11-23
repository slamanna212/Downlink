import { describe, expect, it, vi } from 'vitest';
import { SETTLE_DELAYS_MS, createBoundsReporter, nextBoundsSeq, toPhysicalBounds } from './videoBounds';

function harness() {
  let rect = { left: 68, top: 0, width: 1372, height: 900 };
  let scale = 1.5;
  const frames: Array<() => void> = [];
  const send = vi.fn();
  const r = createBoundsReporter({
    measure: () => toPhysicalBounds(rect, scale),
    send,
    raf: (cb) => frames.push(cb),
    caf: () => undefined,
  });
  return {
    r, send, frames,
    runFrame: () => frames.splice(0).forEach((f) => f()),
    set: (next: Partial<typeof rect>, s = scale) => { rect = { ...rect, ...next }; scale = s; },
  };
}

describe('toPhysicalBounds', () => {
  it('multiplies CSS pixels by the device pixel ratio and rounds', () => {
    expect(toPhysicalBounds({ left: 68, top: 0, width: 1372, height: 900 }, 1.5)).toEqual({ x: 102, y: 0, w: 2058, h: 1350, scale: 1.5 });
    expect(toPhysicalBounds({ left: 0.4, top: 0.4, width: 10.2, height: 10.2 }, 2)).toEqual({ x: 1, y: 1, w: 20, h: 20, scale: 2 });
  });
});

describe('createBoundsReporter', () => {
  it('sends once per frame no matter how many notifications arrive', () => {
    const h = harness();
    for (let i = 0; i < 20; i++) h.r.schedule();
    expect(h.frames).toHaveLength(1);
    expect(h.send).not.toHaveBeenCalled();
    h.runFrame();
    expect(h.send).toHaveBeenCalledTimes(1);
  });
  it('does not resend an unchanged rect', () => {
    const h = harness();
    h.r.schedule(); h.runFrame();
    h.r.schedule(); h.runFrame();
    expect(h.send).toHaveBeenCalledTimes(1);
  });
  it('resends when the rect or scale changes, and after reset', () => {
    const h = harness();
    h.r.schedule(); h.runFrame();
    h.set({ left: 224, width: 1216 });
    h.r.schedule(); h.runFrame();
    h.set({}, 2);
    h.r.schedule(); h.runFrame();
    expect(h.send).toHaveBeenCalledTimes(3);
    h.r.reset();
    h.r.schedule(); h.runFrame();
    expect(h.send).toHaveBeenCalledTimes(4);
  });
  it('skips empty rects', () => {
    const h = harness();
    h.set({ width: 0 });
    h.r.schedule(); h.runFrame();
    expect(h.send).not.toHaveBeenCalled();
  });
});

describe('settle and ordering', () => {
  it('re-measures on trailing timers and sends only what changed', () => {
    const timers: Array<{ cb: () => void; ms: number }> = [];
    const frames: Array<() => void> = [];
    let rect = { left: 0, top: 0, width: 800, height: 600 };
    const send = vi.fn();
    const r = createBoundsReporter({
      measure: () => toPhysicalBounds(rect, 1),
      send,
      raf: (cb) => frames.push(cb),
      caf: () => undefined,
      setTimer: (cb, ms) => timers.push({ cb, ms }),
      clearTimer: () => undefined,
    });
    r.settle();
    frames.splice(0).forEach((f) => f());
    expect(send).toHaveBeenCalledTimes(1);
    rect = { left: 0, top: 0, width: 1920, height: 1080 }; // final size landed after the first measure
    expect(timers.map((t) => t.ms)).toEqual(SETTLE_DELAYS_MS);
    timers[0]?.cb();
    frames.splice(0).forEach((f) => f());
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenLastCalledWith({ x: 0, y: 0, w: 1920, h: 1080, scale: 1 });
    timers[1]?.cb();
    frames.splice(0).forEach((f) => f());
    expect(send).toHaveBeenCalledTimes(2);
  });
  it('nextBoundsSeq strictly increases even with a stalled clock', () => {
    const a = nextBoundsSeq(() => 1000);
    const b = nextBoundsSeq(() => 1000);
    const c = nextBoundsSeq(() => 999);
    expect(b).toBeGreaterThan(a);
    expect(c).toBeGreaterThan(b);
  });
});
