import { formatDuration, splitHoursMinutes } from '../../lib/time';
import { t } from '../../lib/i18n';

/** A saved position is only worth offering when it is more than 2 s in and more than 2 s from the end. */
export function resumeUseful(resumeSecs: number | null | undefined, durationSecs: number | null | undefined): resumeSecs is number {
  if (resumeSecs == null || resumeSecs <= 2) return false;
  return durationSecs == null || durationSecs - resumeSecs > 2;
}

export const progressOf = (resume: number | null, duration: number | null): number | null =>
  resumeUseful(resume, duration) && duration ? Math.min(1, resume / duration) : null;

export function runtimeLabel(secs: number | null): string | null {
  if (!secs) return null;
  const { h, m } = splitHoursMinutes(secs);
  return h ? t('vod.runtimeHm', { h, m }) : t('vod.runtimeM', { m });
}

export const clock = formatDuration;

/** Columns that fit `width` px with tiles of at least `minTile` and `gap` between them. */
export function gridColumns(width: number, minTile: number, gap: number): number {
  return Math.max(1, Math.floor((width + gap) / (minTile + gap)));
}

/** Critic/audience score as shown on tiles and detail pages: one decimal, hidden when absent. */
export function ratingLabel(rating: number | null | undefined): string | null {
  return rating != null && Number.isFinite(rating) && rating > 0 ? rating.toFixed(1) : null;
}
