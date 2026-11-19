import { Badge } from '@mantine/core';
import { IconHistory, IconLock } from '@tabler/icons-react';
import { t, type MessageKey } from '../../lib/i18n';
import { Icon } from '../Icon/Icon';
import classes from './StatusBadge.module.css';

export type StatusBadgeKind = 'live' | 'rec' | 'catchup' | 'new' | 'hd' | '4k' | 'radio' | 'drm';

const LABEL: Record<StatusBadgeKind, MessageKey> = {
  live: 'badge.live',
  rec: 'badge.rec',
  catchup: 'badge.catchup',
  new: 'badge.new',
  hd: 'badge.hd',
  '4k': 'badge.4k',
  radio: 'badge.radio',
  drm: 'badge.drm',
};

const TONE: Record<StatusBadgeKind, string> = {
  live: classes.live ?? '',
  rec: classes.live ?? '',
  catchup: classes.catchup ?? '',
  new: classes.outline ?? '',
  hd: classes.outline ?? '',
  '4k': classes.outline ?? '',
  radio: classes.outline ?? '',
  drm: classes.drm ?? '',
};

export interface StatusBadgeProps {
  kind: StatusBadgeKind;
}

export function StatusBadge({ kind }: StatusBadgeProps) {
  return (
    <Badge unstyled={false} className={`${classes.root} ${TONE[kind]}`} variant="transparent" data-kind={kind}>
      <span className={classes.label}>
        {(kind === 'live' || kind === 'rec') && <span className={classes.dot} aria-hidden />}
        {kind === 'catchup' && <Icon icon={IconHistory} size={14} />}
        {kind === 'drm' && <Icon icon={IconLock} size={14} />}
        {t(LABEL[kind])}
      </span>
    </Badge>
  );
}
