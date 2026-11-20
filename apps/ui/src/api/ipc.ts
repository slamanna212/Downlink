// The ONLY place the UI touches Tauri. Typed wrapper over invoke/listen (CLAUDE rule 2: the UI never
// talks HTTP). Outside Tauri (browser dev, Storybook, tests) a mock backend with synthetic data answers.
import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
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
} from './types';

/** Command name -> args/result. Keep in sync with the Rust `#[tauri::command]` set (PLAN §2.5). */
export interface Commands {
  list_sources: { args: Record<string, never>; result: Source[] };
  list_groups: { args: { sourceId?: number | null }; result: Group[] };
  list_channels: {
    args: { groupId?: number | null; query?: string | null; favouritesOnly?: boolean; offset?: number; limit?: number };
    result: Channel[];
  };
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
  get_update_status: { args: Record<string, never>; result: UpdateStatus };
  set_update_channel: { args: { channel: UpdateChannel }; result: UpdateStatus };
  check_for_updates: { args: Record<string, never>; result: UpdateStatus };
  list_vod_categories: { args: { kind: 'movie' | 'series' }; result: VodCategory[] };
  list_movies: { args: VodListArgs; result: Movie[] };
  get_movie: { args: { id: number }; result: Movie };
  list_series: { args: VodListArgs; result: Series[] };
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
  report_ui_timing: { args: { name: string; ms: number }; result: null };
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
  if (isAppError(e)) return { code: e.code, message: e.message, retryAfter: e.retryAfter ?? null };
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
  return (await getBackend()).invoke(cmd, args);
}

export async function listen<E extends EventName>(event: E, handler: (payload: Events[E]) => void): Promise<Unlisten> {
  return (await getBackend()).listen(event, handler);
}

export const inTauri = (): boolean => isTauri();
