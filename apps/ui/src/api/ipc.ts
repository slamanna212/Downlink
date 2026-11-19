// The ONLY place the UI touches Tauri. Typed wrapper over invoke/listen (CLAUDE rule 2: the UI never
// talks HTTP). Outside Tauri (browser dev, Storybook, tests) a mock backend with synthetic data answers.
import { invoke as tauriInvoke, isTauri } from '@tauri-apps/api/core';
import { listen as tauriListen } from '@tauri-apps/api/event';
import type {
  AppError,
  CatalogUpdated,
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
}
export type CommandName = keyof Commands;

/** Rust -> UI events (PLAN §2.5). */
export interface Events {
  'catalog://updated': CatalogUpdated;
  'player://state': PlayerState;
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
      return await tauriInvoke(cmd, args);
    } catch (e) {
      throw toAppError(e);
    }
  },
  async listen(event, handler) {
    return tauriListen(event, (e) => handler(e.payload as never));
  },
};

let backend: Backend | null = null;

async function getBackend(): Promise<Backend> {
  if (backend) return backend;
  if (isTauri()) {
    backend = tauriBackend;
  } else {
    const { createMockBackend } = await import('./ipc.mock');
    backend = createMockBackend();
  }
  return backend;
}

/** Test/Storybook hook: replace the backend. */
export function setBackend(b: Backend | null): void {
  backend = b;
}

export async function invoke<C extends CommandName>(cmd: C, args: Commands[C]['args']): Promise<Commands[C]['result']> {
  return (await getBackend()).invoke(cmd, args);
}

export async function listen<E extends EventName>(event: E, handler: (payload: Events[E]) => void): Promise<Unlisten> {
  return (await getBackend()).listen(event, handler);
}

export const inTauri = (): boolean => isTauri();
