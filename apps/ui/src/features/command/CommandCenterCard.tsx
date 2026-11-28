import type { ReactNode } from 'react';
import { UnstyledButton } from '@mantine/core';
import type { Channel } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { Icon, type TablerIcon } from '../../components/Icon/Icon';
import { useDLFocusable } from '../../lib/spatial';
import classes from './Command.module.css';

export interface CommandCenterCardProps {
  name: string;
  /** One line under the name: tile count, games live… */
  meta: string;
  /** Optional dim sentence (the built-in My sports card). */
  description?: string;
  /** Up to four channels shown as bare logos (saved centers). */
  channels?: Channel[];
  /** Glyph shown instead of the logos (built-in centers). */
  icon?: TablerIcon;
  /** Accessible name of the card button. */
  label: string;
  onSelect: () => void;
  /** Controls in the top-right corner, outside the card button (no nested controls). */
  corner?: ReactNode;
  disabled?: boolean;
  /** Storybook: force a pseudo state (see StateRow). */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** One command center on the Command center page: My sports first, then each saved wall. */
export function CommandCenterCard({ name, meta, description, channels = [], icon, label, onSelect, corner, disabled, ...demo }: CommandCenterCardProps) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: () => !disabled && onSelect(), focusable: !disabled });
  return (
    <div className={classes.cardWrap}>
      <UnstyledButton ref={ref} className={classes.card} disabled={disabled} onClick={onSelect} aria-label={label} {...focusProps} {...demo}>
        <span className={classes.cardPreview} aria-hidden>
          {icon ? (
            <span className={classes.cardIcon}>
              <Icon icon={icon} size={24} />
            </span>
          ) : (
            channels.slice(0, 4).map((c) => (
              <span key={c.id} className={classes.cardLogo}>
                <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" bare fill />
              </span>
            ))
          )}
        </span>
        <span className={classes.cardName}>{name}</span>
        {description && <span className={classes.cardDesc}>{description}</span>}
        <span className={classes.cardMeta}>{meta}</span>
      </UnstyledButton>
      {corner && <span className={classes.cardCorner}>{corner}</span>}
    </div>
  );
}

/** Loading placeholder in the card's shape. */
export function CommandCenterCardSkeleton() {
  return <span className={`${classes.card} ${classes.cardSkeleton}`} aria-hidden />;
}
