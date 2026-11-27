import { Loader, UnstyledButton } from '@mantine/core';
import classes from './Sports.module.css';

export interface TeamChipProps {
  label: string;
  /** Abbreviation shown before the name in Geist Mono ("CHI"). */
  abbr?: string;
  pressed: boolean;
  onToggle: () => void;
  disabled?: boolean;
  loading?: boolean;
  /** Storybook: force a pseudo state (see StateRow). */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** A toggle chip for the team picker (and its "All games" choice). Selected = accent tint (DESIGN §2). */
export function TeamChip({ label, abbr, pressed, onToggle, disabled, loading, ...demo }: TeamChipProps) {
  return (
    <UnstyledButton
      className={classes.chip}
      aria-pressed={pressed}
      aria-busy={loading || undefined}
      data-loading={loading || undefined}
      disabled={disabled}
      onClick={() => !loading && onToggle()}
      {...demo}
    >
      {loading ? <Loader size={12} color="currentColor" /> : abbr && <span className={classes.abbr}>{abbr}</span>}
      <span>{label}</span>
    </UnstyledButton>
  );
}
