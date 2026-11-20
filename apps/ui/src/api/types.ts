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
  /** The account may switch a live channel to another provider stream (stream picker). */
  StreamSwitch: 1 << 9,
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
  streamSwitch: boolean;
}

/** `connecting` = not probed yet since launch. */
export type SourceStatus = 'connecting' | 'connected' | 'syncing' | 'offline' | 'stale' | 'auth_error';

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
  /** Longest catch-up window across the source's channels, in days (0 = none). */
  archiveDays: number;
  /** The source can serve live TV over HLS (opt-in transport). Absent until the core reports it: hide the switch. */
  supportsHlsTransport?: boolean;
  /** Live TV uses HLS for this source. */
  hlsTransport?: boolean;
  /** Concurrent streams the account may hold; null/absent when unknown or unlimited. */
  streamLimit?: number | null;
}

export interface Group {
  id: number;
  sourceId: number;
  name: string;
  /** Channels in the group (computed by Rust). */
  count: number;
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
  /** Ready-to-use `dlimg://<hash>/<size>` URL (Tauri custom protocol, PLAN §3.6) or null. */
  logo: string | null;
  quality: 'SD' | 'HD' | 'FHD' | '4K' | null;
  isCatchup: boolean;
  catchupDays: number;
  isRadio: boolean;
  hasDrm: boolean;
  isFavourite: boolean;
  epgMatch: EpgMatch;
}

export interface Programme {
  /** Opaque string id (dummy EPG ids are synthetic). Used by play_catchup / schedule_recording. */
  id: string;
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
  /** Programme being played (live: the current one; catch-up: the archived one). */
  programme: Programme | null;
  /** Title of VOD / recording media being played (null for live and catch-up, which use `programme`). */
  title: string | null;
  /** A saved resume point (seconds) the UI may offer; null when none. */
  resumeAt: number | null;
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
  /** True during an automatic retry after the stream dropped (UI shows "Reconnecting…"). */
  reconnecting: boolean;
  /** Set when Buffering lasts > 3 s: what we are waiting on. */
  waitingOn: 'connection' | 'first_data' | 'first_keyframe' | null;
  error: AppError | null;
  /** UTC ms of the last transition (debug panel). */
  transitionedAt: number;
  /** Audio and subtitle tracks of the current file (teletext / DVB subtitles are `sub`). */
  tracks: Track[];
}

export interface Track {
  id: number;
  kind: 'audio' | 'sub';
  lang: string | null;
  title: string | null;
  codec: string | null;
  selected: boolean;
}

/** One provider stream of a channel, in failover order. */
export interface AltStream {
  /** 0-based failover position. */
  index: number;
  streamId: string;
  name: string;
  /** Provider (M3U/XC account) name, when the server shares it. */
  provider: string | null;
  /** `WIDTHxHEIGHT` as probed by the server. */
  resolution: string | null;
  fps: number | null;
  videoCodec: string | null;
  audioCodec: string | null;
  audioChannels: string | null;
  /** kbit/s. */
  bitrateKbps: number | null;
  viewers: number | null;
  /** The provider stopped listing this stream. */
  stale: boolean;
}

/** One multiview tile. Slot 0 is the main player (its playback state is `PlayerState`). */
export interface MultiviewTile {
  slot: number;
  channelId: number;
  state: PlayerPhase;
  failure: 'limit' | 'codec' | 'engine' | 'stream' | null;
}

export interface MultiviewState {
  tiles: MultiviewTile[];
  /** Audible slot (0 = main player). */
  audible: number;
  /** Tiles this machine may show, main player included (1 = multiview unavailable). */
  maxTiles: number;
  /** Concurrent streams the account may hold, when known. */
  streamLimit: number | null;
}

export interface StreamList {
  channelId: number;
  /** 1-based stream believed to be in use, when known. */
  sourceIndex: number | null;
  sourceCount: number;
  streams: AltStream[];
}

export type ErrorCode = 'auth' | 'network' | 'server' | 'stream' | 'media' | 'local';

export interface AppError {
  code: ErrorCode;
  /** Already redacted, human readable. The UI renders it verbatim. */
  message: string;
  /** Seconds. */
  retryAfter: number | null;
  /** More specific cause for code 'stream'/'media' (PLAN §4.9): server stream limit, undecodable codec, engine crash. */
  reason?: 'limit' | 'codec' | 'engine';
  /** Codec name when reason is 'codec'. */
  codec?: string;
  /** Crash id when reason is 'engine'. */
  crashId?: string;
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
  /** Absolute position in seconds (catch-up / VOD). */
  | { kind: 'seek'; seconds: number }
  | { kind: 'goLive' }
  | { kind: 'setVolume'; volume: number }
  | { kind: 'setMuted'; muted: boolean }
  | { kind: 'setAudioTrack'; id: number }
  /** `null` turns subtitles off. */
  | { kind: 'setSubtitleTrack'; id: number | null }
  | { kind: 'retry' };

export type DensitySetting = 'compact' | 'comfortable' | 'tv';

export type HwdecSetting = 'auto-safe' | 'auto' | 'no' | 'vaapi' | 'nvdec' | 'd3d11va' | 'videotoolbox';

/** Machine-local playback/diagnostic settings owned by Rust. */
export type AudioOutput = 'stereo' | 'surround' | 'passthrough';
export type Deinterlace = 'auto' | 'yes' | 'no';

export interface AppSettings {
  /** 1..10 seconds. */
  liveBufferSecs: number;
  hwdec: HwdecSetting;
  debugLogging: boolean;
  /** Null = system default. */
  audioDevice: string | null;
  audioOutput: AudioOutput;
  deinterlace: Deinterlace;
  /** Percent, 50..150. */
  subtitleSize: number;
  subtitleBackground: boolean;
  /** Open the player on the most recently watched channel at launch (off by default). */
  resumeLastChannel: boolean;
}

export interface AudioDevice {
  id: string;
  name: string;
}

/** `session://stats`, 1 Hz, only while the info panel is open (`set_stats_enabled`). */
export interface SessionStats {
  /** e.g. "1920x1080". */
  resolution: string | null;
  videoCodec: string | null;
  audioCodec: string | null;
  bitrateKbps: number | null;
  droppedFrames: number | null;
  bufferSecs: number | null;
  deinterlace: string | null;
  hwdecCurrent: string | null;
  /** 1-based index of the stream in use and how many the channel has. */
  sourceIndex: number | null;
  sourceCount: number | null;
}

export interface Counts {
  all: number;
  favourites: number;
}

export type UpdateChannel = 'stable' | 'dev';

export interface UpdateStatus {
  channel: UpdateChannel;
  current: string;
  /** Newer version, if any. */
  available?: string | null;
  /** UTC seconds. */
  lastChecked?: number | null;
}

export interface AddSourceInput {
  kind: SourceKind;
  name: string;
  url: string;
  /** Dispatcharr. Goes to the OS keychain in Rust; never echoed back. */
  apiKey?: string;
  /** Dispatcharr (alternative) / Xtream. */
  username?: string;
  password?: string;
  /** M3U: optional XMLTV guide URL. */
  xmltvUrl?: string;
  allowSelfSigned: boolean;
}

export interface TestSourceResult {
  channels: number;
  groups: number;
  programmes: number;
  serverVersion: string | null;
  /** Authenticated user, when the server reports one. */
  user?: string | null;
}

export interface Transition {
  /** UTC ms. */
  at: number;
  from: PlayerPhase;
  to: PlayerPhase;
}

export interface Diagnostics {
  appVersion: string;
  serverVersions: string[];
  playerState: PlayerState;
  hwdecCurrent: string | null;
  os: string;
  /** Last 50. */
  transitions: Transition[];
  /** Already redacted by Rust. */
  logTail: string[];
}

export interface TaskProgress {
  sourceId: number;
  phase: string;
  done: number;
  total: number;
}

/** Video rect in PHYSICAL pixels, relative to the window's client area. */
export interface VideoBounds {
  x: number;
  y: number;
  w: number;
  h: number;
  /** devicePixelRatio. */
  scale: number;
}

// ---- Guide (PLAN §8.2) ----
export interface GuideArgs {
  /** UTC seconds. */
  start: number;
  end: number;
  channelOffset: number;
  channelLimit: number;
  groupId?: number | null;
}

export interface GuideResult {
  /** The channels in [channelOffset, channelOffset + channelLimit) of the (group-filtered) list. */
  channels: Channel[];
  /** Programmes overlapping [start, end) for those channels. */
  programmes: Programme[];
}

export type RecordKind = 'once' | 'series';

// ---- Home / search ----
export interface ContinueItem {
  kind: 'live' | 'catchup' | 'movie' | 'episode' | 'recording';
  id: string;
  channel: Channel | null;
  programme: Programme | null;
  title: string;
  subtitle: string | null;
  /** Artwork URL (`dlimg://`) or null. */
  art: string | null;
  /** 0..1, null for live. */
  progress: number | null;
  remainingSecs: number | null;
}

export interface OnNowItem {
  channel: Channel;
  now: Programme | null;
}

export interface HomeData {
  continueWatching: ContinueItem[];
  favourites: Channel[];
  /** Current programme per favourite. */
  onNow: OnNowItem[];
  recents: Channel[];
}

export interface ProgrammeHit {
  programme: Programme;
  channel: Channel;
}

export interface SearchResult {
  channels: Channel[];
  programmes: ProgrammeHit[];
}

// ---- VOD ----
export type VodKind = 'movie' | 'series';

export interface VodCategory {
  id: number;
  name: string;
  count: number;
}

export interface VodListArgs {
  categoryId?: number | null;
  query?: string | null;
  offset?: number;
  limit?: number;
}

export interface Movie {
  id: number;
  title: string;
  year: number | null;
  /** 0..10 or null. */
  rating: number | null;
  durationSecs: number | null;
  /** `dlimg://` URL or null (UI draws a deterministic fallback). */
  poster: string | null;
  backdrop: string | null;
  plot: string | null;
  genres: string[];
  categoryId: number | null;
  /** Saved position in seconds, null when never watched. */
  resumeSecs: number | null;
  watched: boolean;
}

export interface Series {
  id: number;
  title: string;
  year: number | null;
  rating: number | null;
  poster: string | null;
  backdrop: string | null;
  plot: string | null;
  genres: string[];
  categoryId: number | null;
  seasonCount: number;
}

export interface Episode {
  id: number;
  number: number;
  title: string;
  plot: string | null;
  durationSecs: number | null;
  still: string | null;
  resumeSecs: number | null;
  watched: boolean;
}

export interface Season {
  number: number;
  episodes: Episode[];
}

export interface SeriesDetail {
  series: Series;
  seasons: Season[];
  /** Episode to continue with (resume or next unwatched), null when none. */
  nextEpisodeId: number | null;
}

export interface PlayVodArgs {
  kind: 'movie' | 'episode';
  id: number;
  /** Seek to the saved position. */
  resume?: boolean;
}

// ---- DVR ----
export type RecordingStatus = 'scheduled' | 'recording' | 'completed' | 'failed';

export interface Recording {
  id: number;
  title: string;
  channelId: number | null;
  channelName: string;
  startUtc: number;
  endUtc: number;
  status: RecordingStatus;
  ruleId: number | null;
  /** Human label of the rule that created it (e.g. "Series"), null for one-offs. */
  ruleLabel: string | null;
  sizeBytes: number | null;
  /** Failure reason (one sentence) when status is failed. */
  error: string | null;
}

export interface RecordingsData {
  upcoming: Recording[];
  inProgress: Recording[];
  completed: Recording[];
}

export interface RecordingRule {
  id: number;
  title: string;
  channelName: string;
  kind: 'series' | 'recurring';
  /** e.g. "New episodes" or "Weekdays 19:00". */
  schedule: string;
}

// ---- Deep links (app://deeplink) ----
export type DeepLink =
  | { kind: 'channel'; uuid: string }
  /** `channel` is a channel uuid; `programme` an opaque programme id (catch-up); `start` is informational. */
  | { kind: 'play'; channel: string; programme?: string; start?: number };
