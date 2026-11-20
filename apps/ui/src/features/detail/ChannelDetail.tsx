import { useMemo } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Drawer, Menu, UnstyledButton } from '@mantine/core';
import { IconStar } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps, useChannels, useOffline } from '../../api/queries';
import type { Programme } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { addDays, dayLabel, formatClock, formatRange, minutesLeft, progress, startOfDay } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { t } from '../../lib/i18n';
import { useDetailStore } from '../../store/detail';
import { activateProgramme } from '../guide/activate';
import { blockState } from '../guide/guideModel';
import { playChannel } from '../player/actions';
import classes from './Detail.module.css';

/** Channel detail drawer: now/next, catch-up window, schedule, record, favourite. */
export function ChannelDetail() {
  const open = useDetailStore((s) => s.open);
  const close = useDetailStore((s) => s.close);
  const show = useDetailStore((s) => s.show);
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { caps } = useCaps();
  const offline = useOffline();
  const now = useNowEpoch(30_000);
  const channels = useChannels({});
  const channel = useMemo(() => channels.data?.find((c) => c.id === open?.channelId), [channels.data, open?.channelId]);

  // Schedule: from the earlier of (archive window start, today) to end of tomorrow.
  const range = useMemo(() => {
    // From 3 h ago (catch-up-able programmes) to the end of tomorrow.
    return { start: Math.floor((now - 3 * 3600) / 1800) * 1800, end: addDays(startOfDay(now), 2) };
  }, [now]);
  const schedule = useQuery({
    queryKey: ['catalog', 'programmes', open?.channelId, range.start],
    queryFn: () => invoke('get_programmes', { channelId: open!.channelId, start: range.start, end: range.end }),
    enabled: !!open,
  });
  const list = schedule.data ?? [];
  const current = list.find((p) => p.startUtc <= now && p.endUtc > now);
  const next = current ? list[list.indexOf(current) + 1] : list.find((p) => p.startUtc > now);

  const favourite = useMutation({
    mutationFn: (v: { channelId: number; favourite: boolean }) => invoke('set_favourite', v),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog'] }),
  });
  const record = useMutation({
    mutationFn: (v: { programmeId: string; kind: 'once' | 'series' }) => invoke('schedule_recording', v),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog'] }),
  });

  const watch = (p: Programme) => {
    close();
    activateProgramme(p, channel, navigate, show);
  };

  return (
    <Drawer opened={!!open} onClose={close} position="right" size={440} title={t('detail.title')} padding="xl">
      {channel && (
        <>
          <div className={classes.head}>
            <ChannelLogo channelId={channel.id} name={channel.name} src={channel.logo} density="comfortable" />
            <div className={classes.headText}>
              <h2 className={classes.name}>{channel.name}</h2>
              <div className={classes.meta}>
                {channel.number} · {channel.isCatchup && channel.catchupDays > 0 ? t('detail.catchupWindow', { days: channel.catchupDays }) : t('detail.noCatchup')}
              </div>
            </div>
          </div>
          <div className={classes.actions}>
            <DLButton
              variant="filled"
              onClick={() => {
                close();
                void playChannel(channel.id).then((err) => (err ? undefined : navigate('/player')));
              }}
            >
              {t('detail.watchLive')}
            </DLButton>
            <DLIconButton
              label={t(channel.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')}
              icon={IconStar}
              variant="default"
              pressed={channel.isFavourite}
              filledGlyph={channel.isFavourite}
              onClick={() => favourite.mutate({ channelId: channel.id, favourite: !channel.isFavourite })}
            />
          </div>

          <div className={classes.label}>{t('detail.nowNext')}</div>
          <div className={classes.nn}>
            {current ? (
              <>
                <div className={classes.nnTitle}>{current.title}</div>
                <div className={classes.track}>
                  <span className={classes.fill} style={{ width: `${progress(current.startUtc, current.endUtc, now) * 100}%` }} />
                </div>
                <div className={classes.meta}>
                  {formatRange(current.startUtc, current.endUtc)} · {t('home.minLeft', { min: minutesLeft(current.endUtc, now) })}
                </div>
              </>
            ) : (
              <div className={classes.note}>{t('detail.noSchedule')}</div>
            )}
            {next && <div className={classes.nnNext}>{formatClock(next.startUtc)} {next.title}</div>}
          </div>

          <div className={classes.label}>{t('detail.schedule')}</div>
          {schedule.isError && <ErrorState onRetry={() => void schedule.refetch()} />}
          {!schedule.isError && list.length === 0 && <div className={classes.note}>{t('detail.noSchedule')}</div>}
          {list.map((p) => {
            const st = blockState(p, channel, now);
            const isNowOrFuture = st === 'now' || st === 'future';
            return (
              <div key={p.id} className={classes.line}>
                <UnstyledButton
                  className={classes.row}
                  data-state={st}
                  data-highlight={open?.programmeId === p.id || undefined}
                  disabled={st === 'past-none'}
                  onClick={() => (st === 'future' ? undefined : watch(p))}
                >
                  <span className={classes.time}>{dayLabel(p.startUtc, now) === 'Today' ? formatClock(p.startUtc) : `${dayLabel(p.startUtc, now)} ${formatClock(p.startUtc)}`}</span>
                  <span className={classes.rowTitle}>{p.title}</span>
                  {st === 'now' && <StatusBadge kind="live" />}
                  {st === 'past-archive' && <StatusBadge kind="catchup" />}
                  {p.isScheduled && <StatusBadge kind="rec" />}
                </UnstyledButton>
                {caps.dvr && isNowOrFuture && (
                  <Menu position="bottom-end">
                    <Menu.Target>
                      <DLButton variant="subtle" size="xs">{t('detail.recordOnce')}</DLButton>
                    </Menu.Target>
                    <Menu.Dropdown>
                      {offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
                      <Menu.Item disabled={offline} onClick={() => record.mutate({ programmeId: p.id, kind: 'once' })}>{t('guide.recordOnce')}</Menu.Item>
                      <Menu.Item disabled={offline} onClick={() => record.mutate({ programmeId: p.id, kind: 'series' })}>{t('guide.recordSeries')}</Menu.Item>
                    </Menu.Dropdown>
                  </Menu>
                )}
              </div>
            );
          })}
        </>
      )}
    </Drawer>
  );
}
