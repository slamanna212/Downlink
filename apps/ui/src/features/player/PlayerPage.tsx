import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { useIdle } from '@mantine/hooks';
import { Progress, Slider } from '@mantine/core';
import {
  IconAlertTriangle,
  IconArrowLeft,
  IconInfoCircle,
  IconMaximize,
  IconMinimize,
  IconPlayerPause,
  IconPlayerPlay,
  IconRefresh,
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
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { appendDigit, matchChannelNumber, NUMBER_ENTRY_TIMEOUT_MS } from '../../lib/numberEntry';
import { formatClock, formatClockSeconds, progress } from '../../lib/time';
import { t, type MessageKey } from '../../lib/i18n';
import { createBoundsReporter, toPhysicalBounds } from '../../lib/videoBounds';
import { useNowEpoch } from '../../lib/useNow';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { copyDiagnostics } from '../diagnostics/copyDiagnostics';
import { playChannel } from './actions';
import classes from './Player.module.css';

/** Reports the element's rect in physical pixels (ResizeObserver + one rAF debounce). */
export function useVideoBounds(ref: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const reporter = createBoundsReporter({
      measure: () => toPhysicalBounds(el.getBoundingClientRect(), window.devicePixelRatio || 1),
      send: (b) => void invoke('set_video_bounds', b).catch(() => undefined),
    });
    const ro = new ResizeObserver(() => reporter.schedule());
    ro.observe(el);
    // Position can change without a resize (rail collapse transition, window move/zoom).
    const onChange = () => reporter.schedule();
    window.addEventListener('resize', onChange);
    document.addEventListener('transitionend', onChange, true);
    reporter.schedule();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', onChange);
      document.removeEventListener('transitionend', onChange, true);
      reporter.dispose();
    };
  }, [ref]);
}

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

function useSessionStats(enabled: boolean): SessionStats | null {
  const [stats, setStats] = useState<SessionStats | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let off: (() => void) | null = null;
    let cancelled = false;
    void listen('session://stats', setStats).then((u) => (cancelled ? u() : (off = u)));
    void invoke('set_stats_enabled', { enabled: true }).catch(() => undefined);
    return () => {
      cancelled = true;
      off?.();
      void invoke('set_stats_enabled', { enabled: false }).catch(() => undefined);
    };
  }, [enabled]);
  return enabled ? stats : null;
}

function InfoPanel({ player, sourceName, channel, onClose }: { player: PlayerState; sourceName: string; channel: Channel | undefined; onClose: () => void }) {
  const diag = useQuery({ queryKey: ['diagnostics', 'session'], queryFn: () => invoke('get_diagnostics', {}), refetchInterval: 1000 });
  const stats = useSessionStats(true);
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
    [t('player.info.buffer'), stats?.bufferSecs != null ? `${stats.bufferSecs.toFixed(1)} s` : dash],
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
      <div className={classes.infoLabel}>{t('player.info.session')}</div>
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
  return (
    <div className={classes.error} role="alert">
      <div className={classes.errorMark}>
        <Icon icon={IconAlertTriangle} size={24} />
      </div>
      <h2 className={classes.errorTitle}>{t(`player.error.${code}` as MessageKey)}</h2>
      <p className={classes.errorBody}>{error?.message ?? t('error.generic')}</p>
      <div className={classes.errorActions}>
        <DLButton variant="filled" size="lg" icon={IconRefresh} onClick={onRetry}>
          {t('player.retry')}
        </DLButton>
        <DLButton
          variant="default"
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
      </div>
    </div>
  );
}

let pendingStop: ReturnType<typeof setTimeout> | null = null;

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
  const ids = useMemo(() => (player.channelId != null ? [player.channelId] : []), [player.channelId]);
  const nn = useNowNextWindow(ids, `player|${player.channelId}`);
  const now = useNowEpoch(15_000);
  const prog = nn.data?.[0]?.now ?? null;
  const next = nn.data?.[0]?.next ?? null;

  const [infoOpen, setInfoOpen] = useState(false);
  const [entry, setEntry] = useState('');
  const [entryMiss, setEntryMiss] = useState(false);
  const entryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const idle = useIdle(dl.controlsIdleMs, { events: ['mousemove', 'keydown', 'touchstart', 'click'], initialState: false });

  const failed = player.state === 'Failed';
  const paused = player.state === 'Paused';
  const barHidden = idle && !paused && !failed && !infoOpen;

  // Sessions are stopped when leaving the player (CLAUDE rule 6). The stop is deferred one tick and
  // cancelled by a re-mount, so React StrictMode's simulated unmount/mount does not kill playback.
  useEffect(() => {
    if (pendingStop) {
      clearTimeout(pendingStop);
      pendingStop = null;
    }
    return () => {
      pendingStop = setTimeout(() => {
        pendingStop = null;
        void invoke('player_command', { command: { kind: 'stop' } }).catch(() => undefined);
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
  const goBack = useCallback(() => void navigate('/channels'), [navigate]);

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
      else if (e.key === 'f' || e.key === 'F') toggleFullscreen();
      else if (e.key === 'Escape') {
        if (entry) clearEntry();
        else if (infoOpen) setInfoOpen(false);
        else if (fullscreen) toggleFullscreen();
        else goBack();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [entry, infoOpen, fullscreen, commitEntry, clearEntry, togglePause, toggleMute, toggleFullscreen, goBack]);

  useEffect(() => () => { if (entryTimer.current) clearTimeout(entryTimer.current); }, []);

  const showStatus = ['Resolving', 'Connecting', 'Buffering'].includes(player.state);
  const live = player.mode === 'live' || player.mode === null;
  const sourceName = sources.data?.find((s) => s.id === channel?.sourceId)?.name ?? '–';

  return (
    <div ref={rootRef} className={classes.root} data-video-rect data-idle={barHidden || undefined}>
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

      {showStatus && <div className={classes.status} role="status">{t(`player.state.${player.state}` as MessageKey)}</div>}

      {failed ? (
        <ErrorOverlay error={player.error} onRetry={() => cmd({ kind: 'retry' })} />
      ) : (
        <div className={classes.layer}>
          {channel && <ZapPill key={channel.id} channel={channel} title={prog?.title ?? null} until={prog?.endUtc ?? null} />}

          {entry && (
            <div className={`${classes.entry} ${classes.glass}`} data-info={infoOpen || undefined} role="status" aria-label={t('player.numberEntry')}>
              {entry.split('').join(' ')}
              {!entryMiss && <span className={classes.caret} aria-hidden />}
              {entryMiss && <span className={classes.entryHint}>{t('player.noSuchChannel', { number: entry })}</span>}
            </div>
          )}

          {infoOpen && <InfoPanel player={player} sourceName={sourceName} channel={channel} onClose={() => setInfoOpen(false)} />}

          <div className={classes.bar} data-hidden={barHidden || undefined}>
            <div className={classes.badges}>
              {live && <StatusBadge kind="live" />}
              {channel && <span>{channel.number} · {channel.name}</span>}
              {channel?.quality === '4K' && <StatusBadge kind="4k" />}
              {(channel?.quality === 'HD' || channel?.quality === 'FHD') && <StatusBadge kind="hd" />}
            </div>
            <div className={classes.titleRow}>
              <h1 className={classes.title}>{prog?.title ?? (channel ? t('player.noGuide') : '')}</h1>
              {next && <span className={classes.next}>{t('player.next', { time: formatClock(next.startUtc), title: next.title ?? '' })}</span>}
            </div>
            {prog && (
              <div className={classes.timeline}>
                <span>{formatClock(prog.startUtc)}</span>
                <Progress
                  value={progress(prog.startUtc, prog.endUtc, now) * 100}
                  size={4}
                  radius="xs"
                  classNames={{ root: classes.progressRoot, section: classes.progressSection }}
                  aria-label={prog.title ?? ''}
                />
                <span>{formatClock(prog.endUtc)}</span>
              </div>
            )}
            <div className={classes.controls}>
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
                aria-label={t('player.volume')}
              />
              <span className={classes.spacer} />
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
