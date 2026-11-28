import { UnstyledButton } from '@mantine/core';
import { IconCheck, IconX } from '@tabler/icons-react';
import { Icon } from '../../components/Icon/Icon';
import { t } from '../../lib/i18n';
import classes from './Sports.module.css';

export interface FollowChipProps {
  label: string;
  /** Abbreviation shown before the name in Geist Mono ("CHI"). */
  abbr?: string;
  onRemove: () => void;
  disabled?: boolean;
  /** Storybook: force a pseudo state on the remove button (see StateRow). */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** Something followed, in the Settings → Sports list: a static chip with its own remove (×) button. */
export function FollowChip({ label, abbr, onRemove, disabled, ...demo }: FollowChipProps) {
  return (
    <span className={classes.followChip} data-disabled={disabled || undefined}>
      {abbr && <span className={classes.abbr}>{abbr}</span>}
      <span>{label}</span>
      <UnstyledButton className={classes.chipRemove} aria-label={t('sports.following.remove', { name: label })} disabled={disabled} onClick={onRemove} {...demo}>
        <Icon icon={IconX} size={14} />
      </UnstyledButton>
    </span>
  );
}

export interface PickerTileProps {
  label: string;
  abbr?: string;
  selected: boolean;
  onToggle: () => void;
  disabled?: boolean;
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** A checkbox tile in the Add teams dialog: one team, or one session of a series. Selected = accent tint. */
export function PickerTile({ label, abbr, selected, onToggle, disabled, ...demo }: PickerTileProps) {
  return (
    <UnstyledButton className={classes.tile} aria-pressed={selected} disabled={disabled} onClick={onToggle} {...demo}>
      <span className={classes.tileBox} aria-hidden>
        {selected && <Icon icon={IconCheck} size={14} />}
      </span>
      {abbr !== undefined && <span className={classes.tileAbbr}>{abbr}</span>}
      <span className={classes.tileName}>{label}</span>
    </UnstyledButton>
  );
}
