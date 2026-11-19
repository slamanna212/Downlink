// Mock backend for browser dev and Storybook. SYNTHETIC data only (CLAUDE rule 16): no real provider
// names, URLs or EPG. `?mock=empty` in the URL simulates a fresh install with no source.
import type { Backend, CommandName, Commands, EventName, Events, Unlisten } from './ipc';
import { Cap, type Channel, type Group, type NowNext, type PlayerState, type Programme, type Source } from './types';

const NAMES = [
  'Harbor News 24', 'Meridian One', 'Cobalt Sports', 'Lantern Cinema', 'Orchard Kids', 'Northwind Docs',
  'Summit Music', 'Ember Drama', 'Tidepool Nature', 'Quartz Weather', 'Foxglove Comedy', 'Atlas World',
  'Juniper Home', 'Relay Racing', 'Saffron Kitchen', 'Vesper Late', 'Kestrel Science', 'Paper Moon Classics',
];
const GROUP_NAMES = ['News', 'Sports', 'Movies', 'Kids', 'Documentary', 'Music'];
const QUALITIES: Channel['quality'][] = ['HD', 'FHD', '4K', 'SD'];

export function mockSources(empty: boolean): Source[] {
  if (empty) return [];
  return [
    {
      id: 1, uuid: 'mock-source-1', kind: 'dispatcharr', name: 'Demo server', host: 'demo.invalid',
      caps: Cap.Live | Cap.Epg | Cap.Catchup | Cap.Vod | Cap.Dvr | Cap.Favourites,
      status: 'connected', serverVersion: '0.0-mock', lastSyncAt: Math.floor(Date.now() / 1000) - 600,
    },
  ];
}

export function mockGroups(): Group[] {
  return GROUP_NAMES.map((name, i) => ({ id: i + 1, sourceId: 1, name, channelCount: 0 }));
}

export function mockChannels(count = NAMES.length): Channel[] {
  return Array.from({ length: count }, (_, i) => {
    const catchup = i % 3 === 0;
    return {
      id: 100 + i, sourceId: 1, uuid: `mock-channel-${i}`, number: i + 1 + (i === 4 ? 0.1 : 0),
      name: NAMES[i % NAMES.length] ?? `Channel ${i}`, groupId: (i % GROUP_NAMES.length) + 1,
      logoKey: i % 4 === 1 ? null : `mock-logo-${i}`,
      quality: QUALITIES[i % QUALITIES.length] ?? null,
      isCatchup: catchup, catchupDays: catchup ? 7 : 0, isRadio: i === 6, hasDrm: i === 11,
      isFavourite: i % 5 === 0, epgMatch: 'id',
    } satisfies Channel;
  });
}

export function mockNowNext(channelId: number, now = Math.floor(Date.now() / 1000)): NowNext {
  const slot = 3600;
  const start = now - (now % slot);
  const mk = (s: number, title: string): Programme => ({
    channelId, startUtc: s, endUtc: s + slot, title, subtitle: null, description: 'Synthetic programme.',
    season: null, episode: null, isNew: false, isLive: false, isPremiere: false, isFinale: false, isScheduled: false,
  });
  return { channelId, now: mk(start, `Evening feature ${channelId % 7}`), next: mk(start + slot, `Late show ${channelId % 5}`) };
}

export function idleState(): PlayerState {
  return {
    state: 'Idle', mode: null, channelId: null, position: null, duration: null, buffered: null, seekable: false,
    volume: 100, muted: false, hwdecCurrent: null, videoCodec: null, width: null, height: null, waitingOn: null,
    error: null, transitionedAt: Date.now(),
  };
}

export function createMockBackend(): Backend {
  const empty = typeof location !== 'undefined' && new URLSearchParams(location.search).get('mock') === 'empty';
  const sources = mockSources(empty);
  const channels = empty ? [] : mockChannels();
  const groups = mockGroups().map((g) => ({ ...g, channelCount: channels.filter((c) => c.groupId === g.id).length }));
  let player = idleState();
  const handlers = new Map<EventName, Set<(p: never) => void>>();

  const emit = <E extends EventName>(event: E, payload: Events[E]) =>
    handlers.get(event)?.forEach((h) => h(payload as never));
  const setPlayer = (patch: Partial<PlayerState>) => {
    player = { ...player, ...patch, transitionedAt: Date.now() };
    emit('player://state', player);
  };

  const impl: { [C in CommandName]: (a: Commands[C]['args']) => Commands[C]['result'] } = {
    list_sources: () => sources,
    list_groups: () => groups,
    list_channels: ({ groupId, query, favouritesOnly }) =>
      channels.filter(
        (c) =>
          (groupId == null || c.groupId === groupId) &&
          (!favouritesOnly || c.isFavourite) &&
          (!query || c.name.toLowerCase().includes(query.toLowerCase())),
      ),
    get_now_next: ({ channelIds }) => channelIds.map((id) => mockNowNext(id)),
    get_player_state: () => player,
    play_channel: ({ channelId }) => {
      setPlayer({ state: 'Resolving', mode: 'live', channelId, error: null });
      setTimeout(() => setPlayer({ state: 'Connecting' }), 150);
      setTimeout(() => setPlayer({ state: 'Playing', hwdecCurrent: 'vaapi', videoCodec: 'h264', width: 1920, height: 1080 }), 400);
      return null;
    },
    player_command: ({ command }) => {
      if (command.kind === 'pause') setPlayer({ state: 'Paused' });
      else if (command.kind === 'resume') setPlayer({ state: 'Playing' });
      else if (command.kind === 'stop') setPlayer({ ...idleState(), state: 'Stopped' });
      else if (command.kind === 'setVolume') setPlayer({ volume: command.volume });
      else if (command.kind === 'setMuted') setPlayer({ muted: command.muted });
      return null;
    },
    set_favourite: ({ channelId, favourite }) => {
      const c = channels.find((x) => x.id === channelId);
      if (c) c.isFavourite = favourite;
      return null;
    },
  };

  return {
    async invoke(cmd, args) {
      return (impl[cmd] as (a: unknown) => never)(args);
    },
    async listen(event, handler): Promise<Unlisten> {
      const set = handlers.get(event) ?? new Set();
      set.add(handler as (p: never) => void);
      handlers.set(event, set);
      return () => set.delete(handler as (p: never) => void);
    },
  };
}
