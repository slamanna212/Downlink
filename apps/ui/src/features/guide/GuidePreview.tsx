import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { Menu } from '@mantine/core';
import { IconChevronDown, IconStar } from '@tabler/icons-react';
import { inTauri } from '../../api/ipc';
import type { Channel, Programme } from '../../api/types';
import { dl } from '../../theme';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { dayLabel, formatRange } from '../../lib/time';
import { t } from '../../lib/i18n';
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
  canRecord: boolean;
  /** Source offline: record items stay visible but disabled, with the reason shown. */
  offline?: boolean;
  canCatchup: boolean;
  recordOpen: boolean;
  onRecordOpen: (open: boolean) => void;
  onRecord: (kind: 'once' | 'series') => void;
  onPrimary: () => void;
  onFromStart: () => void;
  onDetails: () => void;
  onFavourite: () => void;
}

/**
 * Mini player in the preview box: what's playing keeps playing when the user backs out of the player into
 * the guide. It holds the playback session while mounted (leaving the guide for anything but the player
 * stops it, CLAUDE rule 6), clears the shell so the libmpv surface shows through, and opens the full
 * player on click.
 */
function MiniPlayer() {
  const navigate = useNavigate();
  const ref = useRef<HTMLButtonElement | null>(null);
  useVideoBounds(ref);
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
  return (
    <button ref={ref} type="button" className={classes.mini} aria-label={t('guide.openPlayer')} onClick={() => void navigate('/player')}>
      {!inTauri() && (
        <span className={classes.miniDev} aria-hidden>
          mini player (libmpv)
        </span>
      )}
    </button>
  );
}

/**
 * Preview pane. It follows focus after a 300 ms dwell. Everything shown is already in memory:
 * dwelling never calls a play command or touches a stream endpoint (CLAUDE rule 4).
 */
export const GuidePreview = memo(function GuidePreview({ programme, channel, state, canRecord, offline, canCatchup, recordOpen, onRecordOpen, onRecord, onPrimary, onFromStart, onDetails, onFavourite }: PreviewProps) {
  const [shown, setShown] = useState<{ p: Programme | null; ch: Channel | undefined; state: BlockState | null }>({ p: programme, ch: channel, state });
  useEffect(() => {
    const id = setTimeout(() => setShown({ p: programme, ch: channel, state }), dl.previewDwellMs);
    return () => clearTimeout(id);
  }, [programme, channel, state]);

  const playing = usePlayerStore((s) => ACTIVE_PHASES.has(s.player.state));
  const { p, ch, state: st } = shown;
  const action = st ? enterAction(st) : 'none';
  const primaryLabel = action === 'play-live' ? t('guide.watchLive') : action === 'play-catchup' ? t('guide.playFromStart') : action === 'details' ? t('guide.details') : null;
  return (
    <div className={classes.preview} data-mini={playing || undefined}>
      <div className={classes.info}>
        {p && ch ? (
          <>
            <div className={classes.metaRow}>
              {st === 'now' && <StatusBadge kind="live" />}
              {st === 'past-archive' && <StatusBadge kind="catchup" />}
              <span>{t('guide.metaLine', { number: ch.number ?? '', channel: ch.name, day: dayLabel(p.startUtc), range: formatRange(p.startUtc, p.endUtc) })}</span>
            </div>
            <h2 className={classes.title}>{p.title}</h2>
            <p className={classes.desc}>{p.description}</p>
            <div className={classes.actions}>
              {primaryLabel && (
                <DLButton variant="filled" size="md" onClick={action === 'details' ? onDetails : onPrimary}>
                  {primaryLabel}
                </DLButton>
              )}
              <DLButton variant="default" size="md" onClick={onDetails}>
                {t('guide.channelDetails')}
              </DLButton>
              <DLIconButton label={t(ch.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')} icon={IconStar} size="md" variant="default" pressed={ch.isFavourite} filledGlyph={ch.isFavourite} onClick={onFavourite} />
              {((canRecord && st !== 'past-archive' && st !== 'past-none') || (st === 'now' && canCatchup)) && (
                <Menu opened={recordOpen} onChange={onRecordOpen} position="bottom-start">
                  <Menu.Target>
                    <DLButton variant="default" size="md">
                      {t('guide.record')} <IconChevronDown size={14} />
                    </DLButton>
                  </Menu.Target>
                  <Menu.Dropdown>
                    {st === 'now' && canCatchup && <Menu.Item onClick={onFromStart}>{t('guide.fromStart')}</Menu.Item>}
                    {canRecord && offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
                    {canRecord && <Menu.Item disabled={offline} onClick={() => onRecord('once')}>{t('guide.recordOnce')}</Menu.Item>}
                    {canRecord && <Menu.Item disabled={offline} onClick={() => onRecord('series')}>{t('guide.recordSeries')}</Menu.Item>}
                  </Menu.Dropdown>
                </Menu>
              )}
            </div>
          </>
        ) : (
          <div className={classes.metaRow}>{t('guide.noSelection')}</div>
        )}
      </div>
      {playing ? (
        <MiniPlayer />
      ) : (
        <div className={classes.art} aria-hidden>
          programme art
        </div>
      )}
    </div>
  );
});
