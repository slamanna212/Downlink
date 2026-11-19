// DTOs crossing the Tauri IPC boundary. Mirrors docs/PLAN.md §2.5 (commands/events), §3.3 (channel
// shape), §4.4 (player state machine) and §7.1 (schema). All timestamps are UTC epoch SECONDS.
// The UI never builds these itself outside ipc.mock.ts; Rust owns the real shapes (serde camelCase).

export type SourceKind = 'dispatcharr' | 'xtream' | 'm3u'; // display/diagnostics only: never branch on it (CLAUDE rule 9)

/** Capability bitflags, OR-ed across sources. The UI branches on these, never on source kind. */
export const Cap = {
  Live: 1 << 0,
  Epg: 1 << 1,
  Catchup: 1 << 2,
  Vod: 1 << 3,
  Dvr: 1 << 4,
  Favourites: 1 << 5,
} as const;
export type CapFlag = (typeof Cap)[keyof typeof Cap];

/** Decoded capabilities. */
export interface Caps {
  live: boolean;
  epg: boolean;
  catchup: boolean;
  vod: boolean;
  dvr: boolean;
  favourites: boolean;
}

export type SourceStatus = 'connected' | 'syncing' | 'offline' | 'stale' | 'auth_error';

export interface Source {
  id: number;
  uuid: string;
  kind: SourceKind;
  name: string;
  /** Redacted host only; never a URL carrying credentials. */
  host: string | null;
  /** Bitflags, see `Cap`. */
  caps: number;
  status: SourceStatus;
  serverVersion: string | null;
  lastSyncAt: number | null;
}

export interface Group {
  id: number;
  sourceId: number;
  name: string;
  channelCount: number;
}

export type EpgMatch = 'id' | 'name' | 'display' | 'manual' | 'none';

export interface Channel {
  id: number;
  sourceId: number;
  uuid: string | null;
  /** Float: 5.1 style numbers exist. */
  number: number | null;
  name: string;
  groupId: number | null;
  /** Opaque key resolved by the image command; never a URL. */
  logoKey: string | null;
  quality: 'SD' | 'HD' | 'FHD' | '4K' | null;
  isCatchup: boolean;
  catchupDays: number;
  isRadio: boolean;
  hasDrm: boolean;
  isFavourite: boolean;
  epgMatch: EpgMatch;
}

export interface Programme {
  channelId: number;
  startUtc: number;
  endUtc: number;
  title: string | null;
  subtitle: string | null;
  description: string | null;
  season: number | null;
  episode: number | null;
  isNew: boolean;
  isLive: boolean;
  isPremiere: boolean;
  isFinale: boolean;
  /** Scheduled for recording. */
  isScheduled: boolean;
}

export interface NowNext {
  channelId: number;
  now: Programme | null;
  next: Programme | null;
}

export type PlayerPhase =
  | 'Idle'
  | 'Resolving'
  | 'Connecting'
  | 'Buffering'
  | 'Playing'
  | 'Paused'
  | 'Stopped'
  | 'Failed';

export type PlayerMode = 'live' | 'catchup' | 'vod' | 'recording';

export interface PlayerState {
  state: PlayerPhase;
  mode: PlayerMode | null;
  channelId: number | null;
  /** Seconds. Null while not seekable. */
  position: number | null;
  duration: number | null;
  buffered: number | null;
  seekable: boolean;
  volume: number;
  muted: boolean;
  hwdecCurrent: string | null;
  videoCodec: string | null;
  width: number | null;
  height: number | null;
  /** Set when Buffering lasts > 3 s: what we are waiting on. */
  waitingOn: 'connection' | 'first_data' | 'first_keyframe' | null;
  error: AppError | null;
  /** UTC ms of the last transition (debug panel). */
  transitionedAt: number;
}

export type ErrorCode = 'auth' | 'network' | 'server' | 'stream' | 'media' | 'local';

export interface AppError {
  code: ErrorCode;
  /** Already redacted, human readable. The UI renders it verbatim. */
  message: string;
  /** Seconds. */
  retryAfter: number | null;
}

export interface CatalogUpdated {
  sourceId: number;
  channels: number;
  groups: number;
  programmes: number;
}

export type PlayerCommand =
  | { kind: 'pause' }
  | { kind: 'resume' }
  | { kind: 'stop' }
  | { kind: 'seek'; seconds: number }
  | { kind: 'setVolume'; volume: number }
  | { kind: 'setMuted'; muted: boolean };

export type DensitySetting = 'compact' | 'comfortable' | 'tv';
