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
  if (isValidZone(tz)) zone = tz;
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
export const formatClock = (epoch: Epoch): string => at(epoch).format('HH:mm');
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
export const startOfDay = (epoch: Epoch): Epoch => at(epoch).startOf('day').unix();

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
