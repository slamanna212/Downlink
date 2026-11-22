import { Badge } from '@mantine/core';
import { IconHistory, IconLock } from '@tabler/icons-react';
import { t, type MessageKey } from '../../lib/i18n';
import { Icon } from '../Icon/Icon';
import classes from './StatusBadge.module.css';

export type StatusBadgeKind = 'live' | 'rec' | 'catchup' | 'new' | 'hd' | '4k' | 'radio' | 'drm' | 'premiere' | 'finale' | 'repeat' | 'cert';

const LABEL: Record<StatusBadgeKind, MessageKey> = {
  live: 'badge.live',
  rec: 'badge.rec',
  catchup: 'badge.catchup',
  new: 'badge.new',
  hd: 'badge.hd',
  '4k': 'badge.4k',
  radio: 'badge.radio',
  drm: 'badge.drm',
  premiere: 'badge.premiere',
  finale: 'badge.finale',
  repeat: 'badge.repeat',
  cert: 'badge.hd', // always replaced by `label`
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
  premiere: classes.outline ?? '',
  finale: classes.outline ?? '',
  repeat: classes.drm ?? '',
  cert: classes.outline ?? '',
};

export interface StatusBadgeProps {
  kind: StatusBadgeKind;
  /** Text for `cert` (a content rating such as `TV-14`); shown as sent, never translated. */
  label?: string;
}

export function StatusBadge({ kind, label }: StatusBadgeProps) {
  return (
    <Badge unstyled={false} className={`${classes.root} ${TONE[kind]}`} variant="transparent" data-kind={kind}>
      <span className={classes.label}>
        {(kind === 'live' || kind === 'rec') && <span className={classes.dot} aria-hidden />}
        {kind === 'catchup' && <Icon icon={IconHistory} size={14} />}
        {kind === 'drm' && <Icon icon={IconLock} size={14} />}
        {kind === 'cert' ? label : t(LABEL[kind])}
      </span>
    </Badge>
  );
}
