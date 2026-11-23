import { Menu } from '@mantine/core';
import type { Programme, RecordKind } from '../../api/types';
import { formatClock } from '../../lib/time';
import { t } from '../../lib/i18n';

/** Record items for a channel row's now/next programmes (shared by the row's record button and its context menu). */
export function RecordItems({
  recordable,
  current,
  offline,
  onRecord,
  onDetails,
}: {
  recordable: Programme[];
  current: Programme | null;
  offline: boolean;
  onRecord?: (programmeId: string, kind: RecordKind) => void;
  onDetails?: () => void;
}) {
  return (
    <>
      {offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
      {recordable.map((x) => (
        <div key={x.id}>
          <Menu.Label>
            {t(x === current ? 'channels.record.now' : 'channels.record.next', { time: formatClock(x.startUtc), title: x.title ?? '' })}
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
    </>
  );
}
