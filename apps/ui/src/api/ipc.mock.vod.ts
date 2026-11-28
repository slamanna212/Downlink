// Synthetic VOD + DVR data for the mock backend (CLAUDE rule 16: no real titles or providers).
import type { Episode, Movie, VodHit, Recording, RecordingRule, RecordingsData, Season, Series, SeriesDetail, VodCategory, VodListArgs } from './types';

const WORDS_A = ['Silent', 'Golden', 'Northern', 'Hidden', 'Paper', 'Electric', 'Midnight', 'Crimson', 'Distant', 'Hollow', 'Velvet', 'Iron', 'Last', 'Winter', 'Glass', 'Wild'];
const WORDS_B = ['Harbour', 'Garden', 'Signal', 'Orchard', 'Lantern', 'Station', 'River', 'Compass', 'Engine', 'Archive', 'Meadow', 'Tower', 'Voyage', 'Season', 'Echo', 'Frontier'];
const GENRES = ['Drama', 'Comedy', 'Thriller', 'Documentary', 'Animation', 'Science fiction', 'Family'];
const MOVIE_CATS = ['Action', 'Comedy', 'Drama', 'Family', 'Documentary', 'Sci-fi'];
const SERIES_CATS = ['Drama', 'Comedy', 'Crime', 'Reality', 'Kids'];
const PLOT = 'A synthetic plot summary for the mock backend. Two people, one long night, and a decision that changes the rest of the season.';

// `?mock=showcase` (marketing screenshots): posters point at `/showcase/...`, which the screenshot spec serves.
const showcase = typeof location !== 'undefined' && new URLSearchParams(location.search).get('mock') === 'showcase';
const art = (kind: 'poster' | 'backdrop', i: number): string | null => (showcase ? `/showcase/${kind}/${i % 24}` : null);

const title = (i: number) => `${WORDS_A[i % WORDS_A.length]} ${WORDS_B[(i * 7 + Math.floor(i / 16)) % WORDS_B.length]}${i >= 256 ? ` ${Math.floor(i / 256) + 1}` : ''}`;

let movies: Movie[] | null = null;
let seriesList: Series[] | null = null;
const watched = new Map<string, number>(); // `${kind}:${id}` -> secs

export function vodCategories(kind: 'movie' | 'series'): VodCategory[] {
  const names = kind === 'movie' ? MOVIE_CATS : SERIES_CATS;
  const list = kind === 'movie' ? allMovies() : allSeries();
  return names.map((name, i) => ({ id: i + 1, name, count: list.filter((m) => m.categoryId === i + 1).length }));
}

function allMovies(): Movie[] {
  movies ??= Array.from({ length: 2400 }, (_, i): Movie => ({
    id: i + 1, title: title(i), year: 1980 + ((i * 11) % 45), rating: i % 9 === 0 ? null : 4 + ((i * 7) % 55) / 10,
    durationSecs: (80 + ((i * 13) % 70)) * 60, poster: art('poster', i), backdrop: art('backdrop', i), plot: PLOT,
    genres: [GENRES[i % GENRES.length]!, GENRES[(i + 3) % GENRES.length]!], categoryId: (i % MOVIE_CATS.length) + 1,
    resumeSecs: i % 6 === 0 ? 1800 + i : null, watched: i % 10 === 3,
  }));
  return movies;
}

function allSeries(): Series[] {
  seriesList ??= Array.from({ length: 320 }, (_, i): Series => ({
    id: i + 1, title: title(i + 5), year: 1995 + ((i * 3) % 30), rating: 5 + ((i * 3) % 45) / 10, poster: art('poster', i + 7), backdrop: art('backdrop', i + 7),
    plot: PLOT, genres: [GENRES[i % GENRES.length]!], categoryId: (i % SERIES_CATS.length) + 1, seasonCount: 1 + (i % 4),
  }));
  return seriesList;
}

const page = <T extends { title: string; categoryId: number | null }>(list: T[], a: VodListArgs): T[] => {
  const q = a.query?.trim().toLowerCase();
  const out = list.filter((m) => (a.categoryId == null || m.categoryId === a.categoryId) && (!q || m.title.toLowerCase().includes(q)));
  return out.slice(a.offset ?? 0, (a.offset ?? 0) + (a.limit ?? 5000));
};

const withResume = (m: Movie): Movie => {
  const w = watched.get(`movie:${m.id}`);
  return w === undefined ? m : { ...m, resumeSecs: w, watched: false };
};

export const countMovies = (a: VodListArgs) => page(allMovies(), { ...a, offset: 0, limit: Infinity }).length;
export const countSeries = (a: VodListArgs) => page(allSeries(), { ...a, offset: 0, limit: Infinity }).length;
/** Global search: movies, then series, whose title contains `q` (already lower-cased). */
export const searchVod = (q: string, limit: number): VodHit[] => [
  ...allMovies().filter((m) => m.title.toLowerCase().includes(q)).map((m): VodHit => ({ id: m.id, kind: 'movie', title: m.title, year: m.year, poster: m.poster })),
  ...allSeries().filter((s) => s.title.toLowerCase().includes(q)).map((s): VodHit => ({ id: s.id, kind: 'series', title: s.title, year: s.year, poster: s.poster })),
].slice(0, limit);
export const listMovies = (a: VodListArgs) => page(allMovies(), a).map(withResume);
export const getMovie = (id: number): Movie | undefined => {
  const m = allMovies().find((x) => x.id === id);
  return m && withResume(m);
};
export const listSeries = (a: VodListArgs) => page(allSeries(), a);

export function getSeries(id: number): SeriesDetail | undefined {
  const s = allSeries().find((x) => x.id === id);
  if (!s) return undefined;
  const seasons: Season[] = Array.from({ length: s.seasonCount }, (_, si) => ({
    number: si + 1,
    episodes: Array.from({ length: 6 + ((id + si) % 5) }, (_, ei): Episode => {
      const eid = id * 1000 + si * 100 + ei + 1;
      const w = watched.get(`episode:${eid}`);
      const done = si === 0 && ei < 2;
      return {
        id: eid, number: ei + 1, title: `${WORDS_B[(id + si + ei) % WORDS_B.length]} ${ei + 1}`, plot: PLOT,
        durationSecs: (42 + ((ei * 5) % 15)) * 60, still: null, resumeSecs: w ?? (si === 0 && ei === 2 ? 900 : null), watched: done,
      };
    }),
  }));
  const flat = seasons.flatMap((x) => x.episodes);
  const next = flat.find((e) => e.resumeSecs) ?? flat.find((e) => !e.watched) ?? null;
  return { series: s, seasons, nextEpisodeId: next?.id ?? null };
}

export function vodInfo(kind: 'movie' | 'episode', id: number): { title: string; durationSecs: number; resumeSecs: number | null } | undefined {
  if (kind === 'movie') {
    const m = getMovie(id);
    return m && { title: m.title, durationSecs: m.durationSecs ?? 5400, resumeSecs: m.resumeSecs };
  }
  const sid = Math.floor(id / 1000);
  const ep = getSeries(sid)?.seasons.flatMap((x) => x.episodes).find((e) => e.id === id);
  const s = allSeries().find((x) => x.id === sid);
  return ep && s && { title: `${s.title} · ${ep.title}`, durationSecs: ep.durationSecs ?? 2700, resumeSecs: ep.resumeSecs };
}
export const saveProgress = (kind: 'movie' | 'episode', id: number, secs: number) => watched.set(`${kind}:${id}`, secs);

// ---- DVR ----
let recs: Recording[] | null = null;
let rules: RecordingRule[] | null = null;
let nextRec = 100;

const CHANNEL_NAMES = ['Harbor News 24', 'Meridian One', 'Cobalt Sports', 'Lantern Cinema', 'Orchard Kids', 'Northwind Docs'];

function seed(now: number): void {
  if (recs) return;
  let id = 0;
  const day = 86400;
  // Shows with several episodes, across channels, at every stage: recording now, unwatched, part-watched,
  // watched, interrupted (with and without a partial file), upcoming, and a rule with nothing under it yet.
  const mk = (show: string, ch: number, status: Recording['status'], off: number, dur: number, extra: Partial<Recording> = {}): Recording => {
    id += 1;
    return {
      id, title: show, subtitle: null, description: 'Synthetic recording description from the guide.', season: null, episode: null,
      channelId: 100 + ch, channelName: CHANNEL_NAMES[ch]!, channelLogo: null,
      startUtc: now + off, endUtc: now + off + dur, status, ruleId: null, ruleLabel: null,
      sizeBytes: status === 'completed' ? 800_000_000 + id * 90_000_000 : null, error: null,
      programmeId: status === 'scheduled' ? `rec-prog-${id}` : null,
      positionSecs: null, durationSecs: dur, watched: false, playable: status !== 'scheduled', ...extra,
    };
  };
  const cup = { ruleId: 1, ruleLabel: 'Recurring' };
  const rally = { ruleId: 2, ruleLabel: 'Series' };
  const quiz = { ruleId: 3, ruleLabel: 'Series' };
  recs = [
    mk('Coastline Cup', 2, 'recording', -2 * 3600, 7.5 * 3600, { ...cup, subtitle: 'Day 1' }),
    mk('Coastline Cup', 2, 'scheduled', day - 2 * 3600, 7.5 * 3600, { ...cup, subtitle: 'Day 2' }),
    mk('Coastline Cup', 2, 'scheduled', 2 * day - 2 * 3600, 7.5 * 3600, { ...cup, subtitle: 'Day 3' }),
    mk('Ridgeline Rally', 1, 'completed', -2 * day, 3 * 3600 + 600, { ...rally, subtitle: 'Valley stage · Race' }),
    mk('Ridgeline Rally', 2, 'completed', -3 * day, 6900, { ...rally, subtitle: 'Valley stage · Qualifying', positionSecs: 2760 }),
    mk('Ridgeline Rally', 2, 'completed', -4 * day, 3300, { ...rally, subtitle: 'Valley stage · Practice 3', watched: true }),
    mk('Ridgeline Rally', 2, 'scheduled', 3 * day, 3600, { ...rally, subtitle: 'Coast stage · Practice 1' }),
    mk('Ridgeline Rally', 1, 'scheduled', 4 * day, 9000, { ...rally, subtitle: 'Coast stage · Race 1' }),
    mk('Night Quiz', 0, 'completed', -6 * day, 1800, { ...quiz, season: 10, episode: 8, watched: true }),
    mk('Night Quiz', 0, 'completed', -13 * day, 1800, { ...quiz, season: 10, episode: 7, watched: true }),
    mk('Night Quiz', 0, 'scheduled', 3 * 3600, 1800, { ...quiz, season: 10, episode: 9 }),
    mk('Paper Kites', 5, 'completed', -6 * day, 1800, { season: 17, episode: 4, positionSecs: 480 }),
    mk('Paper Kites', 5, 'scheduled', 4 * 3600, 1800, { season: 17, episode: 5 }),
    mk('Garden Hours', 5, 'completed', -5 * day, 3480, { season: 36, episode: 32 }),
    mk('Harbor Derby', 3, 'failed', -9 * day, 7200, { subtitle: 'North v South', error: 'The stream dropped after 1 h 12 min, so this recording is incomplete.', sizeBytes: 120_000_000 }),
    mk('Lantern Shorts', 3, 'failed', -10 * day, 3600, { error: 'The provider refused the connection.', playable: false }),
  ];
  rules = [
    { id: 1, title: 'Coastline Cup', channelName: 'Cobalt Sports', kind: 'recurring', schedule: 'Every day 10:30' },
    { id: 2, title: 'Ridgeline Rally', channelName: '', kind: 'series', schedule: 'All episodes' },
    { id: 3, title: 'Night Quiz', channelName: 'Harbor News 24', kind: 'series', schedule: 'New episodes' },
    { id: 4, title: 'Morning Tides', channelName: 'Harbor News 24', kind: 'series', schedule: 'New episodes' },
  ];
}

export function listRecordings(now: number): RecordingsData {
  seed(now);
  const r = recs!;
  return {
    upcoming: r.filter((x) => x.status === 'scheduled').sort((a, b) => a.startUtc - b.startUtc),
    inProgress: r.filter((x) => x.status === 'recording'),
    completed: r.filter((x) => x.status === 'completed' || x.status === 'failed').sort((a, b) => b.startUtc - a.startUtc),
  };
}
export function addRecording(
  now: number,
  r: Omit<Recording, 'id' | 'status' | 'sizeBytes' | 'error' | 'ruleId' | 'ruleLabel' | 'channelLogo' | 'positionSecs' | 'durationSecs' | 'watched' | 'playable'>,
  series: boolean,
): void {
  seed(now);
  let ruleId: number | null = null;
  if (series) {
    ruleId = Math.max(0, ...rules!.map((x) => x.id)) + 1;
    rules!.push({ id: ruleId, title: r.title, channelName: r.channelName, kind: 'series', schedule: 'New episodes' });
    // A series rule adopts the show's existing recordings, like the server's title match.
    recs = recs!.map((x) => (x.title === r.title ? { ...x, ruleId, ruleLabel: 'Series' } : x));
  }
  if (recs!.some((x) => x.programmeId === r.programmeId || (x.channelId === r.channelId && x.startUtc === r.startUtc))) return;
  recs!.push({
    ...r, id: ++nextRec, status: 'scheduled', sizeBytes: null, error: null, ruleId, ruleLabel: series ? 'Series' : null,
    channelLogo: null, positionSecs: null, durationSecs: r.endUtc - r.startUtc, watched: false, playable: false,
  });
}
/** Start a series rule from an upcoming recording's programme (mock ids are `rec-prog-*`). */
export function seriesFromRecording(now: number, programmeId: string): boolean {
  seed(now);
  const r = recs!.find((x) => x.programmeId === programmeId);
  if (!r) return false;
  addRecording(now, r, true);
  return true;
}
export function updateRecording(id: number, f: (r: Recording) => Recording | null): void {
  recs = (recs ?? []).flatMap((r) => (r.id === id ? (f(r) ? [f(r)!] : []) : [r]));
}
export const getRecording = (id: number) => recs?.find((r) => r.id === id);
export const listRules = (now: number) => (seed(now), rules!);
export function deleteRule(id: number): void {
  rules = (rules ?? []).filter((r) => r.id !== id);
  recs = (recs ?? [])
    .filter((r) => !(r.ruleId === id && r.status === 'scheduled'))
    .map((r) => (r.ruleId === id ? { ...r, ruleId: null, ruleLabel: null } : r));
}
