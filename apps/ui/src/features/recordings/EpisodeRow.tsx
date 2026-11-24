import { useState } from 'react';
import { Menu, UnstyledButton } from '@mantine/core';
import { IconDots, IconTrash } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import type { Recording } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { ProgrammeArt } from '../../components/ProgrammeArt/ProgrammeArt';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { episodeLine } from '../../lib/episode';
import { t, type MessageKey } from '../../lib/i18n';
import { dayLabel, formatClock, formatRange, formatSpan, progress } from '../../lib/time';
import { openInfo } from '../../store/info';
import { ChannelMark } from './ChannelMark';
import { isNew, watchedFraction } from './shows';
import classes from './Recordings.module.css';

export type Run = (fn: () => Promise<unknown>) => void;

/** Label of the play button for a recording, or null when there is nothing to play. */
export function playLabel(r: Recording): MessageKey | null {
  if (!r.playable) return null;
  if (r.status === 'recording') return 'rec.watch';
  if (r.status === 'failed') return 'rec.playPartial';
  return r.positionSecs != null ? 'rec.resume' : 'rec.play';
}

function metaLine(r: Recording, now: number): string {
  if (r.status === 'recording') return t('rec.meta.recordingNow', { range: formatRange(r.startUtc, r.endUtc) });
  const day = dayLabel(r.startUtc, now);
  if (r.status === 'failed') return r.error ? `${day} · ${r.error}` : day;
  const base = `${day} · ${formatSpan(r.durationSecs)}`;
  if (r.watched) return t('rec.meta.watched', { base });
  if (r.positionSecs != null) return t('rec.meta.left', { base, left: formatSpan(r.durationSecs - r.positionSecs) });
  return base;
}

export interface EpisodeRowProps {
  r: Recording;
  now: number;
  run: Run;
  busy: boolean;
  onPlay: (r: Recording) => void;
  /** Storybook: forced state props on the row. */
  demo?: Record<string, unknown>;
}

/** One recording of a show: thumbnail with progress, episode line + badges, channel and state, actions. */
export function EpisodeRow({ r, now, run, busy, onPlay, demo }: EpisodeRowProps) {
  const [confirming, setConfirming] = useState(false);
  const live = r.status === 'recording';
  const fraction = live ? progress(r.startUtc, r.endUtc, now) : watchedFraction(r);
  const play = playLabel(r);
  const label = episodeLine(r) ?? r.title;
  return (
    <div className={classes.episode} role="listitem" {...demo}>
      <ProgrammeArt programmeId={r.programmeId} channelId={r.channelId} className={`${classes.thumb} ${classes.stripes}`}>
        {live && (
          <span className={classes.thumbBadge}>
            <StatusBadge kind="rec" />
          </span>
        )}
        {fraction != null && (
          <span className={classes.thumbTrack} aria-hidden>
            <span className={classes.thumbFill} style={{ width: `${Math.round(fraction * 100)}%` }} />
          </span>
        )}
      </ProgrammeArt>
      <div className={classes.epMain}>
        <div className={classes.epTitleRow}>
          <UnstyledButton className={classes.epTitle} onClick={() => openInfo({ kind: 'recording', recording: r })}>
            {label}
          </UnstyledButton>
          {isNew(r) && <StatusBadge kind="new" />}
          {r.status === 'failed' && <StatusBadge kind="interrupted" />}
        </div>
        <div className={classes.epMeta}>
          <ChannelMark channel={{ id: r.channelId, name: r.channelName, logo: r.channelLogo }} size="xs" />
          <span className={classes.ellipsis} title={`${r.channelName} · ${metaLine(r, now)}`}>
            {r.channelName} · {metaLine(r, now)}
          </span>
        </div>
      </div>
      <div className={classes.actions}>
        {confirming ? (
          <>
            <span className={classes.confirm}>{t('rec.confirmDeleteShort')}</span>
            <DLButton variant="danger" size="sm" disabled={busy} onClick={() => run(() => invoke('delete_recording', { id: r.id }))}>
              {t('rec.confirmYes')}
            </DLButton>
            <DLButton variant="default" size="sm" onClick={() => setConfirming(false)}>
              {t('rec.confirmNo')}
            </DLButton>
          </>
        ) : (
          <>
            {play && (
              <DLButton variant="default" size="sm" onClick={() => onPlay(r)}>
                {t(play)}
              </DLButton>
            )}
            {live ? (
              <>
                <DLButton variant="danger" size="sm" disabled={busy} onClick={() => run(() => invoke('stop_recording', { id: r.id }))}>
                  {t('rec.stop')}
                </DLButton>
                <Menu position="bottom-end">
                  <Menu.Target>
                    <DLIconButton label={t('rec.more')} icon={IconDots} size="sm" disabled={busy} />
                  </Menu.Target>
                  <Menu.Dropdown>
                    <Menu.Label>{t('rec.extend')}</Menu.Label>
                    {([15, 30, 60] as const).map((m) => (
                      <Menu.Item key={m} onClick={() => run(() => invoke('extend_recording', { id: r.id, minutes: m }))}>
                        {t(m === 15 ? 'rec.extend15' : m === 30 ? 'rec.extend30' : 'rec.extend60')}
                      </Menu.Item>
                    ))}
                  </Menu.Dropdown>
                </Menu>
              </>
            ) : (
              <DLIconButton label={t('rec.deleteRecording')} icon={IconTrash} size="sm" disabled={busy} onClick={() => setConfirming(true)} />
            )}
          </>
        )}
      </div>
    </div>
  );
}

/** An upcoming airing inside a show: time + day, episode line, channel · until, Skip (rule) or Cancel. */
export function ShowUpcomingRow({ r, now, run, busy, ruled }: { r: Recording; now: number; run: Run; busy: boolean; ruled: boolean }) {
  return (
    <div className={classes.episode} role="listitem">
      <div>
        <div className={classes.upTime}>{formatClock(r.startUtc)}</div>
        <div className={classes.dim}>{dayLabel(r.startUtc, now)}</div>
      </div>
      <div className={classes.epMain}>
        <span className={classes.epTitleText}>{episodeLine(r) ?? r.title}</span>
        <div className={classes.epMeta}>
          <ChannelMark channel={{ id: r.channelId, name: r.channelName, logo: r.channelLogo }} size="xs" />
          <span className={classes.ellipsis}>
            {r.channelName} · {t('rec.until', { time: formatClock(r.endUtc) })}
          </span>
        </div>
      </div>
      <div className={classes.actions}>
        <DLButton variant="default" size="sm" disabled={busy} onClick={() => run(() => invoke('cancel_recording', { id: r.id }))}>
          {t(ruled ? 'rec.skip' : 'rec.cancel')}
        </DLButton>
      </div>
    </div>
  );
}
