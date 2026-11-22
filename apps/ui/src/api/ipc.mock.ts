// Mock backend for browser dev and Storybook. SYNTHETIC data only (CLAUDE rule 16): no real provider
// names, URLs or EPG. `?mock=empty` in the URL simulates a fresh install with no source;
// `?mock=offline` simulates a source that is offline.
import * as vod from './ipc.mock.vod';
import type { Backend, CommandName, Commands, EventName, Events, Unlisten } from './ipc';
import {
  Cap,
  type AppError,
  type AppSettings,
  type Channel,
  type PlexStatus,
  type UpdateStatus,
  type Group,
  type NowNext,
  type PlayerState,
  type Programme,
  type Source,
  type Transition,
  type MultiviewState,
  type MultiviewTile,
  type MediaDetails,
} from './types';

const NAMES = [
  'Harbor News 24', 'Meridian One', 'Cobalt Sports', 'Lantern Cinema', 'Orchard Kids', 'Northwind Docs',
  'Summit Music', 'Ember Drama', 'Tidepool Nature', 'Quartz Weather', 'Foxglove Comedy', 'Atlas World',
  'Juniper Home', 'Relay Racing', 'Saffron Kitchen', 'Vesper Late', 'Kestrel Science', 'Paper Moon Classics',
];
const GROUP_NAMES = ['News', 'Sports', 'Movies', 'Kids', 'Documentary', 'Music'];
const QUALITIES: Channel['quality'][] = ['HD', 'FHD', '4K', 'SD'];
const TITLES = ['The Long Field', 'Evening Bulletin', 'Deep Water', 'Paper Planets', 'Robot Garden', 'A Quiet Harbour', 'Inside the Cell', 'Forecast', 'Late Mix', 'Fieldwork S2 E4'];

const nowSec = () => Math.floor(Date.now() / 1000);

/** Synthetic details: the "source" fields always, the TMDB-only extras when a key is set. */
export function mockDetails(base: Partial<MediaDetails> & Pick<MediaDetails, 'kind' | 'title'>, tmdb: boolean): MediaDetails {
  const episodic = base.season != null && base.episode != null;
  return {
    episodeTitle: null, season: null, episode: null, description: null, overview: null, tagline: null,
    genres: ['Drama', 'Mystery'], year: 2024, airDate: episodic ? '2024-03-02' : null, runtimeMins: 60,
    certification: 'TV-14', score: tmdb ? 8.1 : null, country: 'GB', language: 'en', network: tmdb ? 'Synthetic One' : null,
    status: tmdb && episodic ? 'Returning Series' : null, seasonCount: tmdb && episodic ? 4 : null,
    cast: [
      { name: 'Ann Example', role: 'Detective Vale', photo: null, guest: false },
      { name: 'Bo Sample', role: 'The Keeper', photo: null, guest: true },
      { name: 'Cy Placeholder', role: 'Harbourmaster', photo: null, guest: false },
      { name: 'Di Demo', role: 'Dr Moss', photo: null, guest: false },
      { name: 'Ed Mockley', role: null, photo: null, guest: false },
    ],
    crew: [
      { job: 'creator', names: ['Fay Fiction'] },
      { job: 'director', names: ['Rae Director'] },
      { job: 'writer', names: ['Wes Writer', 'Gil Draft'] },
    ],
    poster: null, backdrop: null, still: null, logo: null,
    videoQuality: 'HDTV', aspectRatio: '16:9', audio: 'stereo',
    isNew: false, isLive: false, isPremiere: false, isFinale: false, isRepeat: !base.isNew && !base.isPremiere, advisories: ['Violence'],
    trailerUrl: tmdb ? 'https://www.youtube.com/watch?v=trailerKey1' : null,
    tmdbUrl: tmdb ? 'https://www.themoviedb.org/tv/4242' : null, fromTmdb: tmdb,
    ...(tmdb && episodic ? { overview: 'A small coastal team untangles the oldest secrets of a town that would rather forget them.', episodeTitle: base.episodeTitle ?? 'The Lantern Keeper' } : {}),
    ...(tmdb ? { tagline: 'Every light has a keeper.' } : {}),
    ...Object.fromEntries(Object.entries(base).filter(([, v]) => v != null)),
  } as MediaDetails;
}

export function mockSources(empty: boolean, status: Source['status'] = 'connected'): Source[] {
  if (empty) return [];
  return [
    {
      id: 1, uuid: 'mock-source-1', kind: 'dispatcharr', name: 'Demo server', host: 'demo.invalid',
      caps: Cap.Live | Cap.Epg | Cap.Catchup | Cap.Vod | Cap.Dvr | Cap.Favourites | Cap.StreamSwitch,
      streamLimit: 4,
      status, serverVersion: '0.0-mock', lastSyncAt: nowSec() - 600, archiveDays: 7, supportsHlsTransport: true, hlsTransport: false,
    },
  ];
}

export function mockGroups(): Group[] {
  return GROUP_NAMES.map((name, i) => ({ id: i + 1, sourceId: 1, name, count: 0 }));
}

/** Inline SVG stand-in for a `dlimg://` logo so the real-logo plate renders in the browser. */
function mockLogo(name: string): string {
  const text = name.split(' ')[0]?.slice(0, 6).toUpperCase() ?? '';
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 72 36'><text x='36' y='23' font-family='sans-serif' font-size='13' font-weight='700' text-anchor='middle' fill='%23E8ECF1'>${text}</text></svg>`;
  return `data:image/svg+xml;utf8,${svg}`;
}

/** Enough channels to exercise virtualisation. */
export function mockChannels(count = 240): Channel[] {
  return Array.from({ length: count }, (_, i) => {
    const catchup = i % 3 === 0;
    const base = NAMES[i % NAMES.length] ?? `Channel ${i}`;
    return {
      id: 100 + i, sourceId: 1, uuid: `mock-channel-${i}`, number: i + 1 + (i === 4 ? 0.5 : 0),
      name: i < NAMES.length ? base : `${base} ${Math.floor(i / NAMES.length) + 1}`,
      groupId: (i % GROUP_NAMES.length) + 1,
      logo: i % 4 === 1 ? null : mockLogo(base),
      quality: QUALITIES[i % QUALITIES.length] ?? null,
      isCatchup: catchup, catchupDays: catchup ? (i % 2 === 0 ? 7 : 3) : 0, isRadio: i === 6, hasDrm: i === 11,
      isFavourite: i % 5 === 0, epgMatch: 'id',
    } satisfies Channel;
  });
}

const DURS = [30, 60, 30, 90, 45, 60, 120, 60];
const GUIDE_ANCHOR_DAYS = 5;

const scheduleCache = new Map<number, Programme[]>();

/** Full synthetic schedule for a channel (anchor .. anchor+14d), built once per channel. */
const EP_TITLES = ['The Lantern Keeper', 'Low Tide', 'A Map of Small Things', 'Signal and Noise', 'The Long Way Round', 'Paper Boats', 'Second Light', 'What the River Kept'];

function fullSchedule(channelId: number): Programme[] {
  const hit = scheduleCache.get(channelId);
  if (hit) return hit;
  const out: Programme[] = [];
  if (channelId % 13 !== 0) {
    const anchor = Math.floor(nowSec() / 86400) * 86400 - GUIDE_ANCHOR_DAYS * 86400;
    const end = anchor + 14 * 86400;
    let t = anchor + ((channelId * 7) % 4) * 900;
    for (let i = 0; t < end; i++) {
      const dur = (DURS[(channelId * 7 + i * 3) % DURS.length] ?? 60) * 60;
      out.push({
        id: `${channelId}:${t}`, channelId, startUtc: t, endUtc: t + dur, title: TITLES[(channelId + i) % TITLES.length] ?? 'Programme',
        // Every other slot is an episode of a series, so S/E, episode titles and flags show up in the UI.
        subtitle: i % 2 === 0 ? EP_TITLES[(channelId + i) % EP_TITLES.length] ?? null : null,
        description: 'Synthetic programme description. Headlines, features and a closing segment, generated for the mock backend.',
        season: i % 2 === 0 ? 1 + ((channelId + i) % 4) : null, episode: i % 2 === 0 ? 1 + (i % 12) : null,
        isNew: i % 7 === 0, isLive: channelId % 6 === 2 && i % 2 === 1, isPremiere: i % 23 === 0, isFinale: i % 29 === 0, isScheduled: false,
      });
      t += dur;
    }
  }
  scheduleCache.set(channelId, out);
  return out;
}

/** Deterministic synthetic schedule for a channel over [start, end). Some channels have no guide. */
export function genProgrammes(channelId: number, start: number, end: number, scheduled: ReadonlySet<string> = new Set()): Programme[] {
  const all = fullSchedule(channelId);
  let lo = 0;
  let hi = all.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((all[mid]?.endUtc ?? 0) <= start) lo = mid + 1;
    else hi = mid;
  }
  const out: Programme[] = [];
  for (let i = lo; i < all.length && (all[i]?.startUtc ?? 0) < end; i++) {
    const p = all[i]!;
    out.push(scheduled.has(p.id) ? { ...p, isScheduled: true } : p);
  }
  return out;
}

export function mockNowNext(channelId: number, now = nowSec(), scheduled?: ReadonlySet<string>): NowNext {
  const list = genProgrammes(channelId, now - 4 * 3600, now + 4 * 3600, scheduled);
  const idx = list.findIndex((p) => p.startUtc <= now && p.endUtc > now);
  return { channelId, now: idx >= 0 ? (list[idx] ?? null) : null, next: idx >= 0 ? (list[idx + 1] ?? null) : null };
}

/** Synthetic tracks: two audio languages and subtitles including a teletext page. */
export function mockTracks(): PlayerState['tracks'] {
  return [
    { id: 1, kind: 'audio', lang: 'eng', title: 'Main', codec: 'ac3', channels: '5.1', default: true, selected: true },
    { id: 2, kind: 'audio', lang: 'deu', title: null, codec: 'mp2', channels: 'stereo', default: false, selected: false },
    { id: 1, kind: 'sub', lang: 'eng', title: 'Teletext 888', codec: 'dvb_teletext', channels: null, default: false, selected: false },
    { id: 2, kind: 'sub', lang: 'fra', title: null, codec: 'dvb_subtitle', channels: null, default: false, selected: false },
  ];
}

export function idleState(): PlayerState {
  return {
    state: 'Idle', mode: null, channelId: null, position: null, duration: null, buffered: null, seekable: false,
    volume: 100, muted: false, hwdecCurrent: null, videoCodec: null, width: null, height: null, waitingOn: null, reconnecting: false,
    error: null, transitionedAt: Date.now(), programme: null, title: null, mediaId: null, resumeAt: null, tracks: [], timeline: null,
  };
}

const appError = (code: AppError['code'], message: string): AppError => ({ code, message, retryAfter: null });

export function createMockBackend(): Backend {
  const mode = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('mock') : null;
  const failParam = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('fail') : null;
  const failId = failParam ? Number(failParam) : null;
  // `?mock=newerdb` / `?mock=cert`: catalogue calls fail with the blocking messages until the user resolves them.
  let blockedBy: 'newerdb' | 'cert' | null = mode === 'newerdb' || mode === 'cert' ? mode : null;
  const blockedError = (): AppError =>
    blockedBy === 'newerdb' ? appError('local', 'This library was created by a newer version of Downlink.') : appError('server', 'The server’s certificate changed.');
  const reconnectFlag = typeof location !== 'undefined' && new URLSearchParams(location.search).has('reconnect');
  const failReason = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('reason') : null;
  const failError = (): AppError =>
    failReason === 'limit' ? { ...appError('stream', 'limit'), reason: 'limit' }
    : failReason === 'codec' ? { ...appError('media', 'codec'), reason: 'codec', codec: 'AV1 10-bit' }
    : failReason === 'engine' ? { ...appError('local', 'engine'), reason: 'engine', crashId: 'c-7f3a' }
    : appError('stream', 'This channel stopped sending video. Try again, or pick another channel.');
  const sources = mockSources(mode === 'empty', mode === 'offline' ? 'offline' : mode === 'connecting' ? 'connecting' : 'connected');
  const channels = mode === 'empty' ? [] : mockChannels(mode === 'big' ? 5000 : 240);
  const scheduled = new Set<string>();
  let tick: ReturnType<typeof setInterval> | null = null;
  // Live timeshift mock: seconds behind the edge and when the buffer started.
  let behind = 0;
  let liveSince = 0;
  const liveTimeline = (): PlayerState['timeline'] => {
    const now = nowSec();
    const bufferStartUtc = Math.max(liveSince, now - 3600);
    return { playheadUtc: now - behind, liveEdgeUtc: now, bufferStartUtc, archiveStartUtc: null, behindLiveSecs: behind, source: behind > 5 ? 'buffer' : 'live' };
  };
  const groups = mockGroups();
  let settings: AppSettings = {
    liveBufferSecs: 2, timeshiftMinutes: 60, hwdec: 'auto-safe', debugLogging: false, audioDevice: null, audioOutput: 'stereo',
    deinterlace: 'auto', hdrOutput: 'auto', subtitleSize: 100, subtitleBackground: true, resumeLastChannel: false,
    autoRefreshHours: 4, refreshDuringPlayback: true,
  };
  let plex: PlexStatus = { signedIn: false, accountName: null, serverId: null, serverName: null };
  let plexPinChecks = 0;
  let tmdbKey = mode === 'tmdb';
  let update: UpdateStatus = { channel: 'stable', current: '0.0.0-mock', available: null, notes: null, lastChecked: null, ready: false };
  let searchIndex: { programme: Programme; channel: Channel; hay: string }[] | null = null;
  let statsTimer: ReturnType<typeof setInterval> | null = null;
  let player = idleState();
  const streamInUse = new Map<number, number>();
  const MV_MAX = 4;
  const mvTiles = new Map<number, MultiviewTile>();
  let mvAudible = 0;
  const mvState = (): MultiviewState => ({
    tiles: [
      ...(player.channelId != null && player.mode === 'live' ? [{ slot: 0, channelId: player.channelId, state: player.state, failure: null }] : []),
      ...[...mvTiles.values()].sort((a, b) => a.slot - b.slot),
    ],
    audible: mvAudible,
    maxTiles: MV_MAX,
    streamLimit: 4,
  });
  const transitions: Transition[] = [];
  const handlers = new Map<EventName, Set<(p: never) => void>>();

  const seekableTicker = (mode: 'catchup' | 'vod' | 'recording') => {
    setTimeout(() => setPlayer({ state: 'Connecting' }), 150);
    setTimeout(() => {
      setPlayer({ state: 'Playing', hwdecCurrent: 'vaapi', videoCodec: 'h264', width: 1920, height: 1080, buffered: 30 });
      if (tick) clearInterval(tick);
      tick = setInterval(() => {
        if (player.mode === mode && player.state === 'Playing') setPlayer({ position: Math.min((player.position ?? 0) + 1, player.duration ?? 0), buffered: Math.min((player.position ?? 0) + 40, player.duration ?? 0) });
      }, 1000);
    }, 400);
  };
  const emit = <E extends EventName>(event: E, payload: Events[E]) =>
    handlers.get(event)?.forEach((h) => h(payload as never));
  const setPlayer = (patch: Partial<PlayerState>) => {
    const next = { ...player, ...patch, transitionedAt: Date.now() };
    if (next.state !== player.state) {
      transitions.push({ at: next.transitionedAt, from: player.state, to: next.state });
      if (transitions.length > 50) transitions.shift();
    }
    player = next;
    emit('player://state', player);
  };
  const withCounts = () => groups.map((g) => ({ ...g, count: channels.filter((c) => c.groupId === g.id).length }));
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const sync = async (sourceId: number) => {
    for (const [i, phase] of ['channels', 'guide'].entries()) {
      emit('task://progress', { sourceId, phase, done: i * 50 + 50, total: 100 });
      await wait(150);
    }
    const s = sources.find((x) => x.id === sourceId);
    if (s) {
      s.lastSyncAt = nowSec();
      s.status = 'connected';
    }
    emit('catalog://updated', { sourceId, channels: channels.length, groups: groups.length, programmes: channels.length * 24 });
  };

  const impl: { [C in CommandName]: (a: Commands[C]['args']) => Commands[C]['result'] | Promise<Commands[C]['result']> } = {
    list_sources: () => sources.map((s) => ({ ...s })),
    list_groups: () => withCounts(),
    list_channels: ({ groupId, query, favouritesOnly, offset = 0, limit = 500 }) => {
      if (blockedBy) throw blockedError();
      return channels.filter(
        (c) =>
          (groupId == null || c.groupId === groupId) &&
          (!favouritesOnly || c.isFavourite) &&
          (!query || c.name.toLowerCase().includes(query.toLowerCase()) || String(c.number ?? '').startsWith(query)),
      ).slice(offset, offset + limit);
    },
    get_channel: ({ channelId }) => {
      const channel = channels.find((c) => c.id === channelId);
      if (!channel) throw appError('local', 'That channel is no longer in the list.');
      return channel;
    },
    find_channel_number: ({ number }) => {
      let best: Channel | undefined;
      for (const channel of channels) {
        if (channel.number != null && channel.number >= number && (!best || channel.number < best.number!)) best = channel;
      }
      return best?.id ?? null;
    },
    get_programme_window: ({ channelIds, start, end }) =>
      channelIds.flatMap((id) => genProgrammes(id, start, end, scheduled)),
    get_now_next: ({ channelIds }) => channelIds.map((id) => mockNowNext(id, nowSec(), scheduled)),
    get_guide: ({ start, end, channelOffset, channelLimit, groupId }) => {
      const list = channels.filter((c) => groupId == null || c.groupId === groupId).slice(channelOffset, channelOffset + channelLimit);
      return { channels: list, programmes: list.flatMap((c) => genProgrammes(c.id, start, end, scheduled)) };
    },
    get_programmes: ({ channelId, start, end }) => genProgrammes(channelId, start, end, scheduled),
    play_catchup: ({ programmeId, resume }) => {
      const [cid, st] = programmeId.split(':').map(Number);
      const p = genProgrammes(cid ?? 0, st ?? 0, (st ?? 0) + 1, scheduled).find((x) => x.id === programmeId);
      if (!p) throw appError('stream', 'That programme is not available to play.');
      const dur = p.endUtc - p.startUtc;
      const saved = p.channelId % 2 === 0 ? Math.floor(dur / 3) : null;
      setPlayer({ state: 'Resolving', mode: 'catchup', channelId: p.channelId, programme: p, title: null, mediaId: null, position: resume && saved ? saved : 0, duration: dur, buffered: 0, seekable: true, resumeAt: resume && saved ? saved : null, error: null });
      seekableTicker('catchup');
      return null;
    },
    restart_programme: ({ channelId }) => {
      const nn = mockNowNext(channelId, nowSec(), scheduled).now;
      if (!nn) throw appError('stream', 'There is no programme to restart on this channel.');
      return impl.play_catchup({ programmeId: nn.id });
    },
    schedule_recording: ({ programmeId, kind }) => {
      scheduled.add(programmeId);
      const [cid] = programmeId.split(':').map(Number);
      const p = genProgrammes(cid ?? 0, 0, 4e9, scheduled).find((x) => x.id === programmeId);
      const ch = channels.find((c) => c.id === cid);
      if (p && ch) vod.addRecording(nowSec(), { title: p.title ?? ch.name, subtitle: p.subtitle, description: p.description, season: p.season, episode: p.episode, channelId: ch.id, channelName: ch.name, startUtc: p.startUtc, endUtc: p.endUtc }, kind === 'series');
      emit('catalog://updated', { scope: 'recordings', sourceId: 1, channels: channels.length, groups: groups.length, programmes: 0 });
      return null;
    },
    get_home: () => {
      const favs = channels.filter((c) => c.isFavourite).slice(0, 4);
      const live = channels[1];
      const cu = channels[6];
      const nnOf = (c: Channel) => mockNowNext(c.id, nowSec(), scheduled);
      return {
        continueWatching: [
          ...(live ? [{ kind: 'live' as const, id: `live:${live.id}`, channel: live, programme: nnOf(live).now, title: nnOf(live).now?.title ?? live.name, subtitle: live.name, art: null, progress: null, remainingSecs: null }] : []),
          { kind: 'movie' as const, id: 'movie:1', channel: null, programme: null, title: 'Night Train', subtitle: 'Movie · 1 h 12 min left', art: null, progress: 0.35, remainingSecs: 4320 },
          ...(cu ? [{ kind: 'catchup' as const, id: `catchup:${nnOf(cu).now?.id ?? `${cu.id}:${nowSec() - 3600}`}`, channel: cu, programme: null, title: 'Fieldwork S2 E3', subtitle: `${cu.name} · Yesterday 21:00`, art: null, progress: 0.5, remainingSecs: 1800 }] : []),
        ],
        favourites: favs,
        onNow: favs.map((c) => ({ channel: c, now: nnOf(c).now })),
        recents: channels.slice(1, 6),
      };
    },
    search: ({ query, limit }) => {
      const q = query.trim().toLowerCase();
      if (!q) return { channels: [], programmes: [] };
      const hits = channels.filter((c) => c.name.toLowerCase().includes(q) || String(c.number ?? '').startsWith(q));
      // Programme index built once (now/next of the first 300 channels): keeps the mock off the render path.
      searchIndex ??= channels.slice(0, 300).flatMap((c) => {
        const nn = mockNowNext(c.id, nowSec(), scheduled);
        return [nn.now, nn.next].filter((p): p is Programme => !!p).map((p) => ({ programme: p, channel: c, hay: (p.title ?? '').toLowerCase() }));
      });
      const progs = searchIndex.filter((x) => x.hay.includes(q)).slice(0, limit).map(({ programme, channel }) => ({ programme, channel }));
      return { channels: hits.slice(0, limit), programmes: progs };
    },
    get_channel_by_uuid: ({ uuid }) => channels.find((c) => c.uuid === uuid) ?? null,
    export_diagnostics: () => '/home/user/Downloads/downlink-diagnostics.zip',
    reveal_path: () => null,
    report_ui_timing: () => null,
    set_source_options: ({ sourceId, hlsTransport }) => {
      const src = sources.find((x) => x.id === sourceId);
      if (src) src.hlsTransport = hlsTransport;
      return null;
    },
    archive_newer_database: () => {
      blockedBy = null;
      return null;
    },
    trust_new_certificate: () => {
      blockedBy = null;
      return null;
    },
    quit_app: () => null,
    list_vod_categories: ({ kind }) => vod.vodCategories(kind),
    list_movies: (a) => vod.listMovies(a),
    get_movie: ({ id }) => {
      const m = vod.getMovie(id);
      if (!m) throw appError('stream', 'That movie is no longer in the library.');
      return m;
    },
    list_series: (a) => vod.listSeries(a),
    get_series: ({ id }) => {
      const s = vod.getSeries(id);
      if (!s) throw appError('stream', 'That series is no longer in the library.');
      return s;
    },
    play_vod: ({ kind, id, resume }) => {
      const info = vod.vodInfo(kind, id);
      if (!info) throw appError('stream', 'This title is not available to play right now. Try again later.');
      const at = resume && info.resumeSecs ? info.resumeSecs : 0;
      setPlayer({ state: 'Resolving', mode: 'vod', channelId: null, programme: null, title: info.title, mediaId: id, position: at, duration: info.durationSecs, buffered: at, seekable: true, resumeAt: null, error: null });
      seekableTicker('vod');
      return null;
    },
    list_recordings: () => vod.listRecordings(nowSec()),
    cancel_recording: ({ id }) => {
      vod.updateRecording(id, () => null);
      return null;
    },
    stop_recording: ({ id }) => {
      vod.updateRecording(id, (r) => ({ ...r, status: 'completed', endUtc: nowSec(), sizeBytes: 500_000_000 }));
      return null;
    },
    extend_recording: ({ id, minutes }) => {
      vod.updateRecording(id, (r) => ({ ...r, endUtc: r.endUtc + minutes * 60 }));
      return null;
    },
    delete_recording: ({ id }) => {
      vod.updateRecording(id, () => null);
      return null;
    },
    list_recording_rules: () => vod.listRules(nowSec()),
    delete_recording_rule: ({ id }) => {
      vod.deleteRule(id);
      return null;
    },
    play_recording: ({ id }) => {
      const r = vod.getRecording(id);
      if (!r) throw appError('stream', 'That recording is gone. It may have been deleted.');
      const live = r.status === 'recording';
      const dur = (live ? nowSec() : r.endUtc) - r.startUtc;
      setPlayer({ state: 'Resolving', mode: 'recording', channelId: r.channelId, programme: null, title: r.title, mediaId: id, position: 0, duration: dur, buffered: 0, seekable: true, resumeAt: null, error: null });
      seekableTicker('recording');
      return null;
    },
    get_player_state: () => player,
    play_channel: ({ channelId }) => {
      if (new URLSearchParams(location.search).has('failplay')) throw appError('stream', 'The server did not answer. Try again in a moment.');
      behind = 0;
      liveSince = nowSec() - 1200;
      setPlayer({ state: 'Resolving', mode: 'live', channelId, title: null, mediaId: null, error: null, programme: mockNowNext(channelId, nowSec(), scheduled).now, position: null, duration: null, buffered: null, seekable: true, resumeAt: null, timeline: liveTimeline() });
      if (tick) clearInterval(tick);
      tick = setInterval(() => {
        if (player.mode !== 'live') return;
        if (player.state === 'Paused') behind += 1;
        setPlayer({ timeline: liveTimeline() });
      }, 1000);
      setTimeout(() => setPlayer({ state: 'Connecting' }), 150);
      setTimeout(() => {
        // `?fail=<id>` in the URL makes that channel fail, to exercise the error overlay.
        if (failId !== null && failId === channelId)
          setPlayer({ state: 'Failed', error: failError() });
        else setPlayer({ state: 'Playing', hwdecCurrent: 'vaapi', videoCodec: 'h264', width: 1920, height: 1080, reconnecting: reconnectFlag, tracks: mockTracks() });
      }, 400);
      return null;
    },
    player_command: ({ command }) => {
      if (command.kind === 'pause') setPlayer({ state: 'Paused' });
      else if (command.kind === 'resume') setPlayer({ state: 'Playing' });
      else if (command.kind === 'stop') setPlayer({ ...idleState(), state: 'Stopped' });
      else if (command.kind === 'setVolume') setPlayer({ volume: command.volume });
      else if (command.kind === 'setMuted') setPlayer({ muted: command.muted });
      else if ((command.kind === 'seekTo' || command.kind === 'seekBy') && player.mode === 'live') {
        const tl = liveTimeline();
        const target = command.kind === 'seekTo' ? command.utc : (tl?.playheadUtc ?? nowSec()) + command.seconds;
        behind = Math.max(0, Math.min(nowSec() - target, nowSec() - (tl?.bufferStartUtc ?? nowSec())));
        setPlayer({ timeline: liveTimeline() });
      } else if (command.kind === 'seek') setPlayer({ position: Math.max(0, Math.min(command.seconds, player.duration ?? command.seconds)) });
      else if (command.kind === 'setAudioTrack') setPlayer({ tracks: player.tracks.map((t) => (t.kind === 'audio' ? { ...t, selected: t.id === command.id } : t)) });
      else if (command.kind === 'setSubtitleTrack') setPlayer({ tracks: player.tracks.map((t) => (t.kind === 'sub' ? { ...t, selected: t.id === command.id } : t)) });
      else if (command.kind === 'goLive' && player.channelId != null) impl.play_channel({ channelId: player.channelId });
      else if (command.kind === 'retry' && player.channelId != null) {
        if (player.mode === 'catchup' && player.programme) impl.play_catchup({ programmeId: player.programme.id });
        else impl.play_channel({ channelId: player.channelId });
      }
      return null;
    },
    set_favourite: ({ channelId, favourite }) => {
      const c = channels.find((x) => x.id === channelId);
      if (c) c.isFavourite = favourite;
      return null;
    },
    test_source: async (input) => {
      await wait(500);
      if (!input.url.trim()) throw appError('network', 'Enter the server address first.');
      if (/bad|invalid-key/i.test(input.url + (input.apiKey ?? ''))) throw appError('auth', 'The server rejected that API key. Check it and try again.');
      return { channels: 544, groups: 9, programmes: 15098, serverVersion: input.kind === 'dispatcharr' ? '0.32.0' : null, user: 'demo' };
    },
    add_source: async (input) => {
      await wait(300);
      const id = Math.max(0, ...sources.map((s) => s.id)) + 1;
      const src: Source = {
        id, uuid: `mock-source-${id}`, kind: input.kind, name: input.name || 'New source', host: 'demo.invalid',
        caps: Cap.Live | Cap.Epg | Cap.Favourites | (input.kind === 'dispatcharr' ? Cap.Catchup | Cap.Vod | Cap.Dvr : 0),
        status: 'connected', serverVersion: '0.0-mock', lastSyncAt: nowSec(), archiveDays: 7,
      };
      sources.push(src);
      if (channels.length === 0) channels.push(...mockChannels());
      emit('catalog://updated', { sourceId: id, channels: channels.length, groups: groups.length, programmes: channels.length * 24 });
      return src;
    },
    remove_source: ({ sourceId }) => {
      const i = sources.findIndex((s) => s.id === sourceId);
      if (i >= 0) sources.splice(i, 1);
      if (sources.length === 0) channels.length = 0;
      emit('catalog://updated', { sourceId, channels: channels.length, groups: groups.length, programmes: 0 });
      return null;
    },
    sync_source: ({ sourceId }) => {
      const s = sources.find((x) => x.id === sourceId);
      if (s) s.status = 'syncing';
      void sync(sourceId);
      return null;
    },
    get_recent_transitions: () => transitions.slice(-3),
    get_diagnostics: () => ({
      appVersion: '0.0.0-mock',
      serverVersions: sources.map((s) => s.serverVersion ?? 'unknown'),
      playerState: player,
      hwdecCurrent: player.hwdecCurrent,
      os: 'mock-os',
      transitions: [...transitions],
      logTail: ['12:00:00.000 INFO catalog sync ok channels=240', '12:00:01.250 INFO player idle'],
    }),
    set_debug_logging: ({ enabled }) => {
      settings = { ...settings, debugLogging: enabled };
      return null;
    },
    get_settings: () => settings,
    set_settings: ({ patch }) => {
      settings = { ...settings, ...patch };
      return settings;
    },
    set_video_bounds: () => null,
    set_stats_enabled: ({ enabled }) => {
      if (statsTimer) clearInterval(statsTimer);
      statsTimer = null;
      if (enabled) {
        const tick = () =>
          emit('session://stats', {
            resolution: '1920x1080', videoCodec: 'H.264 High', audioCodec: 'E-AC-3 5.1', bitrateKbps: 8400 + Math.round(Math.random() * 400),
            droppedFrames: 0, bufferSecs: 2 + Math.round(Math.random() * 20) / 10, deinterlace: 'bwdif · on', hwdecCurrent: 'vaapi',
            sourceIndex: 1, sourceCount: 3, fps: 59.94, hdr: false, transfer: 'bt.1886', toneMapped: null, primaries: 'bt.709', bitDepth: 8,
            audioChannels: '5.1', audioSampleRate: 48000, downloadKbps: 9000 + Math.round(Math.random() * 2000), avsyncMs: Math.round(Math.random() * 20 - 10) / 10,
            container: 'mpegts',
          });
        tick();
        statsTimer = setInterval(tick, 1000);
      }
      return null;
    },
    get_counts: () => ({ all: channels.length, favourites: channels.filter((c) => c.isFavourite).length }),
    get_multiview: () => mvState(),
    multiview_set_tile: async ({ slot, channelId }) => {
      if (slot <= 0 || slot >= MV_MAX) throw appError('local', 'That multiview slot is not available.');
      const failSlot = typeof location !== 'undefined' && new URLSearchParams(location.search).get('mvlimit') === String(slot);
      mvTiles.set(slot, { slot, channelId, state: 'Connecting', failure: null });
      emit('multiview://state', mvState());
      await wait(300);
      const t = mvTiles.get(slot);
      if (t && t.channelId === channelId) mvTiles.set(slot, failSlot ? { ...t, state: 'Failed', failure: 'limit' } : { ...t, state: 'Playing' });
      emit('multiview://state', mvState());
      return null;
    },
    multiview_remove_tile: ({ slot }) => {
      mvTiles.delete(slot);
      if (mvAudible === slot) mvAudible = 0;
      emit('multiview://state', mvState());
      return null;
    },
    multiview_set_audio: ({ slot }) => {
      mvAudible = slot;
      emit('multiview://state', mvState());
      return null;
    },
    multiview_make_main: ({ slot }) => {
      const t = mvTiles.get(slot);
      if (!t) return null;
      const main = player.channelId;
      impl.play_channel({ channelId: t.channelId });
      if (main != null) mvTiles.set(slot, { ...t, channelId: main });
      else mvTiles.delete(slot);
      emit('multiview://state', mvState());
      return null;
    },
    multiview_set_bounds: () => null,
    multiview_exit: () => {
      mvTiles.clear();
      mvAudible = 0;
      emit('multiview://state', mvState());
      return null;
    },
    get_channel_streams: ({ channelId }) => ({
      channelId,
      sourceIndex: (streamInUse.get(channelId) ?? 0) + 1,
      sourceCount: 3,
      streams: [
        { name: 'Primary', provider: 'Demo provider A', resolution: '1920x1080', fps: 29.97, videoCodec: 'h264', audioCodec: 'aac', audioChannels: 'stereo', bitrateKbps: 6468.2, viewers: 1, stale: false },
        { name: 'Backup A', provider: 'Demo provider B', resolution: '1280x720', fps: 59.94, videoCodec: 'hevc', audioCodec: 'ac3', audioChannels: '5.1', bitrateKbps: 3120, viewers: 0, stale: false },
        { name: 'Backup B', provider: null, resolution: null, fps: null, videoCodec: null, audioCodec: null, audioChannels: null, bitrateKbps: null, viewers: null, stale: true },
      ].map((s, index) => ({ index, streamId: `s${channelId}-${index}`, ...s })),
    }),
    switch_stream: ({ channelId, index }) => {
      if (player.channelId !== channelId || player.mode !== 'live') throw appError('local', 'Switch streams while the channel is playing live.');
      streamInUse.set(channelId, index);
      return null;
    },
    get_video_caps: () => ({ rtxVideo: true, rtxAdapter: 'Demo RTX adapter' }),
    list_audio_devices: () => [{ id: 'auto', name: 'Autoselect device' }, { id: 'hdmi-1', name: 'Demo TV (HDMI)' }, { id: 'usb-1', name: 'Demo headset (USB)' }],
    get_update_status: () => update,
    get_release_notes: ({ version }) => ({ version, body: update.notes ?? null, source: 'manifest' as const }),
    open_external_link: () => null,
    set_update_channel: ({ channel }) => {
      update = { ...update, channel, available: null, notes: null, ready: false };
      emit('update://status', update);
      return update;
    },
    check_for_updates: async () => {
      await wait(600);
      const available = update.channel === 'dev' ? '0.1.0-dev.4' : null;
      const fresh = available !== update.available;
      update = { ...update, lastChecked: nowSec(), available, ready: available ? !fresh && !!update.ready : false };
      emit('update://status', update);
      if (available && !update.ready) {
        setTimeout(() => {
          update = { ...update, ready: true };
          emit('update://status', update);
        }, 1200);
      }
      return update;
    },
    install_update: async () => {
      await wait(600);
      return null;
    },
    set_fullscreen: () => null,
    set_mini_player: () => null,
    media_server_status: () => ({ plex: { ...plex } }),
    plex_sign_in_start: () => {
      plexPinChecks = 0;
      return { id: 1, code: 'DEMO', authUrl: 'https://app.plex.tv/auth#?code=DEMO' };
    },
    plex_sign_in_check: async () => {
      await wait(300);
      if (++plexPinChecks < 1) return false;
      plex = { ...plex, signedIn: true, accountName: 'demo-user' };
      return true;
    },
    plex_servers: () => [
      { id: 'srv-home', name: 'Home server', owned: true },
      { id: 'srv-friend', name: 'Friend’s server', owned: false },
    ],
    plex_select_server: async ({ serverId }) => {
      await wait(400);
      plex = { ...plex, serverId, serverName: serverId === 'srv-home' ? 'Home server' : 'Friend’s server' };
      return { plex: { ...plex } };
    },
    media_server_sign_out: () => {
      plex = { signedIn: false, accountName: null, serverId: null, serverName: null };
      return null;
    },
    // Synthetic: VOD and recordings with an even id are "in the library".
    media_server_match: ({ target }) => {
      if (!plex.serverId) return null;
      const hit = target.kind === 'programme' ? target.id.length % 2 === 0 : target.id % 2 === 0;
      return hit
        ? { server: 'plex' as const, serverName: plex.serverName ?? 'Plex', kind: target.kind === 'vod' ? 'movie' as const : 'show' as const, title: 'Demo title', url: 'https://app.plex.tv/desktop' }
        : null;
    },
    media_server_open: () => null,
    media_details: async ({ target }) => {
      await wait(250);
      if (target.kind === 'programme') {
        const [cid] = target.id.split(':').map(Number);
        const p = genProgrammes(cid ?? 0, 0, 4e9, scheduled).find((x) => x.id === target.id);
        if (!p) throw appError('local', 'That programme is no longer in the guide.');
        return mockDetails({ kind: 'programme', title: p.title ?? 'Programme', episodeTitle: p.subtitle, season: p.season, episode: p.episode, description: p.description, isNew: p.isNew, isLive: p.isLive, isPremiere: p.isPremiere, isFinale: p.isFinale, runtimeMins: Math.round((p.endUtc - p.startUtc) / 60) }, tmdbKey);
      }
      if (target.kind === 'recording') {
        const r = vod.getRecording(target.id);
        if (!r) throw appError('local', 'That recording is gone.');
        return mockDetails({ kind: 'recording', title: r.title, episodeTitle: r.subtitle, season: r.season, episode: r.episode, description: r.description, runtimeMins: Math.round((r.endUtc - r.startUtc) / 60) }, tmdbKey);
      }
      return mockDetails({ kind: 'movie', title: 'Demo title', description: 'Synthetic plot for a demo title.' }, tmdbKey);
    },
    tmdb_status: () => ({ configured: tmdbKey, language: 'en-US' }),
    tmdb_set_key: async ({ key }) => {
      await wait(500);
      if (key.trim().length < 8) throw appError('auth', 'TMDB rejected the API key.');
      tmdbKey = true;
      return { configured: true, language: 'en-US' };
    },
    tmdb_clear_key: () => {
      tmdbKey = false;
      return { configured: false, language: 'en-US' };
    },
  };

  // `?mock=update`: a newer version is downloaded and waiting (rail dot, Settings → Updates card).
  if (mode === 'update') {
    update = {
      ...update,
      available: '0.1.0',
      notes: "## What's new\n- Faster guide scrolling\n- Restart and update from Settings",
      lastChecked: nowSec(),
      ready: true,
    };
  }

  // `?mock=resume`: the shell resumes the last channel at launch (setting on, playback starts by itself).
  if (mode === 'resume') {
    settings = { ...settings, resumeLastChannel: true };
    setTimeout(() => void impl.play_channel({ channelId: channels[1]?.id ?? 100 }), 400);
  }

  return {
    async invoke(cmd, args) {
      // Test hook: Playwright reads this to assert what the UI did (e.g. no play_* during guide dwell).
      (((window as unknown as { __mockCalls?: string[] }).__mockCalls) ??= []).push(cmd);
      return (await (impl[cmd] as (a: unknown) => Promise<never>)(args)) as never;
    },
    async listen(event, handler): Promise<Unlisten> {
      const set = handlers.get(event) ?? new Set();
      set.add(handler as (p: never) => void);
      handlers.set(event, set);
      return () => set.delete(handler as (p: never) => void);
    },
  };
}
