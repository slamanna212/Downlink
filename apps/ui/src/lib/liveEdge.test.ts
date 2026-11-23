import { describe, expect, it } from 'vitest';
import { goLiveTarget, isBehindLive, LIVE_HIDE_SECS, LIVE_SHOW_SECS, smoothStep } from './liveEdge';

describe('live edge', () => {
  it('counts a few seconds behind the tip as live', () => {
    expect(isBehindLive(0, false)).toBe(false);
    expect(isBehindLive(LIVE_SHOW_SECS, false)).toBe(false);
    expect(isBehindLive(LIVE_SHOW_SECS + 1, false)).toBe(true);
  });

  it('has hysteresis between show and hide thresholds', () => {
    const mid = (LIVE_SHOW_SECS + LIVE_HIDE_SECS) / 2;
    expect(isBehindLive(mid, false)).toBe(false);
    expect(isBehindLive(mid, true)).toBe(true);
    expect(isBehindLive(LIVE_HIDE_SECS, true)).toBe(false);
  });

  it('go live keeps a cushion behind the edge and respects the oldest moment', () => {
    expect(goLiveTarget(1000)).toBe(997);
    expect(goLiveTarget(1000, 500)).toBe(997);
    expect(goLiveTarget(1000, 999)).toBe(999);
  });

  it('smoothing advances at real time and is monotonic with noisy targets', () => {
    let v: number | null = 100;
    let t = 100;
    for (let i = 0; i < 200; i++) {
      t += 1 / 30;
      const noisy = t + (i % 2 ? 0.8 : -0.8);
      const next = smoothStep(v, noisy, 1 / 30, true);
      expect(next).toBeGreaterThanOrEqual(v ?? 0);
      v = next;
    }
    expect(Math.abs((v ?? 0) - t)).toBeLessThan(1);
  });

  it('snaps on seeks and while paused', () => {
    expect(smoothStep(100, 200, 0.03, true)).toBe(200);
    expect(smoothStep(100, 101, 0.03, false)).toBe(101);
    expect(smoothStep(null, 50, 0.03, true)).toBe(50);
  });
});
