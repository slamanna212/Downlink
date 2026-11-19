import { beforeEach, describe, expect, it } from 'vitest';
import {
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
});
