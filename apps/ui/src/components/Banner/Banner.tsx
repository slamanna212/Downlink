import { Group, Text } from '@mantine/core';
import { IconAlertTriangle, IconCircleCheck, IconInfoCircle, IconWifiOff } from '@tabler/icons-react';
import { Icon, type TablerIcon } from '../Icon/Icon';
import { DLButton } from '../DLButton/DLButton';
import classes from './Banner.module.css';

export type BannerTone = 'info' | 'ok' | 'warn' | 'error';

const ICON: Record<BannerTone, TablerIcon> = {
  info: IconInfoCircle,
  ok: IconCircleCheck,
  warn: IconAlertTriangle,
  error: IconWifiOff,
};
const ICON_CLASS: Record<BannerTone, string | undefined> = {
  info: classes.iconInfo,
  ok: classes.iconOk,
  warn: classes.iconWarn,
  error: classes.iconError,
};

export interface BannerProps {
  tone?: BannerTone;
  /** One sentence. Banners are quiet and persistent, never modal. */
  message: string;
  /** One text action. */
  actionLabel?: string;
  onAction?: () => void;
}

export function Banner({ tone = 'info', message, actionLabel, onAction }: BannerProps) {
  return (
    <Group className={`${classes.root} ${classes[tone] ?? ''}`} gap="sm" wrap="nowrap" role={tone === 'error' ? 'alert' : 'status'}>
      <span className={ICON_CLASS[tone]}>
        <Icon icon={ICON[tone]} size={16} />
      </span>
      <Text size="sm" fw={500} className={classes.message}>
        {message}
      </Text>
      {actionLabel && (
        <DLButton variant="subtle" size="xs" onClick={onAction}>
          {actionLabel}
        </DLButton>
      )}
    </Group>
  );
}
