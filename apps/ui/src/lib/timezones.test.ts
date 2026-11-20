import dayjs from 'dayjs';
import utc from 'dayjs/plugin/utc';
import timezone from 'dayjs/plugin/timezone';
import { afterEach, describe, expect, it } from 'vitest';
import { ppsFor, tickMarks, timeToX, type Geometry } from '../features/guide/guideModel';
import { addDays, dayLabel, formatClock, formatDay, formatRange, setDisplayZone, startOfDay } from './time';

dayjs.extend(utc);
dayjs.extend(timezone);

/** UTC epoch for a wall-clock time in `zone` (test helper only). */
const local = (zone: string, wall: string): number => dayjs.tz(wall, zone).unix();
const utcEpoch = (iso: string): number => Date.parse(iso) / 1000;

afterEach(() => setDisplayZone('UTC'));

// Zones and the dates of their clock changes in 2026 (hours in a local day: 23 on spring, 25 on fall).
const ZONES: { zone: string; plain: string; changes: { day: string; hours: number }[]; offsetMin: number }[] = [
  { zone: 'Europe/London', plain: '2026-06-10', offsetMin: 60, changes: [{ day: '2026-03-29', hours: 23 }, { day: '2026-10-25', hours: 25 }] },
  { zone: 'America/New_York', plain: '2026-06-10', offsetMin: -240, changes: [{ day: '2026-03-08', hours: 23 }, { day: '2026-11-01', hours: 25 }] },
  { zone: 'Australia/Sydney', plain: '2026-07-10', offsetMin: 600, changes: [{ day: '2026-04-05', hours: 25 }, { day: '2026-10-04', hours: 23 }] },
  { zone: 'Asia/Kolkata', plain: '2026-01-15', offsetMin: 330, changes: [] },
  { zone: 'Pacific/Chatham', plain: '2026-07-10', offsetMin: 765, changes: [{ day: '2026-04-05', hours: 25 }, { day: '2026-09-27', hours: 23 }] },
];

describe.each(ZONES)('$zone', ({ zone, plain, changes, offsetMin }) => {
  it('renders the UTC instant in local wall-clock time', () => {
    setDisplayZone(zone);
    const t = utcEpoch(`${plain}T12:00:00Z`);
    const wall = dayjs.utc(t * 1000).add(offsetMin, 'minute');
    expect(formatClock(t)).toBe(wall.format('HH:mm'));
    expect(formatDay(t)).toBe(wall.format('ddd D MMM'));
  });

  it('labels Today / Yesterday / Tomorrow by local day, not UTC day', () => {
    setDisplayZone(zone);
    const now = local(zone, `${plain} 00:10`); // just after local midnight: UTC date is often the previous day
    expect(dayLabel(local(zone, `${plain} 23:50`), now)).toBe('Today');
    expect(dayLabel(local(zone, `${plain} 00:05`), now)).toBe('Today');
    expect(dayLabel(now - 3600, now)).toBe('Yesterday'); // 23:10 the previous local day
    expect(dayLabel(now + 86400, now)).toBe('Tomorrow');
  });

  it('shows a day separator exactly at each local midnight and 30-minute ticks on :00/:30 local', () => {
    setDisplayZone(zone);
    const s = startOfDay(local(zone, `${plain} 12:00`));
    const e = addDays(s, 1);
    expect(formatClock(s)).toBe('00:00');
    const ticks = tickMarks({ rangeStart: s, rangeEnd: e }, s, e);
    expect(ticks[0]?.isDay).toBe(true);
    expect(ticks.filter((k) => k.isDay && k.t < e)).toHaveLength(1);
    for (const k of ticks.filter((x) => !x.isDay)) expect(k.label).toMatch(/^\d\d:(00|30)$/);
    // 48 ticks in a 24 h day, plus the closing midnight tick
    expect(ticks.filter((k) => k.t < e)).toHaveLength(48);
  });

  it.each(changes)('keeps ticks and labels right across the clock change on $day ($hours h day)', ({ day, hours }) => {
    setDisplayZone(zone);
    const s = startOfDay(local(zone, `${day} 12:00`));
    const e = addDays(s, 1);
    expect((e - s) / 3600).toBe(hours);
    const ticks = tickMarks({ rangeStart: s, rangeEnd: e }, s, e);
    expect(ticks.filter((k) => k.t < e)).toHaveLength(hours * 2);
    // Absolute spacing stays 30 minutes; labels may jump or repeat across the change.
    for (let i = 1; i < ticks.length; i++) expect(ticks[i]!.t - ticks[i - 1]!.t).toBe(1800);
    expect(ticks.filter((k) => k.isDay && k.t < e)).toHaveLength(1);
    expect(formatClock(startOfDay(e + 3600))).toBe('00:00'); // the next local midnight is also exact
  });
});

describe('a programme spanning a clock change', () => {
  it('Europe/London spring forward: block width is real time, labels skip the missing hour', () => {
    setDisplayZone('Europe/London');
    // 00:30 GMT -> 03:00 BST on 2026-03-29 is 1.5 real hours (the clocks jump 01:00 -> 02:00).
    const start = utcEpoch('2026-03-29T00:30:00Z');
    const end = utcEpoch('2026-03-29T02:00:00Z');
    expect(formatRange(start, end)).toBe('00:30 – 03:00');
    const rangeStart = startOfDay(start);
    const g: Geometry = { rangeStart, rangeEnd: addDays(rangeStart, 1), pps: ppsFor(1180, 180, 3), colW: 180, headerH: 40, rowH: 64 };
    expect(timeToX(g, end) - timeToX(g, start)).toBeCloseTo(5400 * g.pps, 6);
    const labels = tickMarks(g, start, end).map((k) => k.label);
    expect(labels).toEqual(['00:30', '02:00', '02:30', '03:00']);
    // the block's right edge lines up with the 03:00 tick
    const t3 = tickMarks(g, start, end).find((k) => k.label === '03:00')!.t;
    expect(timeToX(g, t3)).toBeCloseTo(timeToX(g, end), 6);
  });

  it('America/New_York fall back: the repeated hour has two 01:00 ticks and the block is 3 real hours', () => {
    setDisplayZone('America/New_York');
    // 00:30 EDT -> 02:30 EST on 2026-11-01 is 3 real hours.
    const start = local('America/New_York', '2026-11-01 00:30');
    const end = utcEpoch('2026-11-01T07:30:00Z'); // 02:30 EST
    expect(end - start).toBe(3 * 3600);
    const g: Geometry = { rangeStart: startOfDay(start), rangeEnd: addDays(startOfDay(start), 1), pps: 0.1, colW: 180, headerH: 40, rowH: 64 };
    expect(timeToX(g, end) - timeToX(g, start)).toBeCloseTo(3 * 3600 * 0.1, 6);
    const labels = tickMarks(g, start, end).map((k) => k.label);
    expect(labels.filter((l) => l === '01:00')).toHaveLength(2);
    expect(labels.filter((l) => l === '01:30')).toHaveLength(2);
  });

  it('Pacific/Chatham (+12:45): positions use UTC seconds, ticks land on local :00/:30', () => {
    setDisplayZone('Pacific/Chatham');
    const start = local('Pacific/Chatham', '2026-07-10 19:00');
    const end = local('Pacific/Chatham', '2026-07-10 20:30');
    expect(formatClock(start)).toBe('19:00');
    expect((end - start) / 60).toBe(90);
    // 19:00 Chatham is 06:15 UTC: the UTC minute is :15 but the tick is on the local half hour.
    expect(dayjs.utc(start * 1000).minute()).toBe(15);
    const g: Geometry = { rangeStart: startOfDay(start), rangeEnd: addDays(startOfDay(start), 1), pps: 0.1, colW: 180, headerH: 40, rowH: 64 };
    const ticks = tickMarks(g, start, end);
    expect(ticks.map((k) => k.label)).toEqual(['19:00', '19:30', '20:00', '20:30']);
    expect(timeToX(g, ticks[0]!.t)).toBeCloseTo(timeToX(g, start), 6);
  });
});
