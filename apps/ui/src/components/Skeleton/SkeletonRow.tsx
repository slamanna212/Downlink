import { useEffect, useState } from 'react';
import { Skeleton, Stack, Text } from '@mantine/core';
import { dl } from '../../theme';
import { useUiStore } from '../../store/ui';
import classes from './SkeletonRow.module.css';

/** A channel-shaped placeholder in the real row grid (logo, number, name, now, next). */
export function SkeletonRow() {
  const density = useUiStore((s) => s.density);
  const [logoW, logoH] = dl.density[density].logo;
  return (
    <div className={classes.row} style={{ height: dl.density[density].channelRow }} aria-hidden>
      <Skeleton className={classes.block} width={logoW} height={logoH} />
      <Skeleton className={classes.block} width={32} height={12} />
      <Skeleton className={classes.block} height={14} width="70%" />
      <Skeleton className={classes.block} height={12} width="60%" />
      <Skeleton className={classes.block} height={12} width="50%" />
    </div>
  );
}

/** True once `delayMs` has passed while mounted: waits > 300 ms say what they wait on. */
export function useAfterDelay(delayMs = dl.previewDwellMs): boolean {
  const [late, setLate] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setLate(true), delayMs);
    return () => clearTimeout(id);
  }, [delayMs]);
  return late;
}

export interface SkeletonListProps {
  rows?: number;
  /** Shown after 300 ms, e.g. t('loading.channels'). */
  label?: string;
}

export function SkeletonList({ rows = 8, label }: SkeletonListProps) {
  const late = useAfterDelay(300);
  return (
    <Stack gap={0} aria-busy="true">
      {label && late && (
        <Text size="sm" c="dimmed" className={classes.label} role="status">
          {label}
        </Text>
      )}
      {Array.from({ length: rows }, (_, i) => (
        <SkeletonRow key={i} />
      ))}
    </Stack>
  );
}
