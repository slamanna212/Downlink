import { useState } from 'react';
import { Menu } from '@mantine/core';
import { IconChevronDown, IconRepeat } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import type { Recording } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Icon } from '../../components/Icon/Icon';
import { ProgrammeArt } from '../../components/ProgrammeArt/ProgrammeArt';
import { t } from '../../lib/i18n';
import { ChannelStack } from './ChannelMark';
import { EpisodeRow, ShowUpcomingRow, type Run } from './EpisodeRow';
import type { Show } from './shows';
import classes from './Recordings.module.css';

/** The hero's main action: watch the running one, resume a part-watched one, or play the newest. */
export function heroAction(show: Show): { label: 'rec.watchLive' | 'rec.resume' | 'rec.playLatest'; r: Recording } | null {
  const live = show.recordings.find((r) => r.status === 'recording');
  if (live) return { label: 'rec.watchLive', r: live };
  const partial = show.recordings.find((r) => r.playable && r.positionSecs != null);
  if (partial) return { label: 'rec.resume', r: partial };
  const latest = show.recordings.find((r) => r.playable);
  return latest ? { label: 'rec.playLatest', r: latest } : null;
}

function channelLine(show: Show): string {
  const names = show.channels.map((c) => c.name);
  if (names.length > 1) return t('rec.channels', { n: names.length, names: names.join(', ') });
  return names[0] ?? (show.rule?.channelName || t('rec.anyChannel'));
}

function ruleLine(show: Show): string {
  const { rule } = show;
  if (!rule) return t('rec.noRule');
  const where = rule.channelName ? t('rec.channelOnly', { channel: rule.channelName }) : t('rec.anyChannel');
  return t('rec.ruleLine', { schedule: rule.schedule, where });
}

export interface ShowDetailProps {
  show: Show;
  now: number;
  run: Run;
  busy: boolean;
  onPlay: (r: Recording) => void;
}

export function ShowDetail({ show, now, run, busy, onPlay }: ShowDetailProps) {
  const [confirmRule, setConfirmRule] = useState(false);
  const action = heroAction(show);
  const artId = [...show.recordings, ...show.upcoming].find((r) => r.programmeId)?.programmeId ?? null;
  const seriesFrom = show.upcoming.find((r) => r.programmeId)?.programmeId ?? null;
  const { rule } = show;

  return (
    <div className={classes.detail}>
      <ProgrammeArt programmeId={artId} channelId={show.channels[0]?.id} fit="backdrop" className={`${classes.hero} ${classes.stripes}`}>
        <div className={classes.heroScrim} aria-hidden />
        <div className={classes.heroBody}>
          <div className={classes.heroChannels}>
            {show.channels.length > 0 && <ChannelStack channels={show.channels} size="md" />}
            <span className={classes.ellipsis}>{channelLine(show)}</span>
          </div>
          <h2 className={classes.heroTitle}>{show.title}</h2>
          <div className={classes.heroRule}>
            <Icon icon={IconRepeat} size={16} />
            {ruleLine(show)}
          </div>
          <div className={classes.heroActions}>
            {confirmRule && rule ? (
              <>
                <span className={classes.confirm}>{t('rec.confirmRule', { title: rule.title })}</span>
                <DLButton
                  variant="danger"
                  size="md"
                  disabled={busy}
                  onClick={() => {
                    setConfirmRule(false);
                    run(() => invoke('delete_recording_rule', { id: rule.id }));
                  }}
                >
                  {t('rec.removeRule')}
                </DLButton>
                <DLButton variant="default" size="md" onClick={() => setConfirmRule(false)}>
                  {t('rec.confirmNo')}
                </DLButton>
              </>
            ) : (
              <>
                {action && (
                  <DLButton variant="filled" size="md" onClick={() => onPlay(action.r)}>
                    {t(action.label)}
                  </DLButton>
                )}
                {rule ? (
                  <Menu position="bottom-start">
                    <Menu.Target>
                      <DLButton variant="default" size="md" disabled={busy}>
                        {t('rec.editRule')} <IconChevronDown size={14} />
                      </DLButton>
                    </Menu.Target>
                    <Menu.Dropdown>
                      <Menu.Item color="live" onClick={() => setConfirmRule(true)}>
                        {t('rec.removeRule')}
                      </Menu.Item>
                    </Menu.Dropdown>
                  </Menu>
                ) : (
                  seriesFrom && (
                    <DLButton variant="default" size="md" disabled={busy} onClick={() => run(() => invoke('schedule_recording', { programmeId: seriesFrom, kind: 'series' }))}>
                      {t('rec.recordSeries')}
                    </DLButton>
                  )
                )}
              </>
            )}
          </div>
        </div>
      </ProgrammeArt>

      <section aria-labelledby="rec-episodes">
        <h3 className={classes.h3} id="rec-episodes">
          {t('rec.episodes')}
          <span className={classes.count}>{show.recordings.length}</span>
        </h3>
        {show.recordings.length === 0 ? (
          <p className={classes.msg}>{t(rule ? 'rec.nothingYet' : 'rec.list.none')}</p>
        ) : (
          <div role="list" aria-labelledby="rec-episodes">
            {show.recordings.map((r) => (
              <EpisodeRow key={r.id} r={r} now={now} run={run} busy={busy} onPlay={onPlay} />
            ))}
          </div>
        )}
      </section>

      {show.upcoming.length > 0 && (
        <section aria-labelledby="rec-show-up">
          <h3 className={classes.h3} id="rec-show-up">
            {t('rec.comingUp')}
            <span className={classes.count}>{show.upcoming.length}</span>
          </h3>
          <div role="list" aria-labelledby="rec-show-up">
            {show.upcoming.map((r) => (
              <ShowUpcomingRow key={r.id} r={r} now={now} run={run} busy={busy} ruled={!!rule} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
