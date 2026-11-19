// Mock backend for browser dev and Storybook. SYNTHETIC data only (CLAUDE rule 16): no real provider
// names, URLs or EPG. `?mock=empty` in the URL simulates a fresh install with no source;
// `?mock=offline` simulates a source that is offline.
import type { Backend, CommandName, Commands, EventName, Events, Unlisten } from './ipc';
import {
  Cap,
  type AppError,
  type AppSettings,
  type Channel,
  type UpdateStatus,
  type Group,
  type NowNext,
  type PlayerState,
  type Programme,
  type Source,
  type Transition,
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

export function mockSources(empty: boolean, status: Source['status'] = 'connected'): Source[] {
  if (empty) return [];
  return [
    {
      id: 1, uuid: 'mock-source-1', kind: 'dispatcharr', name: 'Demo server', host: 'demo.invalid',
      caps: Cap.Live | Cap.Epg | Cap.Catchup | Cap.Vod | Cap.Dvr | Cap.Favourites,
      status, serverVersion: '0.0-mock', lastSyncAt: nowSec() - 600,
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

export function mockNowNext(channelId: number, now = nowSec()): NowNext {
  const slot = (30 + (channelId % 4) * 15) * 60;
  const offset = (channelId * 977) % slot;
  const start = now - offset;
  const mk = (s: number, title: string): Programme => ({
    channelId, startUtc: s, endUtc: s + slot, title, subtitle: null, description: 'Synthetic programme.',
    season: null, episode: null, isNew: false, isLive: false, isPremiere: false, isFinale: false, isScheduled: false,
  });
  return {
    channelId,
    now: mk(start, TITLES[channelId % TITLES.length] ?? 'Programme'),
    next: mk(start + slot, TITLES[(channelId + 3) % TITLES.length] ?? 'Programme'),
  };
}

export function idleState(): PlayerState {
  return {
    state: 'Idle', mode: null, channelId: null, position: null, duration: null, buffered: null, seekable: false,
    volume: 100, muted: false, hwdecCurrent: null, videoCodec: null, width: null, height: null, waitingOn: null,
    error: null, transitionedAt: Date.now(),
  };
}

const appError = (code: AppError['code'], message: string): AppError => ({ code, message, retryAfter: null });

export function createMockBackend(): Backend {
  const mode = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('mock') : null;
  const failParam = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('fail') : null;
  const failId = failParam ? Number(failParam) : null;
  const sources = mockSources(mode === 'empty', mode === 'offline' ? 'offline' : 'connected');
  const channels = mode === 'empty' ? [] : mockChannels();
  const groups = mockGroups();
  let settings: AppSettings = {
    liveBufferSecs: 2, hwdec: 'auto-safe', debugLogging: false, audioDevice: null, audioOutput: 'stereo',
    deinterlace: 'auto', subtitleSize: 100, subtitleBackground: true,
  };
  let update: UpdateStatus = { channel: 'stable', current: '0.0.0-mock', available: null, lastChecked: null };
  let statsTimer: ReturnType<typeof setInterval> | null = null;
  let player = idleState();
  const transitions: Transition[] = [];
  const handlers = new Map<EventName, Set<(p: never) => void>>();

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
    list_channels: ({ groupId, query, favouritesOnly }) =>
      channels.filter(
        (c) =>
          (groupId == null || c.groupId === groupId) &&
          (!favouritesOnly || c.isFavourite) &&
          (!query || c.name.toLowerCase().includes(query.toLowerCase()) || String(c.number ?? '').startsWith(query)),
      ),
    get_now_next: ({ channelIds }) => channelIds.map((id) => mockNowNext(id)),
    get_player_state: () => player,
    play_channel: ({ channelId }) => {
      setPlayer({ state: 'Resolving', mode: 'live', channelId, error: null });
      setTimeout(() => setPlayer({ state: 'Connecting' }), 150);
      setTimeout(() => {
        // `?fail=<id>` in the URL makes that channel fail, to exercise the error overlay.
        if (failId !== null && failId === channelId)
          setPlayer({ state: 'Failed', error: appError('stream', 'This channel stopped sending video. Try again, or pick another channel.') });
        else setPlayer({ state: 'Playing', hwdecCurrent: 'vaapi', videoCodec: 'h264', width: 1920, height: 1080 });
      }, 400);
      return null;
    },
    player_command: ({ command }) => {
      if (command.kind === 'pause') setPlayer({ state: 'Paused' });
      else if (command.kind === 'resume') setPlayer({ state: 'Playing' });
      else if (command.kind === 'stop') setPlayer({ ...idleState(), state: 'Stopped' });
      else if (command.kind === 'setVolume') setPlayer({ volume: command.volume });
      else if (command.kind === 'setMuted') setPlayer({ muted: command.muted });
      else if (command.kind === 'retry' && player.channelId != null) impl.play_channel({ channelId: player.channelId });
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
        status: 'connected', serverVersion: '0.0-mock', lastSyncAt: nowSec(),
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
            sourceIndex: 1, sourceCount: 3,
          });
        tick();
        statsTimer = setInterval(tick, 1000);
      }
      return null;
    },
    get_counts: () => ({ all: channels.length, favourites: channels.filter((c) => c.isFavourite).length }),
    list_audio_devices: () => [{ id: 'default', name: 'System default' }, { id: 'hdmi-1', name: 'Demo TV (HDMI)' }, { id: 'usb-1', name: 'Demo headset (USB)' }],
    get_update_status: () => update,
    set_update_channel: ({ channel }) => {
      update = { ...update, channel, available: null };
      emit('update://status', update);
      return update;
    },
    check_for_updates: async () => {
      await wait(600);
      update = { ...update, lastChecked: nowSec(), available: update.channel === 'dev' ? '0.1.0-dev.4' : null };
      emit('update://status', update);
      return update;
    },
    set_fullscreen: () => null,
  };

  return {
    async invoke(cmd, args) {
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
