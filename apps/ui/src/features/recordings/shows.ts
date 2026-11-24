// Pure: folds the DVR lists and the rules into "shows" for the Recordings page (list + detail).
import type { Recording, RecordingRule, RecordingsData } from '../../api/types';

export interface ShowChannel {
  id: number | null;
  name: string;
  logo: string | null;
}

export interface Show {
  /** `rule:<id>` when a rule owns the show, else `title:<normalised title>`. */
  key: string;
  title: string;
  rule: RecordingRule | null;
  /** Running first, then recorded (failed included), newest first. */
  recordings: Recording[];
  /** Soonest first. */
  upcoming: Recording[];
  /** Distinct channels the show was or will be recorded from, most recent first. */
  channels: ShowChannel[];
  /** Completed and never played. */
  newCount: number;
  recordingNow: boolean;
}

/** Selection on the Recordings page: a show key, or the "Coming up" view. */
export const UPCOMING = 'upcoming';

/** "New": completed and never played (no saved position, not watched). */
export const isNew = (r: Recording): boolean => r.status === 'completed' && r.positionSecs == null && !r.watched;

const norm = (title: string) => title.trim().toLowerCase();

export function groupShows(data: RecordingsData, rules: RecordingRule[]): Show[] {
  const shows = new Map<string, Show>();
  const byTitle = new Map<string, string>();
  const ensure = (key: string, title: string, rule: RecordingRule | null): Show => {
    let s = shows.get(key);
    if (!s) {
      s = { key, title, rule, recordings: [], upcoming: [], channels: [], newCount: 0, recordingNow: false };
      shows.set(key, s);
    }
    if (!byTitle.has(norm(title))) byTitle.set(norm(title), key);
    return s;
  };
  // Rules first, so one-off recordings with the same title join the rule's show, and rules with nothing
  // under them still appear (they can only be managed here).
  for (const rule of rules) ensure(`rule:${rule.id}`, rule.title, rule);

  const place = (r: Recording): Show => {
    const ruleKey = r.ruleId != null ? `rule:${r.ruleId}` : null;
    const key = (ruleKey && shows.has(ruleKey) ? ruleKey : null) ?? byTitle.get(norm(r.title)) ?? `title:${norm(r.title)}`;
    return ensure(key, r.title, null);
  };
  for (const r of data.inProgress) place(r).recordings.push(r);
  for (const r of data.completed) place(r).recordings.push(r);
  for (const r of data.upcoming) place(r).upcoming.push(r);

  for (const s of shows.values()) {
    s.recordings.sort((a, b) => Number(b.status === 'recording') - Number(a.status === 'recording') || b.startUtc - a.startUtc);
    s.upcoming.sort((a, b) => a.startUtc - b.startUtc);
    s.recordingNow = s.recordings.some((r) => r.status === 'recording');
    s.newCount = s.recordings.filter(isNew).length;
    const seen = new Set<string>();
    for (const r of [...s.recordings, ...s.upcoming]) {
      if (seen.has(r.channelName)) continue;
      seen.add(r.channelName);
      s.channels.push({ id: r.channelId, name: r.channelName, logo: r.channelLogo });
    }
    if (s.channels.length === 0 && s.rule?.channelName) s.channels.push({ id: null, name: s.rule.channelName, logo: null });
  }
  return [...shows.values()].sort(compareShows);
}

/** Recording now, then most recently recorded, then soonest upcoming, then rules with nothing yet (A–Z). */
function compareShows(a: Show, b: Show): number {
  if (a.recordingNow !== b.recordingNow) return a.recordingNow ? -1 : 1;
  const la = a.recordings[0]?.startUtc;
  const lb = b.recordings[0]?.startUtc;
  if (la != null || lb != null) return la == null ? 1 : lb == null ? -1 : lb - la;
  const ua = a.upcoming[0]?.startUtc;
  const ub = b.upcoming[0]?.startUtc;
  if (ua != null || ub != null) return ua == null ? 1 : ub == null ? -1 : ua - ub;
  return a.title.localeCompare(b.title);
}

/** Where the page opens: the saved selection if it still exists, else the show recording now, else the newest. */
export function initialSelection(shows: Show[], saved: string | null): string {
  if (saved === UPCOMING || (saved && shows.some((s) => s.key === saved))) return saved;
  return shows[0]?.key ?? UPCOMING;
}

/** Fraction watched (0..1) for the progress bar; null when there is nothing to show. */
export function watchedFraction(r: Recording): number | null {
  if (r.watched) return 1;
  if (r.positionSecs == null || r.durationSecs <= 0) return null;
  return Math.min(1, r.positionSecs / r.durationSecs);
}
