import { IconRepeat } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import type { Recording } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Icon } from '../../components/Icon/Icon';
import { episodeLine } from '../../lib/episode';
import { t } from '../../lib/i18n';
import { dayLabel, formatClock } from '../../lib/time';
import { ChannelMark } from './ChannelMark';
import type { Run } from './EpisodeRow';
import type { Show } from './shows';
import classes from './Recordings.module.css';

interface Item {
  r: Recording;
  show: Show;
}

/** Every scheduled recording across shows, by day. */
export function UpcomingView({ shows, now, run, busy }: { shows: Show[]; now: number; run: Run; busy: boolean }) {
  const items: Item[] = shows.flatMap((show) => show.upcoming.map((r) => ({ r, show }))).sort((a, b) => a.r.startUtc - b.r.startUtc);
  const days: { label: string; items: Item[] }[] = [];
  for (const it of items) {
    const label = dayLabel(it.r.startUtc, now);
    const last = days[days.length - 1];
    if (last?.label === label) last.items.push(it);
    else days.push({ label, items: [it] });
  }
  return (
    <div className={classes.detail}>
      <div>
        <h2 className={classes.h2}>{t('rec.comingUp')}</h2>
        <div className={classes.sub}>{t('rec.scheduled', { n: items.length })}</div>
      </div>
      {items.length === 0 && <p className={classes.msg}>{t('rec.empty.upcoming')}</p>}
      {days.map((d, i) => (
        <section key={d.label} aria-labelledby={`rec-day-${i}`}>
          <h3 className={classes.h3} id={`rec-day-${i}`}>
            {d.label}
          </h3>
          <div role="list" aria-labelledby={`rec-day-${i}`}>
            {d.items.map(({ r, show }) => (
              <div key={r.id} className={classes.upRow} role="listitem">
                <div>
                  <div className={classes.upClock}>{formatClock(r.startUtc)}</div>
                  <div className={classes.dim}>{t('rec.until', { time: formatClock(r.endUtc) })}</div>
                </div>
                <ChannelMark channel={{ id: r.channelId, name: r.channelName, logo: r.channelLogo }} size="lg" />
                <div className={classes.epMain}>
                  <span className={classes.epTitleText}>{show.title}</span>
                  <span className={classes.upLine}>
                    {episodeLine(r) ? (
                      <>
                        {episodeLine(r)} <span className={classes.dim}>· {r.channelName}</span>
                      </>
                    ) : (
                      <span className={classes.dim}>{r.channelName}</span>
                    )}
                  </span>
                </div>
                <span className={classes.rule}>
                  <Icon icon={IconRepeat} size={14} />
                  <span className={classes.ellipsis}>{show.rule?.schedule ?? r.ruleLabel ?? t('rec.once')}</span>
                </span>
                <div className={classes.actions}>
                  <DLButton variant="default" size="sm" disabled={busy} onClick={() => run(() => invoke('cancel_recording', { id: r.id }))}>
                    {t('rec.cancel')}
                  </DLButton>
                </div>
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
