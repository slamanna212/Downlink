import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useShallow } from 'zustand/react/shallow';
import { useNavigate } from 'react-router';
import { Menu } from '@mantine/core';
import { IconArrowsMaximize, IconHistory, IconInfoCircle, IconListDetails, IconPictureInPicture, IconPlayerPlay, IconStar, IconVolume, IconVolumeOff } from '@tabler/icons-react';
import { inTauri, invoke } from '../../api/ipc';
import { useChannel, useNowNextWindow } from '../../api/queries';
import { ProgrammeArt } from '../../components/ProgrammeArt/ProgrammeArt';
import { useNowEpoch } from '../../lib/useNow';
import { setFullscreen } from '../player/fullscreen';
import { setMiniPlayer } from '../player/mini';
import type { Channel, Programme } from '../../api/types';
import { dl } from '../../theme';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { dayLabel, formatRange, progress } from '../../lib/time';
import { t } from '../../lib/i18n';
import { episodeLine } from '../../lib/episode';
import { useVideoBounds } from '../../lib/useVideoBounds';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { ACTIVE_PHASES, holdSession, releaseSession } from '../player/session';
import { enterAction, type BlockState } from './guideModel';
import classes from './Guide.module.css';

export interface PreviewProps {
  programme: Programme | null;
  channel: Channel | undefined;
  state: BlockState | null;
  onPrimary: (programme: Programme, channel: Channel) => void;
  onDetails: (programme: Programme) => void;
  /** Opens the details modal for the programme (record from there or the right-click menu). */
  onInfo: (programme: Programme, channel: Channel) => void;
  onFavourite: (channel: Channel) => void;
}

/** Opens the full player; `fullscreen` also puts the window in OS fullscreen. */
function useOpenPlayer() {
  const navigate = useNavigate();
  return useCallback(
    (mode: 'player' | 'fullscreen' | 'mini') => {
      if (mode === 'fullscreen') setFullscreen(true);
      void navigate('/player');
      if (mode === 'mini') void setMiniPlayer(true);
    },
    [navigate],
  );
}

/**
 * Mini player in the hero: what's playing keeps playing when the user backs out of the player into the
 * guide. It holds the playback session while mounted (leaving the guide for anything but the player stops
 * it, CLAUDE rule 6), clears the shell so the libmpv surface shows through its rect (the hero art is
 * masked around it), and opens the full player on click. Its own controls: mute, mini window, fullscreen.
 */
function MiniPlayer({ live, programme }: { live: boolean; programme: Programme | null }) {
  const open = useOpenPlayer();
  const ref = useRef<HTMLDivElement | null>(null);
  useVideoBounds(ref);
  const muted = usePlayerStore((s) => s.player.muted);
  const now = useNowEpoch(30_000);
  // Layout effect: the shell must be clear on the first painted frame, or coming back from the player shows
  // one opaque frame over the video.
  useLayoutEffect(() => {
    holdSession();
    useUiStore.getState().setVideoHole(true);
    return () => {
      useUiStore.getState().setVideoHole(false);
      releaseSession();
    };
  }, []);
  const mute = () => void invoke('player_command', { command: { kind: 'setMuted', muted: !muted } });
  return (
    <div ref={ref} className={classes.mini}>
      {!inTauri() && (
        <span className={classes.miniDev} aria-hidden>
          mini player (libmpv)
        </span>
      )}
      <button type="button" className={classes.miniOpen} aria-label={t('guide.openPlayer')} onClick={() => open('player')} />
      {live && (
        <span className={classes.miniBadge}>
          <StatusBadge kind="live" />
        </span>
      )}
      <div className={classes.miniControls}>
        <DLIconButton label={muted ? t('player.unmute') : t('player.mute')} icon={muted ? IconVolumeOff : IconVolume} variant="overlay" size="sm" onClick={mute} />
        <span className={classes.miniSpacer} />
        <DLIconButton label={t('player.miniPlayer')} icon={IconPictureInPicture} variant="overlay" size="sm" onClick={() => open('mini')} />
        <DLIconButton label={t('player.fullscreen')} icon={IconArrowsMaximize} variant="overlay" size="sm" onClick={() => open('fullscreen')} />
      </div>
      {live && programme && <span className={classes.miniBar} style={{ width: `${progress(programme.startUtc, programme.endUtc, now) * 100}%` }} aria-hidden />}
    </div>
  );
}

/** Record items for a guide programme (guide right-click menu). */
export function GuideRecordItems({ offline, onRecord }: { offline: boolean; onRecord: (kind: 'once' | 'series') => void }) {
  return (
    <>
      {offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
      <Menu.Item disabled={offline} onClick={() => onRecord('once')}>{t('guide.recordOnce')}</Menu.Item>
      <Menu.Item disabled={offline} onClick={() => onRecord('series')}>{t('guide.recordSeries')}</Menu.Item>
    </>
  );
}

/**
 * Guide hero. Follows playback while the mini player is active, otherwise focus after a 300 ms dwell.
 * Artwork comes from `programme_art` (guide icon first; server/TMDB fallback cached in core).
 * Nothing here touches a stream endpoint (CLAUDE rule 4).
 */
export const GuidePreview = memo(function GuidePreview({ programme, channel, state, onPrimary, onDetails, onInfo, onFavourite }: PreviewProps) {
  const [shown, setShown] = useState<{ p: Programme | null; ch: Channel | undefined; state: BlockState | null }>({ p: programme, ch: channel, state });
  useEffect(() => {
    const id = setTimeout(() => setShown({ p: programme, ch: channel, state }), dl.previewDwellMs);
    return () => clearTimeout(id);
  }, [programme, channel, state]);
  const open = useOpenPlayer();

  const { playing, channelId, mode, archived, title } = usePlayerStore(useShallow(({ player }) => ({
    playing: ACTIVE_PHASES.has(player.state),
    channelId: player.channelId,
    mode: player.mode,
    archived: player.mode === 'catchup' ? player.programme : null,
    title: player.title,
  })));
  const live = mode === 'live' || mode === null;
  const { data: playingChannel } = useChannel(playing ? channelId : null);
  const liveIds = playing && live && channelId != null ? [channelId] : [];
  const nn = useNowNextWindow(liveIds, `mini|${channelId}`);
  // The now/next query carries placeholder data across channel changes. Never show the old channel.
  const current = nn.data?.find((entry) => entry.channelId === channelId)?.now ?? null;
  const p = playing ? (live ? current : archived?.channelId === channelId ? archived : null) : shown.p;
  const ch = playing ? playingChannel : shown.ch;
  const st = playing ? (live ? 'now' : mode === 'catchup' ? 'past-archive' : null) : shown.state;
  const action = !playing && st ? enterAction(st) : 'none';
  const showInfo = () => p && ch && onInfo(p, ch);
  const activate = () => p && ch && onPrimary(p, ch);
  const primary = playing
    ? { label: t('guide.fullScreen'), icon: IconArrowsMaximize, run: () => open('fullscreen') }
    : action === 'play-live'
      ? { label: t('guide.watchLive'), icon: IconPlayerPlay, filled: true, run: activate }
      : action === 'play-catchup'
        ? { label: t('guide.playFromStart'), icon: IconHistory, run: activate }
        : action === 'details'
          ? { label: t('guide.moreInfo'), icon: IconInfoCircle, run: showInfo }
          : null;
  const fallbackTitle = p?.title || title || ch?.name;
  const epLine = p ? episodeLine(p) : null;
  return (
    <section className={classes.preview} data-mini={playing || undefined}>
      <div className={classes.heroBack} aria-hidden>
        <ProgrammeArt programmeId={p?.id} channelId={ch?.id} fit="backdrop" className={classes.heroArt} />
        <div className={classes.heroScrim} />
      </div>
      <div className={classes.info}>
        {p && ch ? (
          <>
            <div className={classes.metaRow}>
              {st === 'now' && <StatusBadge kind="live" />}
              {st === 'past-archive' && <StatusBadge kind="catchup" />}
              <span>{t('guide.metaLine', { number: ch.number ?? '', channel: ch.name, day: dayLabel(p.startUtc), range: formatRange(p.startUtc, p.endUtc) })}</span>
            </div>
            <h2 className={classes.title}>{p.title}</h2>
            {(epLine || p.isNew || p.isPremiere || p.isFinale || p.isLive) && (
              <div className={classes.epRow}>
                {epLine && <span className={classes.epLine}>{epLine}</span>}
                {p.isLive && st !== 'now' && <StatusBadge kind="live" />}
                {p.isPremiere && <StatusBadge kind="premiere" />}
                {p.isFinale && <StatusBadge kind="finale" />}
                {p.isNew && <StatusBadge kind="new" />}
              </div>
            )}
            {p.description && <p className={classes.desc}>{p.description}</p>}
          </>
        ) : playing ? (
          fallbackTitle && <h2 className={classes.title}>{fallbackTitle}</h2>
        ) : (
          <div className={classes.metaRow}>{t('guide.noSelection')}</div>
        )}
        {(playing || (p && ch)) && (
          <div className={classes.actions}>
            {primary && (
              <DLButton variant="filled" size="md" icon={primary.icon} filledGlyph={'filled' in primary} onClick={primary.run}>
                {primary.label}
              </DLButton>
            )}
            {p && ch && (action === 'details' ? (
              <DLIconButton label={t('guide.channelDetails')} icon={IconListDetails} size="md" variant="default" onClick={() => onDetails(p)} />
            ) : (
              <DLIconButton label={t('guide.moreInfo')} icon={IconInfoCircle} size="md" variant="default" onClick={showInfo} />
            ))}
            {ch && (playing || p) && <DLIconButton label={t(ch.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')} icon={IconStar} size="md" variant="default" pressed={ch.isFavourite} filledGlyph={ch.isFavourite} onClick={() => onFavourite(ch)} />}
          </div>
        )}
      </div>
      {playing && <MiniPlayer live={live} programme={p} />}
    </section>
  );
});
