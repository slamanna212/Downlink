import type { ReactNode } from 'react';
import { UnstyledButton } from '@mantine/core';
import { IconPin } from '@tabler/icons-react';
import type { Channel, SportsCatalog, SportsMatch } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { Icon } from '../../components/Icon/Icon';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { t } from '../../lib/i18n';
import { useDLFocusable } from '../../lib/spatial';
import { dayLabel, formatClock, minutesLeft, progress } from '../../lib/time';
import { followLabel, matchStart, onAirNow } from '../sports/follow';
import { matchTitle } from '../sports/SportsMatchCard';
import classes from './Command.module.css';

export interface RailRowProps {
  /** The match this row is for; null for a tile showing a channel that is not a followed game. */
  match: SportsMatch | null;
  /** Shown when there is no match (and as the logo source otherwise). */
  channel: Channel | undefined;
  catalog: SportsCatalog | undefined;
  now: number;
  /** Accessible name of the row's main button. */
  label: string;
  onSelect?: () => void;
  /** Marks the row for the tile that has keyboard focus on the wall. */
  active?: boolean;
  /** The tile is kept on screen (auto-fill leaves it alone). */
  pinned?: boolean;
  disabled?: boolean;
  /** Icon buttons on the right (outside the row button: no nested controls). */
  actions?: ReactNode;
  /** Storybook: force a pseudo state. */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/**
 * One line in the command center rail: logo, title, what it is for and when, live badge. The rail's
 * three sections (on screen, also live, up next) all use it with their own actions.
 */
export function RailRow({ match: m, channel, catalog, now, label, onSelect, active, pinned, disabled, actions, ...demo }: RailRowProps) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: () => !disabled && onSelect?.(), focusable: !disabled });
  const ch = m?.channel ?? channel;
  const p = m?.programme ?? null;
  const onAir = m ? onAirNow(m, now) : true;
  const start = m ? matchStart(m) : null;
  const when = !m
    ? ch?.number != null ? String(ch.number) : ''
    : onAir
      ? p ? t('home.minLeft', { min: minutesLeft(p.endUtc, now) }) : t('sports.onToday')
      : t('sports.startsAt', { day: dayLabel(start ?? now, now), time: formatClock(start ?? now) });
  return (
    <div className={classes.row} data-active={active || undefined}>
      <UnstyledButton ref={ref} className={classes.rowMain} disabled={disabled} onClick={onSelect} aria-label={label} {...focusProps} {...demo}>
        <span className={classes.rowLogo}>
          {ch && <ChannelLogo channelId={ch.id} name={ch.name} src={ch.logo} density="compact" bare fill />}
        </span>
        <span className={classes.rowText}>
          {/* The subtitle names the game ("Arsenal v Chelsea"); titles are often just the league. */}
          <span className={classes.rowTitle}>{m ? p?.subtitle || matchTitle(m) : ch?.name ?? ''}</span>
          <span className={classes.rowMeta}>
            {m ? [followLabel(m, catalog), m.session && t(`sports.session.${m.session}`), when].filter(Boolean).join(' · ') : when}
          </span>
        </span>
        {pinned && (
          <span className={classes.rowPin}>
            <Icon icon={IconPin} size={14} />
          </span>
        )}
        {m && (
          <span className={classes.rowBadge}>
            {!m.live ? <StatusBadge kind="replay" /> : onAir ? <StatusBadge kind={m.liveInferred ? 'likelyLive' : 'live'} /> : null}
          </span>
        )}
        {p && onAir && (
          <span className={classes.rowTrack} aria-hidden>
            <span style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
          </span>
        )}
      </UnstyledButton>
      {actions && <span className={classes.rowActions}>{actions}</span>}
    </div>
  );
}
