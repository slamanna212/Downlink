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
  /** VOD item / recording id being played (with `mode`); null for live and catch-up. */
  mediaId: number | null;
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
  /** Wall-clock timeline of a live or catch-up session; null for VOD and recordings. */
  timeline: Timeline | null;
}

/** Channel timeline in UTC seconds (fractional). Older than the buffer is the archive, when there is one. */
export interface Timeline {
  playheadUtc: number;
  /** Newest playable moment. */
  liveEdgeUtc: number;
  /** Oldest moment in the local timeshift buffer (live only). */
  bufferStartUtc: number | null;
  /** Oldest moment in the server archive (channels with catch-up only). */
  archiveStartUtc: number | null;
  behindLiveSecs: number;
  source: 'buffer' | 'archive' | 'live';
}

export interface Track {
  id: number;
  kind: 'audio' | 'sub';
  lang: string | null;
  title: string | null;
  codec: string | null;
  /** Audio layout such as "stereo" or "5.1"; audio tracks only. */
  channels: string | null;
  default: boolean;
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
  /** Source that requires a new certificate trust decision. */
  sourceId?: number;
  /** More specific cause for code 'stream'/'media' (PLAN §4.9): server stream limit, undecodable codec, engine crash. */
  reason?: 'limit' | 'codec' | 'engine';
  /** Codec name when reason is 'codec'. */
  codec?: string;
  /** Crash id when reason is 'engine'. */
  crashId?: string;
}

export interface CatalogUpdated {
  scope?: 'status' | 'channels' | 'guide' | 'recordings' | 'syncComplete' | 'sources';
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
  /** Wall-clock seek on a live / catch-up timeline (UTC seconds). */
  | { kind: 'seekTo'; utc: number }
  /** Relative seek on a live / catch-up timeline. */
  | { kind: 'seekBy'; seconds: number }
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
/** `auto`: HDR content is shown as HDR when the display and desktop support it; `off`: always SDR. */
export type HdrOutput = 'auto' | 'off';

export interface AppSettings {
  /** 1..10 seconds. */
  liveBufferSecs: number;
  /** Live timeshift buffer in minutes: 0 = off, 15..180. Missing in older settings; treat as 60. */
  timeshiftMinutes?: number;
  hwdec: HwdecSetting;
  debugLogging: boolean;
  /** Null = system default. */
  audioDevice: string | null;
  audioOutput: AudioOutput;
  deinterlace: Deinterlace;
  /** Missing in settings saved before it existed; treat as `auto`. */
  hdrOutput?: HdrOutput;
  /** NVIDIA RTX Video Super Resolution (single player only). Missing = off. */
  rtxSuperResolution?: boolean;
  /** NVIDIA RTX Video HDR, SDR shown as HDR (needs `hdrOutput` auto). Missing = off. */
  rtxHdr?: boolean;
  /** Percent, 50..150. */
  subtitleSize: number;
  subtitleBackground: boolean;
  /** Open the player on the most recently watched channel at launch (off by default). */
  resumeLastChannel: boolean;
  /** Hours between background refreshes of every source, 0 = off (0..24, default 4). */
  autoRefreshHours?: number;
  /** Background refreshes may run while something is playing (default on). */
  refreshDuringPlayback?: boolean;
}

/** Graphics features the player can use on this machine. */
export interface VideoCaps {
  /** An NVIDIA RTX card on Windows: RTX Super Resolution and RTX HDR can be turned on. */
  rtxVideo: boolean;
  rtxAdapter: string | null;
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
  fps: number | null;
  /** Source transfer is PQ or HLG; null until video params are known. */
  hdr: boolean | null;
  /** mpv transfer name, e.g. "pq", "hlg", "bt.1886". */
  transfer: string | null;
  /** HDR (source, or RTX HDR output) rendered to an SDR display; null unless HDR is shown. */
  toneMapped: boolean | null;
  /** RTX Super Resolution factor requested from the driver; null when not in use. Missing on older builds. */
  rtxUpscale?: number | null;
  /** RTX HDR was requested for this video. */
  rtxHdrRequested?: boolean;
  /** RTX HDR is turning this SDR video into HDR; null unless requested and known. */
  rtxHdrActive?: boolean | null;
  /** e.g. "bt.709", "bt.2020". */
  primaries: string | null;
  bitDepth: number | null;
  /** e.g. "stereo", "5.1". */
  audioChannels: string | null;
  audioSampleRate: number | null;
  /** Network read rate into the cache. */
  downloadKbps: number | null;
  avsyncMs: number | null;
  /** e.g. "mpegts", "hls". */
  container: string | null;
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
  /** Release notes (Markdown) of the available update. */
  notes?: string | null;
  /** UTC seconds. */
  lastChecked?: number | null;
  /** The available update is downloaded and can be installed now. */
  ready?: boolean;
}

export interface ReleaseNotes {
  version: string;
  /** Markdown; null when there is nothing to show. */
  body: string | null;
  /** `github` = the release body, `manifest` = the notes in latest.json. */
  source: 'github' | 'manifest';
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
  next: Programme | null;
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
  countPending?: boolean;
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
  /** Episode title, when the programme had one. */
  subtitle: string | null;
  description: string | null;
  season: number | null;
  episode: number | null;
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
  /** Channel logo URL, when the channel has one. */
  channelLogo: string | null;
  /** Guide programme id while the guide still has the airing (artwork, Record series). */
  programmeId: string | null;
  /** Saved watch position, when it is worth resuming. */
  positionSecs: number | null;
  /** Length for "time left": the player's duration, else the scheduled length. */
  durationSecs: number;
  /** Watched to (nearly) the end. */
  watched: boolean;
  /** Something to play: running, completed, or failed with a partial file. */
  playable: boolean;
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

// ---- media servers (Plex) ---------------------------------------------------------------------

export interface PlexStatus {
  signedIn: boolean;
  accountName: string | null;
  serverId: string | null;
  serverName: string | null;
}

export interface MediaServerStatus {
  plex: PlexStatus;
}

/** A Plex sign-in in progress; the desktop has opened `authUrl` in the browser. */
export interface PlexPin {
  id: number;
  code: string;
  authUrl: string;
}

export interface PlexServer {
  id: string;
  name: string;
  owned: boolean;
}

/** What the player is showing, for a media-server lookup. */
export type MediaTarget = { kind: 'vod'; id: number } | { kind: 'recording'; id: number } | { kind: 'programme'; id: string };

/** The playing title, found on the user's media server. */
export interface MediaMatch {
  server: 'plex';
  serverName: string;
  kind: 'movie' | 'show' | 'episode';
  title: string;
  url: string;
}

/** A credited person on a details view. */
export interface Person {
  name: string;
  /** Character (cast) or job. */
  role: string | null;
  /** `dlimg://` headshot. */
  photo: string | null;
  guest: boolean;
}

export type CrewJob = 'creator' | 'director' | 'writer' | 'producer' | 'presenter';

export interface Crew {
  job: CrewJob;
  names: string[];
}

/** Rich details for a programme, recording or VOD title: the source first, TMDB for the gaps. */
export interface MediaDetails {
  kind: 'programme' | 'recording' | 'movie' | 'series' | 'episode';
  title: string;
  episodeTitle: string | null;
  season: number | null;
  episode: number | null;
  /** This airing / episode / movie. */
  description: string | null;
  /** The show's own overview, when it differs from `description`. */
  overview: string | null;
  tagline: string | null;
  genres: string[];
  year: number | null;
  /** Original air date or release date, `YYYY-MM-DD`. */
  airDate: string | null;
  runtimeMins: number | null;
  certification: string | null;
  /** 0..10. */
  score: number | null;
  country: string | null;
  language: string | null;
  network: string | null;
  status: string | null;
  seasonCount: number | null;
  cast: Person[];
  crew: Crew[];
  poster: string | null;
  backdrop: string | null;
  still: string | null;
  /** Transparent title logo. */
  logo: string | null;
  videoQuality: string | null;
  aspectRatio: string | null;
  audio: string | null;
  isNew: boolean;
  isLive: boolean;
  isPremiere: boolean;
  isFinale: boolean;
  /** Shown before. */
  isRepeat: boolean;
  advisories: string[];
  trailerUrl: string | null;
  /** The title's TMDB page (attribution link). */
  tmdbUrl: string | null;
  fromTmdb: boolean;
}

export interface TmdbStatus {
  /** A key is saved (it never comes back to the UI). */
  configured: boolean;
  language: string;
}
