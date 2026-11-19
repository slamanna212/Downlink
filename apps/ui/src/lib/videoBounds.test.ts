import { describe, expect, it, vi } from 'vitest';
import { createBoundsReporter, toPhysicalBounds } from './videoBounds';

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
