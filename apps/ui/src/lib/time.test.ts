import { beforeEach, describe, expect, it } from 'vitest';
import {
  addDays,
  dayLabel,
  floorToTick,
  formatClock,
  formatDay,
  formatDuration,
  formatRange,
  isLiveNow,
  minutesLeft,
  progress,
  setDisplayZone,
  startOfDay,
} from './time';

// 2026-11-19T14:05:00Z
const T = Date.UTC(2026, 10, 19, 14, 5, 0) / 1000;

describe('time', () => {
  beforeEach(() => setDisplayZone('UTC'));

  it('formats clock in the display zone', () => {
    expect(formatClock(T)).toBe('14:05');
    setDisplayZone('America/New_York'); // EST, UTC-5 in November
    expect(formatClock(T)).toBe('09:05');
    setDisplayZone('Asia/Kolkata'); // UTC+5:30
    expect(formatClock(T)).toBe('19:35');
  });

  it('ignores invalid zones', () => {
    setDisplayZone('Not/AZone');
    expect(formatClock(T)).toBe('14:05');
  });

  it('formats day and range', () => {
    expect(formatDay(T)).toBe('Thu 19 Nov');
    expect(formatRange(T, T + 3300)).toBe('14:05 – 15:00');
  });

  it('crosses DST correctly', () => {
    setDisplayZone('Europe/London');
    const summer = Date.UTC(2026, 6, 1, 12, 0, 0) / 1000; // BST UTC+1
    expect(formatClock(summer)).toBe('13:00');
    expect(formatClock(T)).toBe('14:05'); // GMT
  });

  it('formats durations', () => {
    expect(formatDuration(5)).toBe('0:05');
    expect(formatDuration(309)).toBe('5:09');
    expect(formatDuration(3909)).toBe('1:05:09');
    expect(formatDuration(-4)).toBe('0:00');
  });

  it('computes progress, clamped', () => {
    expect(progress(100, 200, 150)).toBe(0.5);
    expect(progress(100, 200, 50)).toBe(0);
    expect(progress(100, 200, 500)).toBe(1);
    expect(progress(100, 100, 100)).toBe(0);
  });

  it('knows what is live and minutes left', () => {
    expect(isLiveNow(100, 200, 100)).toBe(true);
    expect(isLiveNow(100, 200, 200)).toBe(false);
    expect(minutesLeft(260, 100)).toBe(3);
    expect(minutesLeft(100, 200)).toBe(0);
  });

  it('floors to ticks and day starts', () => {
    expect(formatClock(floorToTick(T))).toBe('14:00');
    expect(formatClock(floorToTick(T + 1500))).toBe('14:30');
    expect(formatClock(startOfDay(T))).toBe('00:00');
  });

  it('adds calendar days and labels days relative to now', () => {
    expect(formatClock(addDays(startOfDay(T), 1))).toBe('00:00');
    expect(dayLabel(T, T)).toBe('Today');
    expect(dayLabel(T + 86400, T)).toBe('Tomorrow');
    expect(dayLabel(T - 86400, T)).toBe('Yesterday');
    expect(dayLabel(T + 3 * 86400, T)).toBe('Sun 22 Nov');
  });
});

describe('memoised formatting', () => {
  it('startOfDay and formatClock stay correct across a zone change', async () => {
    const { setDisplayZone, getDisplayZone, startOfDay, formatClock } = await import('./time');
    const prev = getDisplayZone();
    try {
      const e = 1_700_000_000; // 2023-11-14 22:13:20 UTC
      setDisplayZone('UTC');
      expect(formatClock(e)).toBe('22:13');
      expect(startOfDay(e)).toBe(1_699_920_000);
      expect(startOfDay(e + 60)).toBe(1_699_920_000);
      setDisplayZone('Asia/Tokyo');
      expect(formatClock(e)).toBe('07:13');
      expect(startOfDay(e)).toBe(1_699_974_000);
    } finally {
      setDisplayZone(prev);
    }
  });
});

import { bucketEpoch, epochAtRatio, formatBehindLive, programmeSegments, ratioOfEpoch, timelineWindow } from './time';

describe('channel timeline', () => {
  const w = timelineWindow(10_000, 3600);

  it('centres the window on the playhead', () => {
    expect(w).toEqual({ start: 8200, end: 11_800 });
    expect(ratioOfEpoch(w, 10_000)).toBe(0.5);
    expect(epochAtRatio(w, 0.25)).toBe(9100);
    expect(epochAtRatio(w, 2)).toBe(11_800);
  });

  it('splits only at programme boundaries', () => {
    expect(programmeSegments([], w)).toEqual([{ start: 0, end: 1, title: null }]);
    const segs = programmeSegments(
      [
        { startUtc: 9000, endUtc: 10_800, title: 'B' },
        { startUtc: 5000, endUtc: 9000, title: 'A' },
        { startUtc: 20_000, endUtc: 21_000, title: 'far' },
      ],
      w,
    );
    expect(segs.map((s) => s.title)).toEqual(['A', 'B', null]);
    expect(segs[0]).toMatchObject({ start: 0 });
    expect(segs[1]?.start).toBeCloseTo(800 / 3600);
    expect(segs[1]?.end).toBeCloseTo(2600 / 3600);
    expect(segs[2]?.end).toBe(1);
  });

  it('formats time behind live and buckets epochs', () => {
    expect(formatBehindLive(2)).toBe('');
    expect(formatBehindLive(320)).toBe('−5:20');
    expect(bucketEpoch(1234)).toBe(1200);
  });
});

describe('calendar dates', () => {
  it('formats server dates without shifting them', async () => {
    const { formatCalendarDate, setDisplayZone } = await import('./time');
    setDisplayZone('Pacific/Auckland');
    expect(formatCalendarDate('2024-03-02')).toBe('2 Mar 2024');
    expect(formatCalendarDate('2024')).toBeNull();
    expect(formatCalendarDate(null)).toBeNull();
  });
});
