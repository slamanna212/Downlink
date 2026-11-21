// The ONLY module that does date math in the UI (CLAUDE rule 11). Store/transport UTC epoch seconds;
// render through here. dayjs with utc + timezone plugins.
import dayjs from 'dayjs';
import utc from 'dayjs/plugin/utc';
import timezone from 'dayjs/plugin/timezone';

dayjs.extend(utc);
dayjs.extend(timezone);

let zone: string = safeLocalZone();

function safeLocalZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

export function isValidZone(tz: string): boolean {
  try {
    new Intl.DateTimeFormat('en', { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

/** Display zone (defaults to the system zone; the user may override later). */
export function setDisplayZone(tz: string): void {
  if (isValidZone(tz)) {
    zone = tz;
    clockCache.clear();
    dayCache = null;
  }
}
export function getDisplayZone(): string {
  return zone;
}

export type Epoch = number; // UTC seconds

function at(epoch: Epoch) {
  return dayjs.unix(epoch).tz(zone);
}

export const nowEpoch = (): Epoch => dayjs().unix();

/** 14:05 */
// dayjs.tz is slow (it builds an Intl formatter per call) and the guide formats thousands of block times,
// so clock strings are memoised per epoch (cleared when the display zone changes, bounded in size).
const clockCache = new Map<Epoch, string>();
const CLOCK_CACHE_MAX = 20_000;
export const formatClock = (epoch: Epoch): string => {
  const hit = clockCache.get(epoch);
  if (hit !== undefined) return hit;
  if (clockCache.size >= CLOCK_CACHE_MAX) clockCache.clear();
  const s = at(epoch).format('HH:mm');
  clockCache.set(epoch, s);
  return s;
};
/** Thu 19 Nov */
export const formatDay = (epoch: Epoch): string => at(epoch).format('ddd D MMM');
/** Thu 19 Nov, 14:05 */
export const formatDateTime = (epoch: Epoch): string => at(epoch).format('ddd D MMM, HH:mm');
/** 14:05 – 15:00 */
export const formatRange = (start: Epoch, end: Epoch): string => `${formatClock(start)} – ${formatClock(end)}`;

/** 1:05:09 or 5:09 */
export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

/** Progress 0..1 of `now` within a programme; clamped. */
export function progress(start: Epoch, end: Epoch, now: Epoch = nowEpoch()): number {
  if (end <= start) return 0;
  return Math.min(1, Math.max(0, (now - start) / (end - start)));
}

export const isLiveNow = (start: Epoch, end: Epoch, now: Epoch = nowEpoch()): boolean => now >= start && now < end;

/** Minutes remaining, rounded up, never negative. */
export const minutesLeft = (end: Epoch, now: Epoch = nowEpoch()): number => Math.max(0, Math.ceil((end - now) / 60));

/** Epoch of local midnight (display zone) for the day containing `epoch`. */
export const startOfDay = (epoch: Epoch): Epoch => {
  // The scroll handler calls this every frame; remember the last local day as [start, end) to skip dayjs.
  if (dayCache && epoch >= dayCache.start && epoch < dayCache.end) return dayCache.start;
  // dayjs.tz().startOf('day') mis-resolves times on a clock-change day; build the local midnight explicitly.
  const start = dayjs.tz(`${at(epoch).format('YYYY-MM-DD')}T00:00:00`, zone).unix();
  dayCache = { start, end: addDays(start, 1) };
  return start;
};
let dayCache: { start: Epoch; end: Epoch } | null = null;

/** Floor to a guide tick (default 30 min) in the display zone. */
export function floorToTick(epoch: Epoch, minutes = 30): Epoch {
  const d = at(epoch);
  const m = d.minute() - (d.minute() % minutes);
  return d.minute(m).second(0).millisecond(0).unix();
}

/** "Updated 14:05" style relative label: within a day shows the clock, otherwise the date. */
export function formatUpdated(epoch: Epoch, now: Epoch = nowEpoch()): string {
  return startOfDay(epoch) === startOfDay(now) ? formatClock(epoch) : formatDateTime(epoch);
}

/** 14:05:09 (for session logs) */
export const formatClockSeconds = (epoch: Epoch): string => at(epoch).format('HH:mm:ss');

/** `n` calendar days from the display-zone midnight containing `epoch` (DST-safe). */
export function addDays(epoch: Epoch, n: number): Epoch {
  // dayjs.tz().add(n, 'day') adds n*24 h, which drifts an hour across a clock change. Add on the naive
  // wall-clock date instead and re-resolve it in the display zone.
  const naive = dayjs.utc(at(epoch).format('YYYY-MM-DDTHH:mm:ss')).add(n, 'day').format('YYYY-MM-DDTHH:mm:ss');
  return dayjs.tz(naive, zone).unix();
}

/** "Today", "Tomorrow", "Yesterday" or "Thu 20 Nov". */
export function dayLabel(epoch: Epoch, now: Epoch = nowEpoch()): string {
  const diff = Math.round((startOfDay(epoch) - startOfDay(now)) / 86400);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Tomorrow';
  if (diff === -1) return 'Yesterday';
  return formatDay(epoch);
}

// ---- Epoch arithmetic helpers (rule 11: no other module does date math or owns these constants) ----

/** One guide tick, in seconds. */
export const TICK_SECONDS = 1800;
export const hoursToSeconds = (h: number): number => h * 3600;
export const daysToSeconds = (d: number): number => d * 86400;

/** Guide anchor for "go to now": one tick before now, floored to the tick grid. */
export const guideNowAnchor = (now: Epoch = nowEpoch()): Epoch => floorToTick(now - TICK_SECONDS);

/** Earliest instant still inside a `days`-day archive window ending at `now`. */
export const archiveWindowStart = (days: number, now: Epoch): Epoch => now - daysToSeconds(days);

/** `hours` before `now`, floored to the tick grid (start of a "recent schedule" window). */
export const recentWindowStart = (hours: number, now: Epoch = nowEpoch()): Epoch => floorToTick(now - hoursToSeconds(hours));

/** Whole hours and rounded minutes of a duration in seconds. */
export function splitHoursMinutes(totalSeconds: number): { h: number; m: number } {
  return { h: Math.floor(totalSeconds / 3600), m: Math.round((totalSeconds % 3600) / 60) };
}
