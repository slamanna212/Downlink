import { describe, expect, it } from 'vitest';
import { finishSync, reduceSync, SYNC_START, type SyncView } from './syncProgress';

const ev = (phase: string, done: number, total: number) => ({ sourceId: 7, phase, done, total });
const run = (...events: ReturnType<typeof ev>[]): SyncView => events.reduce(reduceSync, SYNC_START);

describe('first sync progress', () => {
  it('treats the first event of a phase as its step marker, not as counts', () => {
    const v = run(ev('connect', 0, 4), ev('channels', 1, 4));
    expect(v.phase).toBe('channels');
    expect(v.channels).toBe(0);
    expect(v.overall).toBeCloseTo(0.05);
  });

  it('counts channel pages against the total', () => {
    const v = run(ev('connect', 0, 4), ev('channels', 1, 4), ev('channels', 500, 1000));
    expect(v.channels).toBe(500);
    expect(v.channelsTotal).toBe(1000);
    expect(v.overall).toBeCloseTo(0.05 + 0.35 * 0.5);
  });

  it('creeps toward the phase end when the total is unknown, without reaching it', () => {
    const v = run(ev('guide', 2, 4), ev('guide', 1_000_000, 0));
    expect(v.guideDone).toBe(1_000_000);
    expect(v.overall).toBeGreaterThan(0.89);
    expect(v.overall).toBeLessThanOrEqual(0.9);
  });

  it('never moves backwards, and ignores late events from an earlier phase', () => {
    const v = run(ev('channels', 1, 4), ev('channels', 900, 1000), ev('guide', 2, 4), ev('channels', 950, 1000), ev('guide', 10, 0));
    expect(v.phase).toBe('guide');
    expect(v.channels).toBe(900);
    expect(v.overall).toBeGreaterThanOrEqual(0.4);
  });

  it('ignores phases it does not know', () => {
    expect(run(ev('vacuum', 3, 4))).toEqual(SYNC_START);
  });

  it('finishes at 100 %', () => {
    expect(finishSync(run(ev('guide', 2, 4)))).toMatchObject({ phase: 'done', overall: 1 });
    expect(run(ev('done', 4, 4)).overall).toBe(1);
  });
});
