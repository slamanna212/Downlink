import { memo, useEffect, useState } from 'react';
import { Menu } from '@mantine/core';
import { IconChevronDown, IconStar } from '@tabler/icons-react';
import type { Channel, Programme } from '../../api/types';
import { dl } from '../../theme';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { dayLabel, formatRange } from '../../lib/time';
import { t } from '../../lib/i18n';
import { enterAction, type BlockState } from './guideModel';
import classes from './Guide.module.css';

export interface PreviewProps {
  programme: Programme | null;
  channel: Channel | undefined;
  state: BlockState | null;
  canRecord: boolean;
  recordOpen: boolean;
  onRecordOpen: (open: boolean) => void;
  onRecord: (kind: 'once' | 'series') => void;
  onPrimary: () => void;
  onFromStart: () => void;
  onDetails: () => void;
  onFavourite: () => void;
}

/**
 * Preview pane. It follows focus after a 300 ms dwell. Everything shown is already in memory:
 * dwelling never calls a play command or touches a stream endpoint (CLAUDE rule 4).
 */
export const GuidePreview = memo(function GuidePreview({ programme, channel, state, canRecord, recordOpen, onRecordOpen, onRecord, onPrimary, onFromStart, onDetails, onFavourite }: PreviewProps) {
  const [shown, setShown] = useState<{ p: Programme | null; ch: Channel | undefined; state: BlockState | null }>({ p: programme, ch: channel, state });
  useEffect(() => {
    const id = setTimeout(() => setShown({ p: programme, ch: channel, state }), dl.previewDwellMs);
    return () => clearTimeout(id);
  }, [programme, channel, state]);

  const { p, ch, state: st } = shown;
  const action = st ? enterAction(st) : 'none';
  const primaryLabel = action === 'play-live' ? t('guide.watchLive') : action === 'play-catchup' ? t('guide.playFromStart') : action === 'details' ? t('guide.details') : null;
  return (
    <div className={classes.preview}>
      <div className={classes.art} aria-hidden>
        programme art
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
            <p className={classes.desc}>{p.description}</p>
            <div className={classes.actions}>
              {primaryLabel && (
                <DLButton variant="filled" size="lg" onClick={action === 'details' ? onDetails : onPrimary}>
                  {primaryLabel}
                </DLButton>
              )}
              {st === 'now' && (
                <DLButton variant="default" size="lg" onClick={onFromStart}>
                  {t('guide.fromStart')}
                </DLButton>
              )}
              <DLButton variant="default" size="lg" onClick={onDetails}>
                {t('guide.channelDetails')}
              </DLButton>
              <DLIconButton label={t(ch.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')} icon={IconStar} size="lg" variant="default" pressed={ch.isFavourite} filledGlyph={ch.isFavourite} onClick={onFavourite} />
              {canRecord && st !== 'past-archive' && st !== 'past-none' && (
                <Menu opened={recordOpen} onChange={onRecordOpen} position="bottom-start">
                  <Menu.Target>
                    <DLButton variant="default" size="lg">
                      {t('guide.record')} <IconChevronDown size={14} />
                    </DLButton>
                  </Menu.Target>
                  <Menu.Dropdown>
                    <Menu.Item onClick={() => onRecord('once')}>{t('guide.recordOnce')}</Menu.Item>
                    <Menu.Item onClick={() => onRecord('series')}>{t('guide.recordSeries')}</Menu.Item>
                  </Menu.Dropdown>
                </Menu>
              )}
            </div>
          </>
        ) : (
          <div className={classes.metaRow}>{t('guide.noSelection')}</div>
        )}
      </div>
    </div>
  );
});
