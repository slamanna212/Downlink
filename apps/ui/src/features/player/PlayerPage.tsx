import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useIdle } from '@mantine/hooks';
import { Progress, Slider } from '@mantine/core';
import {
  IconAlertTriangle,
  IconArrowLeft,
  IconHistory,
  IconInfoCircle,
  IconMaximize,
  IconMinimize,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
  IconStar,
  IconRewindBackward10,
  IconRewindForward10,
  IconVolume,
  IconVolumeOff,
  IconX,
} from '@tabler/icons-react';
import { inTauri, invoke, listen } from '../../api/ipc';
import { useChannels, useNowNextWindow, useSources } from '../../api/queries';
import type { AppError, Channel, PlayerState, SessionStats } from '../../api/types';
import { dl } from '../../theme';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { notifyInfo } from '../../components/Notify/notify';
import { SeekBar } from '../../components/SeekBar/SeekBar';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { appendDigit, matchChannelNumber, NUMBER_ENTRY_TIMEOUT_MS } from '../../lib/numberEntry';
import { dayLabel, formatClock, formatClockSeconds, formatDuration, formatRange, progress } from '../../lib/time';
import { t, type MessageKey } from '../../lib/i18n';
import { useVideoBounds } from '../../lib/useVideoBounds';
import { useNowEpoch } from '../../lib/useNow';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { copyDiagnostics } from '../diagnostics/copyDiagnostics';
import { resumeUseful } from '../vod/vod';
import { playChannel, restartProgramme } from './actions';
import { holdSession, releaseSession } from './session';
import classes from './Player.module.css';

function ZapPill({ channel, title, until }: { channel: Channel; title: string | null; until: number | null }) {
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const id = setTimeout(() => setVisible(false), dl.zapOverlayMs);
    return () => clearTimeout(id);
  }, []);
  return (
    <div className={`${classes.zap} ${classes.glass}`} data-visible={visible || undefined} aria-live="polite">
      <ChannelLogo channelId={channel.id} name={channel.name} src={channel.logo} density="tv" />
      <span className={classes.zapNumber}>{channel.number ?? ''}</span>
      <span>
        <div className={classes.zapName}>{channel.name}</div>
        {title && until != null && (
          <div className={classes.zapMeta}>{t('player.until', { title, time: formatClock(until) })}</div>
        )}
      </span>
    </div>
  );
}

const BUFFER_SLOTS = 30;

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
  const diag = useQuery({ queryKey: ['diagnostics', 'session'], queryFn: () => invoke('get_diagnostics', {}), refetchInterval: 1000 });
  const { stats, bufferHistory } = useSessionStats(true);
  const dash = '–';
  const rows: [string, string][] = [
    [t('player.info.source'), sourceName],
    [t('player.info.channel'), channel ? `${channel.number ?? ''} ${channel.name}`.trim() : dash],
    [t('player.info.stream'), stats?.sourceIndex != null ? t('player.info.streamOf', { n: stats.sourceIndex, total: stats.sourceCount ?? stats.sourceIndex }) : dash],
    [t('player.info.video'), stats?.resolution ? `${stats.resolution} · ${stats.videoCodec ?? dash}` : player.width && player.height ? `${player.width}×${player.height} · ${player.videoCodec ?? dash}` : dash],
    [t('player.info.deinterlace'), stats?.deinterlace ?? dash],
    [t('player.info.audio'), stats?.audioCodec ?? dash],
    [t('player.info.bitrate'), stats?.bitrateKbps != null ? `${(stats.bitrateKbps / 1000).toFixed(1)} Mb/s` : dash],
    [t('player.info.dropped'), stats?.droppedFrames != null ? String(stats.droppedFrames) : dash],
    [t('player.info.decoder'), stats?.hwdecCurrent ?? player.hwdecCurrent ?? dash],
    [t('player.info.state'), player.state],
  ];
  const trans = (diag.data?.transitions ?? []).slice(-3);
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

let pendingFullscreenReset: ReturnType<typeof setTimeout> | null = null;

export function PlayerPage() {
  const navigate = useNavigate();
  const player = usePlayerStore((s) => s.player);
  const fullscreen = useUiStore((s) => s.fullscreen);
  const setFullscreen = useUiStore((s) => s.setFullscreen);
  const rootRef = useRef<HTMLDivElement | null>(null);
  useVideoBounds(rootRef);

  const channels = useChannels({});
  const sources = useSources();
  const channel = useMemo(() => channels.data?.find((c) => c.id === player.channelId), [channels.data, player.channelId]);
  const channelRef = useRef<Channel | undefined>(channel);
  useEffect(() => {
    channelRef.current = channel;
  });
  const qc = useQueryClient();
  const ids = useMemo(() => (player.channelId != null ? [player.channelId] : []), [player.channelId]);
  const nn = useNowNextWindow(ids, `player|${player.channelId}`);
  const now = useNowEpoch(15_000);
  const prog = nn.data?.[0]?.now ?? null;
  const next = nn.data?.[0]?.next ?? null;

  const [infoOpen, setInfoOpen] = useState(false);
  const [entry, setEntry] = useState('');
  const [entryMiss, setEntryMiss] = useState(false);
  const [resumeDismissed, setResumeDismissed] = useState<string | null>(null);
  const entryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const idle = useIdle(dl.controlsIdleMs, { events: ['mousemove', 'keydown', 'touchstart', 'click'], initialState: false });

  const failed = player.state === 'Failed';
  const paused = player.state === 'Paused';
  const barHidden = idle && !paused && !failed && !infoOpen;

  // Sessions are stopped when leaving the player (CLAUDE rule 6), unless the guide's mini player takes the
  // session over (see ./session). Fullscreen always ends with the player; deferred one tick so React
  // StrictMode's simulated unmount/mount does not drop it.
  useEffect(() => {
    holdSession();
    if (pendingFullscreenReset) clearTimeout(pendingFullscreenReset);
    pendingFullscreenReset = null;
    return () => {
      releaseSession();
      pendingFullscreenReset = setTimeout(() => {
        pendingFullscreenReset = null;
        if (!useUiStore.getState().fullscreen) return;
        useUiStore.getState().setFullscreen(false);
        void invoke('set_fullscreen', { enabled: false }).catch(() => undefined);
      }, 0);
    };
  }, []);

  const cmd = useCallback(
    (command: Parameters<typeof invoke<'player_command'>>[1]['command']) => void invoke('player_command', { command }),
    [],
  );
  const togglePause = useCallback(() => cmd({ kind: paused ? 'resume' : 'pause' }), [cmd, paused]);
  const toggleMute = useCallback(() => cmd({ kind: 'setMuted', muted: !player.muted }), [cmd, player.muted]);
  const toggleFullscreen = useCallback(() => {
    const enabled = !fullscreen;
    setFullscreen(enabled);
    void invoke('set_fullscreen', { enabled });
  }, [fullscreen, setFullscreen]);
  const toggleFavourite = useCallback(() => {
    const st = usePlayerStore.getState().player;
    const ch = channelRef.current;
    if (st.channelId == null || !ch) return;
    const favourite = !ch.isFavourite;
    void invoke('set_favourite', { channelId: ch.id, favourite }).then(() => {
      void qc.invalidateQueries({ queryKey: ['catalog'] });
      notifyInfo(t(favourite ? 'player.favouriteAdded' : 'player.favouriteRemoved', { name: ch.name }));
    });
  }, [qc]);
  const seekBy = useCallback(
    (delta: number) => {
      const st = usePlayerStore.getState().player;
      if (st.mode === 'live' || st.mode === null || !st.seekable) return;
      cmd({ kind: 'seek', seconds: Math.max(0, Math.min((st.position ?? 0) + delta, st.duration ?? 0)) });
    },
    [cmd],
  );
  const restart = useCallback(() => {
    const st = usePlayerStore.getState().player;
    if (st.channelId != null) void restartProgramme(st.channelId);
  }, []);
  const goLive = useCallback(() => cmd({ kind: 'goLive' }), [cmd]);
  // Back to wherever the player was opened from (the guide keeps playing in its mini player); a player
  // reached without history (resume on launch, deep link into a fresh window) falls back to the list.
  const goBack = useCallback(() => {
    const idx = (window.history.state as { idx?: number } | null)?.idx ?? 0;
    if (idx > 0) void navigate(-1);
    else void navigate('/channels');
  }, [navigate]);

  const clearEntry = useCallback(() => {
    if (entryTimer.current) clearTimeout(entryTimer.current);
    entryTimer.current = null;
    setEntry('');
  }, []);

  const commitEntry = useCallback(
    (value: string) => {
      const match = matchChannelNumber(value, channels.data ?? []);
      clearEntry();
      if (match) void playChannel(match.id);
      else {
        setEntryMiss(true);
        setEntry(value);
        entryTimer.current = setTimeout(() => {
          setEntryMiss(false);
          setEntry('');
        }, 1200);
      }
    },
    [channels.data, clearEntry],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Fullscreen: F11, and Ctrl+Cmd+F on macOS (F itself is favourite, PLAN §8.4).
      if (e.key === 'F11' || (e.ctrlKey && e.metaKey && (e.key === 'f' || e.key === 'F'))) {
        e.preventDefault();
        toggleFullscreen();
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target && ['INPUT', 'TEXTAREA'].includes(target.tagName)) return;
      if (/^[0-9]$/.test(e.key)) {
        e.preventDefault();
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
      else if (e.key === 'f' || e.key === 'F') toggleFavourite();
      else if (e.key === 'ArrowLeft') seekBy(-10);
      else if (e.key === 'ArrowRight') seekBy(10);
      else if (e.key === 's' || e.key === 'S') restart();
      else if (e.key === 'End') goLive();
      else if (e.key === 'Escape') {
        if (entry) clearEntry();
        else if (infoOpen) setInfoOpen(false);
        else if (fullscreen) toggleFullscreen();
        else goBack();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [entry, infoOpen, fullscreen, toggleFavourite, commitEntry, clearEntry, togglePause, toggleMute, toggleFullscreen, goBack, seekBy, restart, goLive]);

  useEffect(() => () => { if (entryTimer.current) clearTimeout(entryTimer.current); }, []);

  const showStatus = player.reconnecting || ['Resolving', 'Connecting', 'Buffering'].includes(player.state);
  const catchup = player.mode === 'catchup';
  const timeshift = player.mode === 'catchup' || player.mode === 'vod' || player.mode === 'recording';
  const live = !timeshift && (player.mode === 'live' || player.mode === null);
  const sourceName = sources.data?.find((s) => s.id === channel?.sourceId)?.name ?? '–';
  const shownProg = catchup ? player.programme : prog;
  const position = player.position ?? 0;
  const duration = player.duration ?? 0;
  const showResume = timeshift && resumeUseful(player.resumeAt, player.duration) && resumeDismissed !== player.programme?.id;

  return (
    <div
      ref={rootRef}
      className={classes.root}
      onDoubleClick={(e) => {
        if (!(e.target as HTMLElement).closest('button, input, [role="slider"], [role="dialog"]')) toggleFullscreen();
      }}
      data-video-rect data-idle={barHidden || undefined}>
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
          {channel && <ZapPill key={channel.id} channel={channel} title={shownProg?.title ?? null} until={shownProg?.endUtc ?? null} />}

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
                <DLButton
                  variant="filled"
                  onClick={() => {
                    cmd({ kind: 'seek', seconds: player.resumeAt ?? 0 });
                    setResumeDismissed(player.programme?.id ?? null);
                  }}
                >
                  {t('player.resume')}
                </DLButton>
                <DLButton variant="default" onClick={() => setResumeDismissed(player.programme?.id ?? null)}>
                  {t('player.startOver')}
                </DLButton>
              </div>
            </div>
          )}

          <div className={classes.bar} data-hidden={barHidden || undefined}>
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
            </div>
            <div className={classes.titleRow}>
              <h1 className={classes.title}>{player.title ?? shownProg?.title ?? (channel ? t('player.noGuide') : '')}</h1>
              {live && next && <span className={classes.next}>{t('player.next', { time: formatClock(next.startUtc), title: next.title ?? '' })}</span>}
            </div>
            {timeshift && player.seekable && duration > 0 ? (
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
              <DLIconButton label={t('player.back')} icon={IconArrowLeft} variant="overlay" size="lg" onClick={goBack} />
              {timeshift && <DLIconButton label={t('player.back10')} icon={IconRewindBackward10} variant="overlay" size="lg" onClick={() => seekBy(-10)} />}
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
              {timeshift && <DLIconButton label={t('player.fwd10')} icon={IconRewindForward10} variant="overlay" size="lg" onClick={() => seekBy(10)} />}
              {(live || catchup) && channel && (
                <DLButton variant="overlay" size="lg" icon={IconHistory} keyHint="S" onClick={restart}>
                  {t('player.restart')}
                </DLButton>
              )}
              {catchup && (
                <DLButton variant="live" size="lg" keyHint="End" onClick={goLive}>
                  {t('player.goLive')}
                </DLButton>
              )}
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
              <span className={classes.spacer} />
              {channel && (
                <DLIconButton
                  label={channel.isFavourite ? t('player.favouriteRemove') : t('player.favouriteAdd')}
                  icon={IconStar}
                  variant="overlay"
                  size="lg"
                  pressed={channel.isFavourite}
                  filledGlyph={channel.isFavourite}
                  onClick={toggleFavourite}
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
      )}
    </div>
  );
}
