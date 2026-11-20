import { forwardRef, useState, type CSSProperties } from 'react';
import { Menu } from '@mantine/core';
import { IconHistory, IconPlayerRecord, IconStar, IconVolume } from '@tabler/icons-react';
import type { Channel, NowNext, Programme, RecordKind } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { formatClock, formatRange, progress } from '../../lib/time';
import { t } from '../../lib/i18n';
import classes from './Channels.module.css';

export interface ChannelRowProps {
  channel: Channel;
  nowNext: NowNext | undefined;
  now: number;
  active: boolean;
  playing: boolean;
  style: CSSProperties;
  onPlay: () => void;
  onToggleFavourite: () => void;
  onFocusRow: () => void;
  /** DVR cap on: show the record menu (button + right-click). */
  canRecord?: boolean;
  /** Source offline: record items stay visible but disabled. */
  offline?: boolean;
  onRecord?: (programmeId: string, kind: RecordKind) => void;
  onDetails?: () => void;
  'data-index': number;
}

const fmtNumber = (n: number | null) => (n == null ? '' : String(n));

export const ChannelRow = forwardRef<HTMLDivElement, ChannelRowProps>(function ChannelRow(
  { channel, nowNext, now, active, playing, style, onPlay, onToggleFavourite, onFocusRow, canRecord = false, offline = false, onRecord, onDetails, 'data-index': dataIndex },
  ref,
) {
  const [recordOpen, setRecordOpen] = useState(false);
  const p = nowNext?.now ?? null;
  const next = nowNext?.next ?? null;
  const recordable = [p, next].filter((x): x is Programme => x != null && x.endUtc > now);
  const showRecord = canRecord && onRecord != null && (recordable.length > 0 || onDetails != null);
  const label = t('channels.row', {
    number: fmtNumber(channel.number),
    name: channel.name,
    now: p?.title ?? t('channels.noGuide'),
    next: next?.title ?? t('channels.noGuide'),
  });
  return (
    <div
      ref={ref}
      role="listitem"
      aria-current={active ? 'true' : undefined}
      aria-label={label}
      tabIndex={active ? 0 : -1}
      className={classes.row}
      style={style}
      data-playing={playing || undefined}
      data-focusable
      data-index={dataIndex}
      onClick={onPlay}
      onFocus={onFocusRow}
      onContextMenu={
        showRecord
          ? (e) => {
              e.preventDefault();
              setRecordOpen(true);
            }
          : undefined
      }
    >
      <ChannelLogo channelId={channel.id} name={channel.name} src={channel.logo} />
      <span className={classes.number}>{fmtNumber(channel.number)}</span>
      <span className={classes.name} data-playing={playing || undefined}>
        {playing && <Icon icon={IconVolume} size={16} />}
        <span className={classes.nameText}>{channel.name}</span>
        {(channel.quality === 'HD' || channel.quality === 'FHD') && <StatusBadge kind="hd" />}
        {channel.quality === '4K' && <StatusBadge kind="4k" />}
        {channel.isRadio && <StatusBadge kind="radio" />}
        {channel.hasDrm && <StatusBadge kind="drm" />}
      </span>
      <span className={classes.now}>
        {p ? (
          <>
            <span className={classes.nowTitle}>
              <span>{p.title}</span>
              <span className={classes.nowTime}>{formatRange(p.startUtc, p.endUtc)}</span>
            </span>
            <span className={classes.track} role="progressbar" aria-label={p.title ?? ''} aria-valuenow={Math.round(progress(p.startUtc, p.endUtc, now) * 100)} aria-valuemin={0} aria-valuemax={100}>
              <span className={classes.fill} style={{ display: 'block', width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
            </span>
          </>
        ) : (
          <span className={classes.nowTime}>{t('channels.noGuide')}</span>
        )}
      </span>
      <span className={classes.next}>{next ? `${formatClock(next.startUtc)} ${next.title ?? ''}` : ''}</span>
      <span className={classes.end}>
        {channel.isCatchup && channel.catchupDays > 0 && (
          <span className={classes.catchup} title={t('channels.catchupDays', { days: channel.catchupDays })}>
            <Icon icon={IconHistory} size={14} />
            {channel.catchupDays}d
          </span>
        )}
        {showRecord && (
          <span onClick={(e) => e.stopPropagation()} onContextMenu={(e) => e.stopPropagation()}>
            <Menu opened={recordOpen} onChange={setRecordOpen} position="bottom-end" withinPortal>
              <Menu.Target>
                <DLIconButton label={t('guide.record')} icon={IconPlayerRecord} size="sm" tabIndex={-1} />
              </Menu.Target>
              <Menu.Dropdown>
                {offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
                {recordable.map((x) => (
                  <div key={x.id}>
                    <Menu.Label>
                      {t(x === p ? 'channels.record.now' : 'channels.record.next', { time: formatClock(x.startUtc), title: x.title ?? '' })}
                    </Menu.Label>
                    <Menu.Item disabled={offline || x.isScheduled} onClick={() => onRecord?.(x.id, 'once')}>
                      {x.isScheduled ? t('guide.recording') : t('guide.recordOnce')}
                    </Menu.Item>
                    <Menu.Item disabled={offline} onClick={() => onRecord?.(x.id, 'series')}>{t('guide.recordSeries')}</Menu.Item>
                  </div>
                ))}
                {onDetails && (
                  <>
                    {recordable.length > 0 && <Menu.Divider />}
                    <Menu.Item onClick={onDetails}>{t('channels.record.schedule')}</Menu.Item>
                  </>
                )}
              </Menu.Dropdown>
            </Menu>
          </span>
        )}
        <span onClick={(e) => e.stopPropagation()}>
          <DLIconButton
            label={t(channel.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')}
            icon={IconStar}
            size="sm"
            pressed={channel.isFavourite}
            filledGlyph={channel.isFavourite}
            onClick={onToggleFavourite}
            tabIndex={-1}
          />
        </span>
      </span>
    </div>
  );
});
