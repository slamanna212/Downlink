// The ONLY place the UI touches Tauri. Typed wrapper over invoke/listen (CLAUDE rule 2: the UI never
// talks HTTP). Outside Tauri (browser dev, Storybook, tests) a mock backend with synthetic data answers.
import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import { noteBlockingError } from '../lib/blocking';
import type {
  AddSourceInput,
  AppError,
  AppSettings,
  AudioDevice,
  GuideArgs,
  GuideResult,
  HomeData,
  Programme,
  RecordKind,
  VodCategory,
  VodListArgs,
  Movie,
  Series,
  SeriesDetail,
  PlayVodArgs,
  RecordingsData,
  RecordingRule,
  SearchResult,
  Counts,
  SessionStats,
  UpdateChannel,
  UpdateStatus,
  ReleaseNotes,
  CatalogUpdated,
  DeepLink,
  Diagnostics,
  TaskProgress,
  TestSourceResult,
  VideoBounds,
  Channel,
  Group,
  NowNext,
  PlayerCommand,
  PlayerState,
  Source,
  StreamList,
  MultiviewState,
  MediaMatch,
  MediaServerStatus,
  MediaTarget,
  PlexPin,
  PlexServer,
} from './types';

/** Command name -> args/result. Keep in sync with the Rust `#[tauri::command]` set (PLAN §2.5). */
export interface Commands {
  list_sources: { args: Record<string, never>; result: Source[] };
  list_groups: { args: { sourceId?: number | null }; result: Group[] };
  list_channels: {
    args: { groupId?: number | null; query?: string | null; favouritesOnly?: boolean; offset?: number; limit?: number };
    result: Channel[];
  };
  find_channel_number: { args: { number: number }; result: number | null };
  get_channel: { args: { channelId: number }; result: Channel };
  get_programme_window: { args: { channelIds: number[]; start: number; end: number }; result: Programme[] };
  get_now_next: { args: { channelIds: number[] }; result: NowNext[] };
  get_player_state: { args: Record<string, never>; result: PlayerState };
  play_channel: { args: { channelId: number }; result: null };
  player_command: { args: { command: PlayerCommand }; result: null };
  set_favourite: { args: { channelId: number; favourite: boolean }; result: null };
  add_source: { args: AddSourceInput; result: Source };
  test_source: { args: AddSourceInput; result: TestSourceResult };
  remove_source: { args: { sourceId: number }; result: null };
  sync_source: { args: { sourceId: number }; result: null };
  get_diagnostics: { args: Record<string, never>; result: Diagnostics };
  set_debug_logging: { args: { enabled: boolean }; result: null };
  get_settings: { args: Record<string, never>; result: AppSettings };
  set_settings: { args: { patch: Partial<AppSettings> }; result: AppSettings };
  set_video_bounds: { args: VideoBounds; result: null };
  set_fullscreen: { args: { enabled: boolean }; result: null };
  set_mini_player: { args: { enabled: boolean }; result: null };
  get_guide: { args: GuideArgs; result: GuideResult };
  get_programmes: { args: { channelId: number; start: number; end: number }; result: Programme[] };
  play_catchup: { args: { programmeId: string; resume?: boolean }; result: null };
  restart_programme: { args: { channelId: number }; result: null };
  schedule_recording: { args: { programmeId: string; kind: RecordKind }; result: null };
  get_home: { args: Record<string, never>; result: HomeData };
  search: { args: { query: string; limit: number }; result: SearchResult };
  set_stats_enabled: { args: { enabled: boolean }; result: null };
  get_counts: { args: Record<string, never>; result: Counts };
  list_audio_devices: { args: Record<string, never>; result: AudioDevice[] };
  get_channel_streams: { args: { channelId: number }; result: StreamList };
  switch_stream: { args: { channelId: number; index: number }; result: null };
  get_multiview: { args: Record<string, never>; result: MultiviewState };
  multiview_set_tile: { args: { slot: number; channelId: number }; result: null };
  multiview_remove_tile: { args: { slot: number }; result: null };
  multiview_set_audio: { args: { slot: number }; result: null };
  multiview_make_main: { args: { slot: number }; result: null };
  multiview_set_bounds: { args: VideoBounds & { slot: number }; result: null };
  multiview_exit: { args: Record<string, never>; result: null };
  get_update_status: { args: Record<string, never>; result: UpdateStatus };
  set_update_channel: { args: { channel: UpdateChannel }; result: UpdateStatus };
  check_for_updates: { args: Record<string, never>; result: UpdateStatus };
  install_update: { args: Record<string, never>; result: null };
  list_vod_categories: { args: { kind: 'movie' | 'series'; withCounts?: boolean }; result: VodCategory[] };
  list_movies: { args: VodListArgs & { compact?: boolean }; result: Movie[] };
  get_movie: { args: { id: number }; result: Movie };
  list_series: { args: VodListArgs & { compact?: boolean }; result: Series[] };
  get_series: { args: { id: number }; result: SeriesDetail };
  play_vod: { args: PlayVodArgs; result: null };
  list_recordings: { args: Record<string, never>; result: RecordingsData };
  cancel_recording: { args: { id: number }; result: null };
  stop_recording: { args: { id: number }; result: null };
  extend_recording: { args: { id: number; minutes: number }; result: null };
  delete_recording: { args: { id: number }; result: null };
  list_recording_rules: { args: Record<string, never>; result: RecordingRule[] };
  delete_recording_rule: { args: { id: number }; result: null };
  play_recording: { args: { id: number }; result: null };
  get_channel_by_uuid: { args: { uuid: string }; result: Channel | null };
  export_diagnostics: { args: Record<string, never>; result: string };
  reveal_path: { args: { path: string }; result: null };
  /** Debug: the UI reports a named startup timing (ms since navigation start) for the desktop perf harness. */
  /** Move the newer-version database aside and start with an empty library (the UI then reloads). */
  archive_newer_database: { args: Record<string, never>; result: null };
  /** Accept the server's new TLS certificate for this source. */
  trust_new_certificate: { args: { sourceId: number }; result: null };
  quit_app: { args: Record<string, never>; result: null };
  set_source_options: { args: { sourceId: number; hlsTransport: boolean }; result: null };
  report_ui_timing: { args: { name: string; ms: number }; result: null };
  media_server_status: { args: Record<string, never>; result: MediaServerStatus };
  /** Starts Plex sign-in and opens the approval page in the browser. */
  plex_sign_in_start: { args: Record<string, never>; result: PlexPin };
  /** True once the user approved the sign-in. */
  plex_sign_in_check: { args: { pinId: number }; result: boolean };
  plex_servers: { args: Record<string, never>; result: PlexServer[] };
  plex_select_server: { args: { serverId: string }; result: MediaServerStatus };
  media_server_sign_out: { args: Record<string, never>; result: null };
  media_server_match: { args: { target: MediaTarget }; result: MediaMatch | null };
  /** Opens a page from `media_server_match` in the browser. */
  media_server_open: { args: { url: string }; result: null };
  /** GitHub release body for a version (Markdown), or the latest.json notes if GitHub is unreachable. */
  get_release_notes: { args: { version: string }; result: ReleaseNotes };
  /** Opens a web link from release notes in the system browser. */
  open_external_link: { args: { url: string }; result: null };
}
export type CommandName = keyof Commands;

/** Rust -> UI events (PLAN §2.5). */
export interface Events {
  'catalog://updated': CatalogUpdated;
  'player://state': PlayerState;
  'task://progress': TaskProgress;
  'session://stats': SessionStats;
  'update://status': UpdateStatus;
  'app://deeplink': DeepLink;
  'multiview://state': MultiviewState;
  /** A tile's playback state changed (payload: slot); refetch `get_multiview`. */
  'multiview://tile': number;
}
export type EventName = keyof Events;

export type Unlisten = () => void;

/** Backend contract; implemented by Tauri and by the mock. */
export interface Backend {
  invoke<C extends CommandName>(cmd: C, args: Commands[C]['args']): Promise<Commands[C]['result']>;
  listen<E extends EventName>(event: E, handler: (payload: Events[E]) => void): Promise<Unlisten>;
}

const KNOWN_CODES = ['auth', 'network', 'server', 'stream', 'media', 'local'];

export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'code' in e &&
    'message' in e &&
    typeof (e as AppError).message === 'string' &&
    KNOWN_CODES.includes(String((e as AppError).code))
  );
}

/** Normalise anything thrown by invoke into an AppError. Never leaks raw exception text. */
export function toAppError(e: unknown): AppError {
  if (isAppError(e)) return { code: e.code, message: e.message, retryAfter: e.retryAfter ?? null, reason: e.reason, codec: e.codec, crashId: e.crashId };
  return { code: 'local', message: 'Something went wrong. Try again.', retryAfter: null };
}

const tauriBackend: Backend = {
  async invoke(cmd, args) {
    try {
      return (await tauriInvoke(cmd, args as Record<string, unknown>)) as never;
    } catch (e) {
      throw toAppError(e);
    }
  },
  async listen(event, handler) {
    return tauriListen(event, (e) => handler(e.payload as never));
  },
};

let backend: Promise<Backend> | null = null;

function getBackend(): Promise<Backend> {
  // Cache the promise: concurrent first calls must share ONE backend (the mock is stateful).
  backend ??= isTauri()
    ? Promise.resolve(tauriBackend)
    : import('./ipc.mock').then((m) => m.createMockBackend());
  return backend;
}

/** Test/Storybook hook: replace the backend. */
export function setBackend(b: Backend | null): void {
  backend = b ? Promise.resolve(b) : null;
}

export async function invoke<C extends CommandName>(cmd: C, args: Commands[C]['args']): Promise<Commands[C]['result']> {
  try {
    return await (await getBackend()).invoke(cmd, args);
  } catch (e) {
    if (isAppError(e)) noteBlockingError(e.message);
    throw e;
  }
}

export async function listen<E extends EventName>(event: E, handler: (payload: Events[E]) => void): Promise<Unlisten> {
  return (await getBackend()).listen(event, handler);
}

export const inTauri = (): boolean => isTauri();
