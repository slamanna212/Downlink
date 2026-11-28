import { useState } from 'react';
import { Modal, UnstyledButton } from '@mantine/core';
import { IconStack2 } from '@tabler/icons-react';
import type { Channel, SportsCatalog, SportsMatch } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { t } from '../../lib/i18n';
import { useDLFocusable } from '../../lib/spatial';
import { dayLabel, formatClock, formatSpan, progress } from '../../lib/time';
import { followLabel, matchStart, onAirNow } from './follow';
import classes from './Sports.module.css';

export interface SportsMatchCardProps {
  match: SportsMatch;
  catalog: SportsCatalog | undefined;
  now: number;
  onSelect: (m: SportsMatch) => void;
  /** Plays one of the channels showing this event (`match.channel` or one of `match.alternates`). */
  onSelectChannel?: (c: Channel) => void;
  disabled?: boolean;
  /** Storybook: force a pseudo state (see StateRow). */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** The title a match shows: the programme's, or the channel name for event channels. */
export const matchTitle = (m: SportsMatch): string => m.programme?.title || m.channel.name;

const channelLine = (c: Channel): string => [c.number, c.name].filter((x) => x != null && x !== '').join(' · ');

/**
 * One followed match as a time-first ticket: a time column (badge + time left while on air, else the
 * start time and day), then the title and a meta line (what you follow, session, channel). The home
 * "Your sports" row and the settings preview share it (SPORTS §3). On air: plays the channel.
 * Upcoming: opens the programme details, as the guide and search do.
 */
export function SportsMatchCard({ match: m, catalog, now, onSelect, onSelectChannel, disabled, ...demo }: SportsMatchCardProps) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: () => !disabled && onSelect(m), focusable: !disabled });
  const p = m.programme;
  const onAir = onAirNow(m, now);
  const start = matchStart(m) ?? now;
  const title = matchTitle(m);
  const meta = [followLabel(m, catalog), m.session ? t(`sports.session.${m.session}`) : null, p ? channelLine(m.channel) : m.channel.number != null ? String(m.channel.number) : null]
    .filter((x): x is string => !!x)
    .join(' · ');
  const hasStreams = m.alternates.length > 0 && !!onSelectChannel;
  return (
    <div className={classes.cardWrap}>
      <UnstyledButton
        ref={ref}
        className={classes.card}
        disabled={disabled}
        onClick={() => onSelect(m)}
        aria-label={t(onAir ? 'sports.card.watch' : 'sports.card.details', { title: p?.subtitle ? `${title}: ${p.subtitle}` : title, channel: m.channel.name })}
        data-on-air={onAir || undefined}
        data-streams={hasStreams || undefined}
        {...focusProps}
        {...demo}
      >
        <span className={classes.timeCol}>
          {onAir ? (
            <>
              <span className={classes.timeBadge}>
                <StatusBadge kind={!m.live ? 'replay' : m.liveInferred ? 'likelyLiveShort' : 'live'} />
              </span>
              <span className={classes.timeSub}>{p ? t('sports.card.left', { span: formatSpan(p.endUtc - now) }) : t('sports.onToday')}</span>
            </>
          ) : (
            <>
              <span className={classes.timeStart}>{formatClock(start)}</span>
              <span className={classes.timeSub}>{dayLabel(start, now)}</span>
            </>
          )}
        </span>
        <span className={classes.cardBody}>
          <span className={classes.headline}>
            <span className={classes.cardTitle}>
              {title}
              {p?.subtitle && <span className={classes.cardSub}> · {p.subtitle}</span>}
            </span>
            {!m.live && !onAir && <StatusBadge kind="replay" />}
          </span>
          <span className={classes.cardMeta}>
            <span className={classes.metaLogo}>
              <ChannelLogo channelId={m.channel.id} name={m.channel.name} src={m.channel.logo} density="compact" bare fill />
            </span>
            <span className={classes.metaText}>{meta}</span>
          </span>
        </span>
        {p && onAir && (
          <span className={classes.track} aria-hidden>
            <span className={classes.fill} style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
          </span>
        )}
      </UnstyledButton>
      {/* Outside the card button so there are no nested interactive controls. */}
      {hasStreams && (
        <span className={classes.corner}>
          <StreamPicker match={m} title={title} now={now} disabled={disabled} onSelectChannel={onSelectChannel} />
        </span>
      )}
    </div>
  );
}

/** "+2" button: a dialog listing every channel showing the same event, the guide's pick first. Picking plays it. */
function StreamPicker({ match: m, title, now, disabled, onSelectChannel }: { match: SportsMatch; title: string; now: number; disabled?: boolean; onSelectChannel: (c: Channel) => void }) {
  const [open, setOpen] = useState(false);
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ focusable: !disabled, onEnterPress: () => setOpen(true) });
  const start = matchStart(m) ?? now;
  const when = onAirNow(m, now)
    ? m.programme
      ? t('sports.card.left', { span: formatSpan(m.programme.endUtc - now) })
      : t('sports.onToday')
    : t('sports.startsAt', { day: dayLabel(start, now), time: formatClock(start) });
  const channels = [m.channel, ...m.alternates];
  return (
    <>
      <DLButton
        ref={ref}
        size="xs"
        variant="default"
        icon={IconStack2}
        disabled={disabled}
        aria-label={t('sports.card.alternates.aria', { count: m.alternates.length })}
        onClick={() => setOpen(true)}
        {...focusProps}
      >
        {t('sports.card.alternates', { count: m.alternates.length })}
      </DLButton>
      <Modal
        opened={open}
        onClose={() => setOpen(false)}
        size="md"
        withinPortal
        title={
          <span className={classes.streamsHead}>
            <span className={classes.streamsTitle}>{t('sports.streams.title')}</span>
            <span className={classes.streamsSub}>{title} · {when}</span>
          </span>
        }
      >
        <div className={classes.streams} role="list">
          {channels.map((c, i) => (
            <UnstyledButton
              key={c.id}
              role="listitem"
              className={classes.streamRow}
              data-autofocus={i === 0 || undefined}
              onClick={() => {
                setOpen(false);
                onSelectChannel(c);
              }}
            >
              <span className={classes.streamLogo}>
                <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" bare fill />
              </span>
              <span className={classes.streamText}>
                <span className={classes.streamName}>{channelLine(c)}</span>
                <span className={classes.streamSub}>{i === 0 ? t('sports.streams.listed') : t('sports.streams.same')}</span>
              </span>
              {c.quality && <StatusBadge kind="cert" label={c.quality} />}
            </UnstyledButton>
          ))}
        </div>
      </Modal>
    </>
  );
}

/** Loading placeholder in the card's shape. */
export function SportsMatchCardSkeleton() {
  return <span className={`${classes.card} ${classes.skeleton}`} aria-hidden />;
}
