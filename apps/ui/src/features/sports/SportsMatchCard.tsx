import { Menu, UnstyledButton } from '@mantine/core';
import type { Channel, SportsCatalog, SportsMatch } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { t } from '../../lib/i18n';
import { useDLFocusable } from '../../lib/spatial';
import { dayLabel, formatClock, minutesLeft, progress } from '../../lib/time';
import { followLabel, matchStart, onAirNow } from './follow';
import classes from './Sports.module.css';

export interface SportsMatchCardProps {
  match: SportsMatch;
  catalog: SportsCatalog | undefined;
  now: number;
  onSelect: (m: SportsMatch) => void;
  /** Plays one of the other channels showing this event (`match.alternates`). */
  onSelectChannel?: (c: Channel) => void;
  disabled?: boolean;
  /** Storybook: force a pseudo state (see StateRow). */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** The title a match shows: the programme's, or the channel name for event channels. */
export const matchTitle = (m: SportsMatch): string => m.programme?.title || m.channel.name;

/**
 * One followed match: the home "Your sports" row and the settings preview share it (SPORTS §3).
 * On air: plays the channel. Upcoming: opens the programme details, as the guide and search do.
 */
export function SportsMatchCard({ match: m, catalog, now, onSelect, onSelectChannel, disabled, ...demo }: SportsMatchCardProps) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: () => !disabled && onSelect(m), focusable: !disabled });
  const p = m.programme;
  const onAir = onAirNow(m, now);
  const start = matchStart(m);
  const title = matchTitle(m);
  const label = followLabel(m, catalog);
  // Event channels already show their name as the title; the foot then only needs the number.
  const shownName = p ? m.channel.name : '';
  const name = [m.channel.number, shownName].filter((x) => x != null && x !== '').join(' · ');
  return (
    <div className={classes.cardWrap}>
    <UnstyledButton
      ref={ref}
      className={classes.card}
      disabled={disabled}
      onClick={() => onSelect(m)}
      aria-label={t(onAir ? 'sports.card.watch' : 'sports.card.details', { title: p?.subtitle ? `${title}: ${p.subtitle}` : title, channel: m.channel.name })}
      data-on-air={onAir || undefined}
      {...focusProps}
      {...demo}
    >
      <span className={classes.cardHead}>
        <span className={classes.cardLogo}>
          <ChannelLogo channelId={m.channel.id} name={m.channel.name} src={m.channel.logo} density="comfortable" bare fill />
        </span>
      </span>
      <span className={classes.cardTitle}>{title}</span>
      {p?.subtitle && <span className={classes.cardSub}>{p.subtitle}</span>}
      <span className={classes.cardLabel}>
        <span className={classes.labelText}>{label}</span>
        {/* Outline badge with translated text (the `cert` kind shows its label as given). */}
        {m.session && <StatusBadge kind="cert" label={t(`sports.session.${m.session}`)} />}
      </span>
      <span className={classes.cardFoot}>
        <span className={classes.channel}>{name}</span>
        {onAir && !p ? (
          <span className={classes.when}>{t('sports.onToday')}</span>
        ) : onAir && p ? (
          <span className={classes.when}>{t('home.minLeft', { min: minutesLeft(p.endUtc, now) })}</span>
        ) : (
          <span className={classes.when}>{t('sports.startsAt', { day: dayLabel(start ?? now, now), time: formatClock(start ?? now) })}</span>
        )}
      </span>
      {p && onAir && (
        <span className={classes.track} aria-hidden>
          <span className={classes.fill} style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
        </span>
      )}
    </UnstyledButton>
    {/* Outside the card button: badges plus the alternates menu (no nested interactive controls). */}
    <span className={classes.corner}>
      {m.alternates.length > 0 && onSelectChannel && (
        <AlternatesMenu channels={m.alternates} disabled={disabled} onSelectChannel={onSelectChannel} />
      )}
      <span className={classes.badges}>
        {!m.live ? <StatusBadge kind="replay" /> : onAir ? <StatusBadge kind={m.liveInferred ? 'likelyLive' : 'live'} /> : null}
      </span>
    </span>
    </div>
  );
}

/** "+2" button listing the other channels showing the same event, best first. */
function AlternatesMenu({ channels, disabled, onSelectChannel }: { channels: Channel[]; disabled?: boolean; onSelectChannel: (c: Channel) => void }) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ focusable: !disabled, onEnterPress: () => ref.current?.click() });
  return (
    <Menu position="bottom-end" withinPortal trapFocus returnFocus>
      <Menu.Target>
        <DLButton
          ref={ref}
          size="xs"
          variant="subtle"
          disabled={disabled}
          aria-label={t('sports.card.alternates.aria', { count: channels.length })}
          {...focusProps}
        >
          {t('sports.card.alternates', { count: channels.length })}
        </DLButton>
      </Menu.Target>
      <Menu.Dropdown aria-label={t('sports.card.alternates.menu')}>
        <Menu.Label>{t('sports.card.alternates.menu')}</Menu.Label>
        {channels.map((c) => (
          <Menu.Item
            key={c.id}
            leftSection={<span className={classes.menuLogo}><ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" bare fill /></span>}
            rightSection={c.quality ? <StatusBadge kind="cert" label={c.quality} /> : undefined}
            onClick={() => onSelectChannel(c)}
          >
            {[c.number, c.name].filter((x) => x != null && x !== '').join(' · ')}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

/** Loading placeholder in the card's shape. */
export function SportsMatchCardSkeleton() {
  return <span className={`${classes.card} ${classes.skeleton}`} aria-hidden />;
}
