// Synthetic VOD + DVR data for the mock backend (CLAUDE rule 16: no real titles or providers).
import type { Episode, Movie, Recording, RecordingRule, RecordingsData, Season, Series, SeriesDetail, VodCategory, VodListArgs } from './types';

const WORDS_A = ['Silent', 'Golden', 'Northern', 'Hidden', 'Paper', 'Electric', 'Midnight', 'Crimson', 'Distant', 'Hollow', 'Velvet', 'Iron', 'Last', 'Winter', 'Glass', 'Wild'];
const WORDS_B = ['Harbour', 'Garden', 'Signal', 'Orchard', 'Lantern', 'Station', 'River', 'Compass', 'Engine', 'Archive', 'Meadow', 'Tower', 'Voyage', 'Season', 'Echo', 'Frontier'];
const GENRES = ['Drama', 'Comedy', 'Thriller', 'Documentary', 'Animation', 'Science fiction', 'Family'];
const MOVIE_CATS = ['Action', 'Comedy', 'Drama', 'Family', 'Documentary', 'Sci-fi'];
const SERIES_CATS = ['Drama', 'Comedy', 'Crime', 'Reality', 'Kids'];
const PLOT = 'A synthetic plot summary for the mock backend. Two people, one long night, and a decision that changes the rest of the season.';

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
    durationSecs: (80 + ((i * 13) % 70)) * 60, poster: null, backdrop: null, plot: PLOT,
    genres: [GENRES[i % GENRES.length]!, GENRES[(i + 3) % GENRES.length]!], categoryId: (i % MOVIE_CATS.length) + 1,
    resumeSecs: i % 6 === 0 ? 1800 + i : null, watched: i % 10 === 3,
  }));
  return movies;
}

function allSeries(): Series[] {
  seriesList ??= Array.from({ length: 320 }, (_, i): Series => ({
    id: i + 1, title: title(i + 5), year: 1995 + ((i * 3) % 30), rating: 5 + ((i * 3) % 45) / 10, poster: null, backdrop: null,
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

function seed(now: number): void {
  if (recs) return;
  const mk = (i: number, status: Recording['status'], off: number, dur: number, extra: Partial<Recording> = {}): Recording => ({
    id: i, title: title(i * 3), subtitle: i % 2 ? ['Low Tide', 'Second Light', 'Paper Boats', 'Signal and Noise', 'The Lantern Keeper'][i % 5]! : null,
    description: 'Synthetic recording description from the guide.', season: i % 2 ? 1 + (i % 3) : null, episode: i % 2 ? i : null,
    channelId: 100 + (i % 6), channelName: ['Harbor News 24', 'Meridian One', 'Cobalt Sports', 'Lantern Cinema', 'Orchard Kids', 'Northwind Docs'][i % 6]!,
    startUtc: now + off, endUtc: now + off + dur, status, ruleId: null, ruleLabel: null, sizeBytes: status === 'completed' ? 800_000_000 + i * 90_000_000 : null, error: null, ...extra,
  });
  recs = [
    mk(1, 'scheduled', 3600, 3600, { ruleId: 1, ruleLabel: 'Series' }), mk(2, 'scheduled', 86400, 1800), mk(3, 'scheduled', 90000, 3600, { ruleId: 1, ruleLabel: 'Series' }),
    mk(4, 'scheduled', 2 * 86400, 5400, { ruleId: 2, ruleLabel: 'Weekdays 19:00' }),
    mk(5, 'recording', -1500, 3600),
    mk(6, 'completed', -86400, 3600), mk(7, 'completed', -2 * 86400, 1800, { ruleId: 1, ruleLabel: 'Series' }), mk(8, 'completed', -3 * 86400, 5400),
    mk(9, 'failed', -4 * 86400, 3600, { error: 'The channel stopped sending video, so this recording is incomplete.', sizeBytes: 120_000_000 }),
  ];
  rules = [
    { id: 1, title: title(3), channelName: 'Harbor News 24', kind: 'series', schedule: 'New episodes' },
    { id: 2, title: title(12), channelName: 'Northwind Docs', kind: 'recurring', schedule: 'Weekdays 19:00' },
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
export function addRecording(now: number, r: Omit<Recording, 'id' | 'status' | 'sizeBytes' | 'error' | 'ruleId' | 'ruleLabel'>, series: boolean): void {
  seed(now);
  let ruleId: number | null = null;
  if (series) {
    ruleId = Math.max(0, ...rules!.map((x) => x.id)) + 1;
    rules!.push({ id: ruleId, title: r.title, channelName: r.channelName, kind: 'series', schedule: 'New episodes' });
  }
  recs!.push({ ...r, id: ++nextRec, status: 'scheduled', sizeBytes: null, error: null, ruleId, ruleLabel: series ? 'Series' : null });
}
export function updateRecording(id: number, f: (r: Recording) => Recording | null): void {
  recs = (recs ?? []).flatMap((r) => (r.id === id ? (f(r) ? [f(r)!] : []) : [r]));
}
export const getRecording = (id: number) => recs?.find((r) => r.id === id);
export const listRules = (now: number) => (seed(now), rules!);
export function deleteRule(id: number): void {
  rules = (rules ?? []).filter((r) => r.id !== id);
  recs = (recs ?? []).filter((r) => !(r.ruleId === id && r.status === 'scheduled'));
}
