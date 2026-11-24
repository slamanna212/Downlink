import { describe, expect, it } from 'vitest';
import type { Recording, RecordingRule } from '../../api/types';
import { UPCOMING, groupShows, initialSelection, isNew, watchedFraction } from './shows';

const T = 1_800_000_000;
let nextId = 0;
const rec = (title: string, status: Recording['status'], start: number, extra: Partial<Recording> = {}): Recording => ({
  id: ++nextId, title, subtitle: null, description: null, season: null, episode: null, channelId: 100, channelName: 'Harbor News 24',
  startUtc: start, endUtc: start + 3600, status, ruleId: null, ruleLabel: null, sizeBytes: null, error: null,
  channelLogo: null, programmeId: null, positionSecs: null, durationSecs: 3600, watched: false, playable: status !== 'scheduled', ...extra,
});
const rule = (id: number, title: string): RecordingRule => ({ id, title, channelName: '', kind: 'series', schedule: 'New episodes' });

describe('groupShows', () => {
  it('folds recordings, upcoming and rules into shows by rule, then by title', () => {
    const shows = groupShows(
      {
        inProgress: [],
        completed: [
          rec('Night Quiz', 'completed', T - 100, { ruleId: 1 }),
          rec('night quiz ', 'completed', T - 200), // one-off of the same show joins the rule's show
          rec('Paper Kites', 'completed', T - 300, { channelId: 101, channelName: 'Meridian One' }),
        ],
        upcoming: [rec('Paper Kites', 'scheduled', T + 100, { channelId: 102, channelName: 'Cobalt Sports' })],
      },
      [rule(1, 'Night Quiz'), rule(2, 'Morning Tides')],
    );
    expect(shows.map((s) => [s.key, s.recordings.length, s.upcoming.length])).toEqual([
      ['rule:1', 2, 0],
      ['title:paper kites', 1, 1],
      ['rule:2', 0, 0], // a rule with nothing under it still shows, last
    ]);
    expect(shows[1]!.channels.map((c) => c.name)).toEqual(['Meridian One', 'Cobalt Sports']);
  });

  it('puts the show recording now first, running episode first, and counts never-played as new', () => {
    const shows = groupShows(
      {
        inProgress: [rec('Coastline Cup', 'recording', T - 7200)],
        completed: [
          rec('Coastline Cup', 'completed', T - 86400),
          rec('Ridgeline Rally', 'completed', T - 60, { positionSecs: 600 }),
          rec('Ridgeline Rally', 'completed', T - 120),
          rec('Ridgeline Rally', 'completed', T - 180, { watched: true }),
        ],
        upcoming: [],
      },
      [],
    );
    expect(shows.map((s) => s.title)).toEqual(['Coastline Cup', 'Ridgeline Rally']);
    expect(shows[0]!.recordingNow).toBe(true);
    expect(shows[0]!.recordings[0]!.status).toBe('recording');
    expect(shows[1]!.newCount).toBe(1);
  });
});

describe('selection and progress', () => {
  const shows = groupShows({ inProgress: [], completed: [rec('A', 'completed', T)], upcoming: [] }, []);
  it('keeps a saved selection that still exists, else opens the first show', () => {
    expect(initialSelection(shows, UPCOMING)).toBe(UPCOMING);
    expect(initialSelection(shows, 'title:a')).toBe('title:a');
    expect(initialSelection(shows, 'rule:99')).toBe('title:a');
    expect(initialSelection([], null)).toBe(UPCOMING);
  });
  it('reports new and watched fractions', () => {
    const r = rec('A', 'completed', T);
    expect(isNew(r)).toBe(true);
    expect(isNew({ ...r, status: 'failed' })).toBe(false);
    expect(watchedFraction(r)).toBeNull();
    expect(watchedFraction({ ...r, positionSecs: 900 })).toBe(0.25);
    expect(watchedFraction({ ...r, watched: true })).toBe(1);
  });
});
