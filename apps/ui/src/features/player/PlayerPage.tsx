import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { useIdle } from '@mantine/hooks';
import { Progress, Slider } from '@mantine/core';
import {
  IconAlertTriangle,
  IconArrowLeft,
  IconHistory,
  IconInfoCircle,
  IconMaximize,
  IconMinimize,
  IconArrowsMaximize,
  IconPictureInPicture,
  IconLayoutGrid,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
  IconRewindBackward10,
  IconRewindForward10,
  IconVolume,
  IconVolumeOff,
  IconX,
} from '@tabler/icons-react';
import { inTauri, invoke, listen } from '../../api/ipc';
import { useChannel, useChannelProgrammes, useMediaMatch, useNowNextWindow, useSources } from '../../api/queries';
import type { AppError, Channel, MediaTarget, PlayerState, SessionStats } from '../../api/types';
import { dl } from '../../theme';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { IconPlex } from '../../components/IconPlex/IconPlex';
import { Icon } from '../../components/Icon/Icon';
import { notifyError } from '../../components/Notify/notify';
import { SeekBar } from '../../components/SeekBar/SeekBar';
import { Timeline, TIMELINE_SPAN_SECONDS } from '../../components/Timeline/Timeline';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { appendDigit, NUMBER_ENTRY_TIMEOUT_MS } from '../../lib/numberEntry';
import { goLiveTarget, isBehindLive } from '../../lib/liveEdge';
import { bucketEpoch, dayLabel, nowEpoch, formatClock, formatClockSeconds, formatDuration, formatRange, progress } from '../../lib/time';
import { t, type MessageKey } from '../../lib/i18n';
import { episodeLine } from '../../lib/episode';
import { useVideoBounds } from '../../lib/useVideoBounds';
import { useNowEpoch } from '../../lib/useNow';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { copyDiagnostics } from '../diagnostics/copyDiagnostics';
import { resumeUseful } from '../vod/vod';
import { decodeCaps } from '../../lib/caps';
import { playChannel, restartProgramme } from './actions';
import { AudioMenu, StreamMenu, SubtitleMenu, streamPosition } from './PlayerMenus';
import { rtxSummary } from './rtx';
import { useMultiviewAvailable } from '../multiview/available';
import { holdSession, releaseSession } from './session';
import { holdFullscreen, isFullscreenKey, releaseFullscreen, toggleFullscreen } from './fullscreen';
import classes from './Player.module.css';

const BUFFER_SLOTS = 30;
/** Repeated skip presses are gathered into one seek after this pause. */
const SEEK_COALESCE_MS = 250;
/** Give up waiting for the playhead to reach a pending seek after this long. */
const SEEK_PENDING_MAX_SECS = 6;

/** Live stats (1 Hz) plus the last `BUFFER_SLOTS` buffer samples for the graph. */
function useSessionStats(enabled: boolean): { stats: SessionStats | null; bufferHistory: number[] } {
  const [stats, setStats] = useState<SessionStats | null>(null);
  const [bufferHistory, setBufferHistory] = useState<number[]>([]);
  useEffect(() => {
    if (!enabled) return;
    let off: (() => void) | null = null;
    let cancelled = false;
    const onStats = (s: SessionStats) => {
      setStats(s);
      const secs = s.bufferSecs;
      if (secs != null) setBufferHistory((h) => [...h, secs].slice(-BUFFER_SLOTS));
    };
    void listen('session://stats', onStats).then((u) => (cancelled ? u() : (off = u)));
    void invoke('set_stats_enabled', { enabled: true }).catch(() => undefined);
    return () => {
      cancelled = true;
      off?.();
      void invoke('set_stats_enabled', { enabled: false }).catch(() => undefined);
    };
  }, [enabled]);
  return enabled ? { stats, bufferHistory } : { stats: null, bufferHistory: [] };
}

function BufferGraph({ history }: { history: number[] }) {
  const now = history.at(-1);
  const secs = now != null ? now.toFixed(1) : '–';
  const scale = Math.max(1, ...history);
  const pad = BUFFER_SLOTS - history.length;
  return (
    <div>
      <div className={classes.infoLabel}>{t('player.info.buffer', { secs })}</div>
      <div className={classes.bufferGraph} role="img" aria-label={t('player.info.bufferGraph', { secs })}>
        {Array.from({ length: pad }, (_, i) => (
          <div key={`pad-${i}`} className={classes.bufferSlot} aria-hidden />
        ))}
        {history.map((v, i) => (
          <div key={i} className={classes.bufferBar} style={{ height: `${Math.max(4, (v / scale) * 100)}%` }} aria-hidden />
        ))}
      </div>
    </div>
  );
}

function InfoPanel({ player, sourceName, channel, onClose }: { player: PlayerState; sourceName: string; channel: Channel | undefined; onClose: () => void }) {
  const recent = useQuery({ queryKey: ['playerTransitions', player.state], queryFn: () => invoke('get_recent_transitions', {}), staleTime: 0 });
  const { stats, bufferHistory } = useSessionStats(true);
  const dash = '–';
  const join = (parts: (string | null | undefined)[]) => parts.filter(Boolean).join(' · ') || dash;
  const upper = (v: string | null | undefined) => v?.toUpperCase();
  const mbps = (kbps: number | null | undefined) => (kbps != null ? `${(kbps / 1000).toFixed(1)} Mb/s` : null);
  const transfer = upper(stats?.transfer) ?? dash;
  const hdr =
    stats?.hdr == null ? dash
    : stats.rtxHdrActive ? t(stats.toneMapped ? 'player.info.hdrRtxToneMapped' : 'player.info.hdrRtx', { transfer })
    : !stats.hdr ? t('player.info.hdrNo')
    : t(stats.toneMapped ? 'player.info.hdrToneMapped' : 'player.info.hdrYes', { transfer });
  const rtx = rtxSummary(stats);
  const video = stats?.resolution
    ? join([stats.resolution, stats.videoCodec, stats.fps != null ? `${stats.fps.toFixed(2)} fps` : null])
    : player.width && player.height ? join([`${player.width}×${player.height}`, player.videoCodec]) : dash;
  const rows: [string, string][] = [
    [t('player.info.source'), sourceName],
    [t('player.info.channel'), channel ? `${channel.number ?? ''} ${channel.name}`.trim() : dash],
    ...(streamPosition(stats) ? [[t('player.info.stream'), streamPosition(stats) as string] as [string, string]] : []),
    [t('player.info.video'), video],
    [t('player.info.hdr'), hdr],
    ...(rtx ? [[t('player.info.rtx'), rtx] as [string, string]] : []),
    [t('player.info.colour'), join([upper(stats?.primaries), stats?.bitDepth != null ? t('player.info.bitDepth', { n: stats.bitDepth }) : null])],
    [t('player.info.deinterlace'), stats?.deinterlace ?? dash],
    [t('player.info.audio'), join([stats?.audioCodec, stats?.audioChannels, stats?.audioSampleRate ? `${stats.audioSampleRate / 1000} kHz` : null])],
    [t('player.info.container'), stats?.container ?? dash],
    [t('player.info.bitrate'), mbps(stats?.bitrateKbps) ?? dash],
    [t('player.info.download'), mbps(stats?.downloadKbps) ?? dash],
    [t('player.info.avsync'), stats?.avsyncMs != null ? `${Math.round(stats.avsyncMs) > 0 ? '+' : ''}${Math.round(stats.avsyncMs)} ms` : dash],
    [t('player.info.dropped'), stats?.droppedFrames != null ? String(stats.droppedFrames) : dash],
    [t('player.info.decoder'), stats?.hwdecCurrent ?? player.hwdecCurrent ?? dash],
    [t('player.info.state'), player.state],
  ];
  const trans = recent.data ?? [];
  return (
    <aside className={classes.info} aria-label={t('player.info')}>
      <div className={classes.infoHead}>
        <span>{t('player.info')}</span>
        <DLIconButton label={t('player.infoClose')} icon={IconX} size="sm" onClick={onClose} />
      </div>
      {rows.map(([k, v]) => (
        <div className={classes.infoRow} key={k}>
          <span>{k}</span>
          <span className={classes.infoValue}>{v}</span>
        </div>
      ))}
      <BufferGraph history={bufferHistory} />
      <div className={`${classes.infoLabel} ${classes.infoLabelPlain}`}>{t('player.info.session')}</div>
      <div className={classes.session}>
        {trans.map((x) => (
          <div key={x.at}>
            {formatClockSeconds(x.at / 1000)} {x.from} → {x.to}
          </div>
        ))}
      </div>
    </aside>
  );
}

function ErrorOverlay({ error, onRetry }: { error: AppError | null; onRetry: () => void }) {
  const [copied, setCopied] = useState(false);
  const code = error?.code ?? 'local';
  const reason = error?.reason;
  const title = reason ? t(`player.error.${reason}` as MessageKey) : t(`player.error.${code}` as MessageKey);
  const body =
    reason === 'limit'
      ? t('player.error.limit.body')
      : reason === 'codec'
        ? t('player.error.codec.body', { codec: error?.codec ?? '?' })
        : reason === 'engine'
          ? t('player.error.engine.body', { id: error?.crashId ?? '?' })
          : (error?.message ?? t('error.generic'));
  // Retrying cannot fix an undecodable format; diagnostics are the useful action there and after a crash.
  const copyFirst = reason === 'codec' || reason === 'engine';
  const copy = (
    <DLButton
      key="copy"
      variant={copyFirst ? 'filled' : 'default'}
      size="lg"
      onClick={() =>
        void copyDiagnostics().then((ok) => {
          setCopied(ok);
          setTimeout(() => setCopied(false), 2000);
        })
      }
    >
      {copied ? t('player.copied') : t('player.copyDiagnostics')}
    </DLButton>
  );
  const retry = (
    <DLButton key="retry" variant={copyFirst ? 'default' : 'filled'} size="lg" icon={IconRefresh} onClick={onRetry}>
      {t('player.retry')}
    </DLButton>
  );
  return (
    <div className={classes.error} role="alert">
      <div className={classes.errorMark}>
        <Icon icon={IconAlertTriangle} size={24} />
      </div>
      <h2 className={classes.errorTitle}>{title}</h2>
      <p className={classes.errorBody}>{body}</p>
      <div className={classes.errorActions}>{reason === 'codec' ? [copy] : copyFirst ? [copy, retry] : [retry, copy]}</div>
    </div>
  );
}

let pendingMiniReset: ReturnType<typeof setTimeout> | null = null;

/** Enter or leave the mini player (compact always-on-top window). */
export function setMiniPlayer(enabled: boolean): Promise<void> {
  return invoke('set_mini_player', { enabled })
    .then(() => useUiStore.getState().setMini(enabled))
    .catch(() => undefined);
}

/** Mini player chrome: drag anywhere, hover shows play/pause, mute, restore and close. */
function MiniOverlay({ paused, muted, onTogglePause, onToggleMute, onRestore, onClose }: {
  paused: boolean; muted: boolean; onTogglePause: () => void; onToggleMute: () => void; onRestore: () => void; onClose: () => void;
}) {
  return (
    <div className={classes.mini} data-tauri-drag-region onDoubleClick={onRestore}>
      <div className={classes.miniControls}>
        <DLIconButton label={paused ? t('player.play') : t('player.pause')} icon={paused ? IconPlayerPlay : IconPlayerPause} variant="overlay" size="md" filledGlyph onClick={onTogglePause} />
        <DLIconButton label={muted ? t('player.unmute') : t('player.mute')} icon={muted ? IconVolumeOff : IconVolume} variant="overlay" size="md" onClick={onToggleMute} />
        <DLIconButton label={t('player.miniRestore')} icon={IconArrowsMaximize} variant="overlay" size="md" onClick={onRestore} />
        <DLIconButton label={t('player.miniClose')} icon={IconX} variant="overlay" size="md" onClick={onClose} />
      </div>
    </div>
  );
}

export function PlayerPage() {
  const navigate = useNavigate();
  const player = usePlayerStore((s) => s.player);
  const fullscreen = useUiStore((s) => s.fullscreen);
  const mini = useUiStore((s) => s.mini);
  const multiviewAvailable = useMultiviewAvailable();
  const rootRef = useRef<HTMLDivElement | null>(null);
  useVideoBounds(rootRef);

  const sources = useSources();
  const { data: channel } = useChannel(player.channelId);
  const ids = useMemo(() => (player.channelId != null ? [player.channelId] : []), [player.channelId]);
  const nn = useNowNextWindow(ids, `player|${player.channelId}`);
  const now = useNowEpoch(15_000);
  const prog = nn.data?.[0]?.now ?? null;
  const next = nn.data?.[0]?.next ?? null;
  // Guide entries around the timeline window, refetched as the window moves by 5 minutes.
  const tlCenter = bucketEpoch(player.timeline?.playheadUtc ?? now);
  const tlProgs = useChannelProgrammes(player.timeline ? player.channelId : null, tlCenter - TIMELINE_SPAN_SECONDS, tlCenter + TIMELINE_SPAN_SECONDS);

  const [infoOpen, setInfoOpen] = useState(false);
  const [menu, setMenu] = useState<'audio' | 'subs' | 'stream' | null>(null);
  const [streamDenied, setStreamDenied] = useState(false);
  const menuProps = (name: 'audio' | 'subs' | 'stream') => ({
    opened: menu === name,
    onChange: (open: boolean) => setMenu(open ? name : null),
  });
  const [entry, setEntry] = useState('');
  const [entryMiss, setEntryMiss] = useState(false);
  const [resumeDismissed, setResumeDismissed] = useState<string | null>(null);
  const entryRequest = useRef(0);
  const entryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const idle = useIdle(dl.controlsIdleMs, { events: ['mousemove', 'keydown', 'touchstart', 'click'], initialState: false });

  const failed = player.state === 'Failed';
  const paused = player.state === 'Paused';
  const barHidden = idle && !paused && !failed && !infoOpen && menu === null;
  const audioTracks = player.tracks.filter((x) => x.kind === 'audio');
  const subTracks = player.tracks.filter((x) => x.kind === 'sub');
  const hasAudioMenu = audioTracks.length > 0;
  const hasSubMenu = subTracks.length > 0;

  // Sessions are stopped when leaving the player (CLAUDE rule 6), unless the guide's mini player takes the
  // session over (see ./session). Fullscreen carries over to multiview and ends anywhere else (see
  // ./fullscreen). The mini window always ends with the player; deferred one tick so React StrictMode's
  // simulated unmount/mount does not drop it.
  useEffect(() => {
    holdSession();
    holdFullscreen();
    if (pendingMiniReset) clearTimeout(pendingMiniReset);
    pendingMiniReset = null;
    return () => {
      releaseSession();
      releaseFullscreen();
      pendingMiniReset = setTimeout(() => {
        pendingMiniReset = null;
        if (useUiStore.getState().mini) void setMiniPlayer(false);
      }, 0);
    };
  }, []);

  const cmd = useCallback(
    (command: Parameters<typeof invoke<'player_command'>>[1]['command']) => void invoke('player_command', { command }),
    [],
  );
  const togglePause = useCallback(() => cmd({ kind: paused ? 'resume' : 'pause' }), [cmd, paused]);
  const toggleMute = useCallback(() => cmd({ kind: 'setMuted', muted: !player.muted }), [cmd, player.muted]);
  // Timeline seeks (live buffer / archive): shown at once, sent once the user stops pressing.
  const [pendingUtc, setPendingUtc] = useState<number | null>(null);
  const pendingRef = useRef<{ utc: number; since: number } | null>(null);
  const seekTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seekTo = useCallback(
    (utc: number, delay = 0) => {
      const tl = usePlayerStore.getState().player.timeline;
      if (!tl) return;
      const oldest = tl.archiveStartUtc ?? tl.bufferStartUtc ?? tl.playheadUtc;
      const target = Math.min(tl.liveEdgeUtc, Math.max(oldest, utc));
      pendingRef.current = { utc: target, since: nowEpoch() };
      setPendingUtc(target);
      if (seekTimer.current) clearTimeout(seekTimer.current);
      seekTimer.current = setTimeout(() => {
        seekTimer.current = null;
        cmd({ kind: 'seekTo', utc: target });
      }, delay);
    },
    [cmd],
  );
  // Clear the pending seek once the player is there (or it plainly is not going to get there).
  useEffect(() => {
    const p = pendingRef.current;
    const tl = player.timeline;
    if (!p || seekTimer.current) return;
    if (!tl || Math.abs(tl.playheadUtc - p.utc) < 3 || nowEpoch() - p.since > SEEK_PENDING_MAX_SECS) {
      pendingRef.current = null;
      setPendingUtc(null);
    }
  }, [player.timeline]);
  useEffect(() => () => {
    if (seekTimer.current) clearTimeout(seekTimer.current);
  }, []);
  const seekBy = useCallback(
    (delta: number) => {
      const st = usePlayerStore.getState().player;
      if (!st.seekable) return;
      if (st.timeline) {
        seekTo((pendingRef.current?.utc ?? st.timeline.playheadUtc) + delta, SEEK_COALESCE_MS);
        return;
      }
      if (st.mode === 'live' || st.mode === null) return;
      cmd({ kind: 'seek', seconds: Math.max(0, Math.min((st.position ?? 0) + delta, st.duration ?? 0)) });
    },
    [cmd, seekTo],
  );
  const restart = useCallback(() => {
    const st = usePlayerStore.getState().player;
    if (st.mode === 'vod' || st.mode === 'recording') {
      if (st.seekable) cmd({ kind: 'seek', seconds: 0 });
    } else if (st.channelId != null) void restartProgramme(st.channelId);
  }, [cmd]);
  // Behind live in the local buffer: jump to the edge without reloading. From the archive: back to the stream.
  const goLive = useCallback(() => {
    const st = usePlayerStore.getState().player;
    if (st.mode === 'live' && st.timeline) seekTo(goLiveTarget(st.timeline.liveEdgeUtc, st.timeline.archiveStartUtc ?? st.timeline.bufferStartUtc));
    else cmd({ kind: 'goLive' });
  }, [cmd, seekTo]);
  const toggleMini = useCallback(() => void setMiniPlayer(!useUiStore.getState().mini), []);
  // Back to wherever the player was opened from (the guide keeps playing in its mini player); a player
  // reached without history (resume on launch, deep link into a fresh window) falls back to the list.
  const goBack = useCallback(() => {
    const idx = (window.history.state as { idx?: number } | null)?.idx ?? 0;
    if (idx > 0) void navigate(-1);
    else void navigate('/channels');
  }, [navigate]);

  const clearEntry = useCallback(() => {
    entryRequest.current++;
    if (entryTimer.current) clearTimeout(entryTimer.current);
    entryTimer.current = null;
    setEntry('');
  }, []);

  const commitEntry = useCallback(
    (value: string) => {
      clearEntry();
      const request = entryRequest.current;
      void invoke('find_channel_number', { number: Number(value) }).then((id) => {
        if (request !== entryRequest.current) return;
        if (id != null) {
          void playChannel(id);
          return;
        }
        setEntryMiss(true);
        setEntry(value);
        entryTimer.current = setTimeout(() => {
          setEntryMiss(false);
          setEntry('');
        }, 1200);
      }).catch(() => undefined);
    },
    [clearEntry],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Fullscreen: F11, and Ctrl+Cmd+F on macOS .
      if (isFullscreenKey(e)) {
        e.preventDefault();
        toggleFullscreen();
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target && ['INPUT', 'TEXTAREA'].includes(target.tagName)) return;
      // Keys inside an open menu (Esc closes it) belong to the menu, not the player.
      if (target?.closest('[role="menu"], .mantine-Menu-dropdown, [data-mantine-stop-propagation]')) return;
      if (e.key === 'p' || e.key === 'P') {
        e.preventDefault();
        toggleMini();
        return;
      }
      if (mini) {
        if (e.key === ' ') {
          e.preventDefault();
          togglePause();
        } else if (e.key === 'm' || e.key === 'M') toggleMute();
        else if (e.key === 'Escape') toggleMini();
        return;
      }
      if (/^[0-9]$/.test(e.key)) {
        e.preventDefault();
        entryRequest.current++;
        setEntryMiss(false);
        const next = appendDigit(entry, e.key);
        setEntry(next);
        if (entryTimer.current) clearTimeout(entryTimer.current);
        entryTimer.current = setTimeout(() => commitEntry(next), NUMBER_ENTRY_TIMEOUT_MS);
      } else if (e.key === 'Enter' && entry) {
        e.preventDefault();
        if (entryTimer.current) clearTimeout(entryTimer.current);
        commitEntry(entry);
      } else if (e.key === ' ' && target?.tagName !== 'BUTTON') {
        e.preventDefault();
        togglePause();
      } else if (e.key === 'm' || e.key === 'M') toggleMute();
      else if (e.key === 'i' || e.key === 'I') setInfoOpen((v) => !v);
      else if ((e.key === 'a' || e.key === 'A') && hasAudioMenu) setMenu((m) => (m === 'audio' ? null : 'audio'));
      else if ((e.key === 't' || e.key === 'T') && hasSubMenu) setMenu((m) => (m === 'subs' ? null : 'subs'));
      else if (e.key === 'ArrowLeft') seekBy(e.shiftKey ? -60 : -10);
      else if (e.key === 'ArrowRight') seekBy(e.shiftKey ? 60 : 10);
      else if (e.key === 's' || e.key === 'S') restart();
      else if (e.key === 'End') goLive();
      else if (e.key === 'Escape') {
        if (menu) setMenu(null);
        else if (entry) clearEntry();
        else if (infoOpen) setInfoOpen(false);
        else if (fullscreen) toggleFullscreen();
        else goBack();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [mini, toggleMini, entry, infoOpen, menu, hasAudioMenu, hasSubMenu, fullscreen, commitEntry, clearEntry, togglePause, toggleMute, goBack, seekBy, restart, goLive]);

  useEffect(() => () => {
    entryRequest.current++;
    if (entryTimer.current) clearTimeout(entryTimer.current);
  }, []);

  const showStatus = player.reconnecting || ['Resolving', 'Connecting', 'Buffering'].includes(player.state);
  const catchup = player.mode === 'catchup';
  // Seekable media with a position/duration bar (catch-up keeps this name: it is seekable too).
  const timeshift = player.mode === 'catchup' || player.mode === 'vod' || player.mode === 'recording';
  const live = !timeshift && (player.mode === 'live' || player.mode === null);
  const tl = player.timeline;
  const behindLive = tl ? Math.max(0, tl.liveEdgeUtc - (pendingUtc ?? tl.playheadUtc)) : 0;
  // Every control below shows only when it does something for what is playing; nothing is shown disabled.
  const canSkip = player.seekable && (player.timeline != null || (player.mode !== 'live' && player.mode !== null && (player.duration ?? 0) > 0));
  // A few seconds behind the tip still counts as live; hysteresis keeps the button from flickering.
  const [behindFlag, setBehindFlag] = useState(false);
  const behindNow = isBehindLive(behindLive, behindFlag);
  if (behindNow !== behindFlag) setBehindFlag(behindNow);
  // Start over: live/catch-up restart the programme; VOD and recordings jump back to 0 once past the start.
  const canRestart = ((live || catchup) && !!channel) || ((player.mode === 'vod' || player.mode === 'recording') && player.seekable && (player.position ?? 0) > 5);
  const showGoLive = catchup || (live && behindNow);
  const source = sources.data?.find((s) => s.id === channel?.sourceId);
  const sourceName = source?.name ?? '–';
  const canSwitchStream = live && player.state !== 'Idle' && !!channel && !!source && decodeCaps(source.caps).streamSwitch && !streamDenied;
  const shownProg = catchup ? player.programme : prog;
  const position = player.position ?? 0;
  const duration = player.duration ?? 0;
  // Playback has already started at `resumeAt`; the prompt only offers to go back to the start. Recordings and
  // VOD have no programme, so the dismiss key covers mode + title + position, not just the programme id.
  const resumeKey = `${player.mode}|${player.programme?.id ?? player.title ?? ''}|${player.resumeAt ?? ''}`;
  const mediaTarget = useMemo<MediaTarget | null>(() => {
    if ((player.mode === 'vod' || player.mode === 'recording') && player.mediaId != null) return { kind: player.mode, id: player.mediaId };
    return shownProg ? { kind: 'programme', id: shownProg.id } : null;
  }, [player.mode, player.mediaId, shownProg]);
  const mediaMatch = useMediaMatch(player.state === 'Idle' || player.state === 'Stopped' ? null : mediaTarget).data ?? null;
  const showResume = timeshift && player.mode !== 'vod' && resumeUseful(player.resumeAt, player.duration) && resumeDismissed !== resumeKey;
  // A recording with a saved position is loaded paused (downlink-player `start_paused`) so nothing plays under
  // the prompt. Whenever the prompt is not up (either button, or it never applied: the duration says the
  // position is no longer useful), release the pause once per session. Escape leaves the player entirely.
  const heldPause = player.mode === 'recording' && player.resumeAt != null && player.resumeAt > 2;
  const releasedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!heldPause || showResume || player.state !== 'Paused' || releasedFor.current === resumeKey) return;
    releasedFor.current = resumeKey;
    cmd({ kind: 'resume' });
  }, [heldPause, showResume, player.state, resumeKey, cmd]);

  return (
    <div
      ref={rootRef}
      className={classes.root}
      onDoubleClick={(e) => {
        if (mini) return;
        if (!(e.target as HTMLElement).closest('button, input, [role="slider"], [role="dialog"]')) toggleFullscreen();
      }}
      data-video-rect data-idle={barHidden || undefined} data-mini={mini || undefined}>
      {mini ? (
        <MiniOverlay
          paused={paused}
          muted={player.muted}
          onTogglePause={togglePause}
          onToggleMute={toggleMute}
          onRestore={toggleMini}
          onClose={() => void setMiniPlayer(false).then(goBack)}
        />
      ) : (
      <>
      {!inTauri() && <div className={classes.devVideo} aria-hidden>video surface (libmpv, under webview)</div>}
      {player.state === 'Idle' || player.state === 'Stopped' ? (
        <div className={classes.layer}>
          <div className={classes.status} style={{ pointerEvents: 'auto' }}>
            <DLButton variant="overlay" size="lg" icon={IconArrowLeft} onClick={goBack}>
              {t('player.back')}
            </DLButton>
          </div>
        </div>
      ) : null}

      {showStatus && <div className={classes.status} role="status">{player.reconnecting ? t('player.reconnecting') : t(`player.state.${player.state}` as MessageKey)}</div>}

      {failed ? (
        <ErrorOverlay error={player.error} onRetry={() => cmd({ kind: 'retry' })} />
      ) : (
        <div className={classes.layer}>
          {entry && (
            <div className={`${classes.entry} ${classes.glass}`} data-info={infoOpen || undefined} role="status" aria-label={t('player.numberEntry')}>
              {entry.split('').join(' ')}
              {!entryMiss && <span className={classes.caret} aria-hidden />}
              {entryMiss && <span className={classes.entryHint}>{t('player.noSuchChannel', { number: entry })}</span>}
            </div>
          )}

          {infoOpen && <InfoPanel player={player} sourceName={sourceName} channel={channel} onClose={() => setInfoOpen(false)} />}

          {showResume && player.resumeAt != null && (
            <div className={`${classes.resume} ${classes.glass}`} role="dialog" aria-label={t('player.resumeTitle', { time: formatDuration(player.resumeAt) })}>
              <span className={classes.resumeTitle}>{t('player.resumeTitle', { time: formatDuration(player.resumeAt) })}</span>
              <div className={classes.errorActions}>
                <DLButton variant="filled" onClick={() => setResumeDismissed(resumeKey)}>
                  {t('player.resume')}
                </DLButton>
                <DLButton
                  variant="default"
                  onClick={() => {
                    cmd({ kind: 'seek', seconds: 0 });
                    setResumeDismissed(resumeKey);
                  }}
                >
                  {t('player.startOver')}
                </DLButton>
              </div>
            </div>
          )}

          <div className={classes.bar} data-hidden={barHidden || undefined}>
            <div className={classes.nowRow}>
              {channel && <ChannelLogo channelId={channel.id} name={channel.name} src={channel.logo} density="tv" bare />}
              <div className={classes.nowText}>
                <div className={classes.badges}>
                  {live && <StatusBadge kind="live" />}
                  {catchup && <StatusBadge kind="catchup" />}
                  {player.mode === 'recording' && <StatusBadge kind="rec" />}
                  {channel && (
                    <span>
                      {catchup && shownProg
                        ? t('player.catchupMeta', { number: channel.number ?? '', channel: channel.name, day: dayLabel(shownProg.startUtc, now), range: formatRange(shownProg.startUtc, shownProg.endUtc) })
                        : `${channel.number} · ${channel.name}`}
                    </span>
                  )}
                  {channel?.quality === '4K' && <StatusBadge kind="4k" />}
                  {(channel?.quality === 'HD' || channel?.quality === 'FHD') && <StatusBadge kind="hd" />}
                  {!player.title && shownProg?.isPremiere && <StatusBadge kind="premiere" />}
                  {!player.title && shownProg?.isFinale && <StatusBadge kind="finale" />}
                  {!player.title && shownProg?.isNew && <StatusBadge kind="new" />}
                </div>
                <div className={classes.titleRow}>
                  <h1 className={classes.title}>{player.title ?? shownProg?.title ?? (channel ? t('player.noGuide') : '')}</h1>
                  {live && next && <span className={classes.next}>{t('player.next', { time: formatClock(next.startUtc), title: next.title ?? '' })}</span>}
                </div>
                {!player.title && shownProg && episodeLine(shownProg) && <div className={classes.epLine}>{episodeLine(shownProg)}</div>}
              </div>
            </div>
            {tl && player.channelId != null ? (
              <div className={classes.timeline}>
                <span>{formatClock(Math.round(pendingUtc ?? tl.playheadUtc))}</span>
                <Timeline
                  label={t('player.timeline')}
                  timeline={tl}
                  programmes={tlProgs.data ?? []}
                  pendingUtc={pendingUtc}
                  disabled={!player.seekable}
                  playing={player.state === 'Playing'}
                  onSeekTo={(utc) => seekTo(utc)}
                />
                <span aria-label={behindNow ? t('player.behindLive', { time: formatDuration(behindLive) }) : undefined}>
                  {behindNow ? `−${formatDuration(behindLive)}` : ''}
                </span>
              </div>
            ) : timeshift && player.seekable && duration > 0 ? (
              <div className={classes.timeline}>
                <span>{formatDuration(position)}</span>
                <SeekBar label={t('player.seek')} position={position} duration={duration} buffered={player.buffered ?? 0} onSeek={(s) => cmd({ kind: 'seek', seconds: s })} />
                <span>{t('player.remaining', { time: formatDuration(Math.max(0, duration - position)) })}</span>
              </div>
            ) : (
              shownProg && (
                <div className={classes.timeline}>
                  <span>{formatClock(shownProg.startUtc)}</span>
                  <Progress
                    value={progress(shownProg.startUtc, shownProg.endUtc, now) * 100}
                    size={4}
                    radius="xs"
                    classNames={{ root: classes.progressRoot, section: classes.progressSection }}
                    aria-label={shownProg.title ?? ''}
                  />
                  <span>{formatClock(shownProg.endUtc)}</span>
                </div>
              )
            )}
            <div className={classes.controls}>
              <div className={classes.group} data-slot="start">
                <DLIconButton label={t('player.back')} icon={IconArrowLeft} variant="overlay" size="lg" onClick={goBack} />
                <DLIconButton
                  label={player.muted ? t('player.unmute') : t('player.mute')}
                  icon={player.muted ? IconVolumeOff : IconVolume}
                  variant="overlay"
                  size="lg"
                  onClick={toggleMute}
                />
                <Slider
                  className={classes.volume}
                  size="xs"
                  min={0}
                  max={100}
                  value={player.muted ? 0 : player.volume}
                  onChange={(v) => cmd({ kind: 'setVolume', volume: v })}
                  label={null}
                  thumbLabel={t('player.volume')}
                />
                {hasSubMenu && <SubtitleMenu tracks={player.tracks} {...menuProps('subs')} />}
              </div>
              <div className={classes.transport} data-slot="transport">
              <div className={`${classes.group} ${classes.groupEnd}`}>
              {canRestart && (
                <DLButton variant="overlay" size="lg" icon={IconHistory} keyHint="S" onClick={restart}>
                  {t('player.restart')}
                </DLButton>
              )}
              {canSkip && <DLIconButton label={t('player.back10')} icon={IconRewindBackward10} variant="overlay" size="lg" onClick={() => seekBy(-10)} />}
              </div>
              <span className={classes.playPause}>
                <DLIconButton
                  label={paused ? t('player.play') : t('player.pause')}
                  icon={paused ? IconPlayerPlay : IconPlayerPause}
                  variant="overlay"
                  size="lg"
                  filledGlyph
                  onClick={togglePause}
                />
              </span>
              <div className={classes.group}>
              {canSkip && <DLIconButton label={t('player.fwd10')} icon={IconRewindForward10} variant="overlay" size="lg" onClick={() => seekBy(10)} />}
              {showGoLive && (
                <DLButton variant="live" size="lg" keyHint="End" onClick={goLive}>
                  {t('player.goLive')}
                </DLButton>
              )}
              </div>
              </div>
              <div className={`${classes.group} ${classes.groupEnd}`} data-slot="end">
              {hasAudioMenu && <AudioMenu tracks={player.tracks} {...menuProps('audio')} />}
              {canSwitchStream && channel && (
                <StreamMenu channelId={channel.id} onDenied={() => setStreamDenied(true)} {...menuProps('stream')} />
              )}
              {mediaMatch && (
                <DLIconButton
                  label={t('player.openInPlex.help', { title: mediaMatch.title, server: mediaMatch.serverName })}
                  icon={IconPlex}
                  variant="overlay"
                  size="lg"
                  onClick={() => void invoke('media_server_open', { url: mediaMatch.url }).catch(notifyError)}
                />
              )}
              <DLIconButton
                label={t('player.info')}
                icon={IconInfoCircle}
                variant="overlay"
                size="lg"
                pressed={infoOpen}
                onClick={() => setInfoOpen((v) => !v)}
              />
              {live && multiviewAvailable && player.channelId != null && (
                <DLIconButton label={t('player.multiview')} icon={IconLayoutGrid} variant="overlay" size="lg" onClick={() => void navigate('/multiview')} />
              )}
              <DLIconButton label={t('player.miniPlayer')} icon={IconPictureInPicture} variant="overlay" size="lg" onClick={toggleMini} />
              <DLIconButton
                label={fullscreen ? t('player.exitFullscreen') : t('player.fullscreen')}
                icon={fullscreen ? IconMinimize : IconMaximize}
                variant="overlay"
                size="lg"
                onClick={toggleFullscreen}
              />
              </div>
            </div>
          </div>
        </div>
      )}
      </>
      )}
    </div>
  );
}
