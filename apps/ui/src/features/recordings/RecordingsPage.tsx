import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { TextInput, UnstyledButton } from '@mantine/core';
import { IconCalendarTime, IconSearch } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { invalidateRecordings, useCaps, useOffline, useRecordings } from '../../api/queries';
import type { Recording } from '../../api/types';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { Icon } from '../../components/Icon/Icon';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { EmptySource } from '../../pages/EmptySource';
import { useNowEpoch } from '../../lib/useNow';
import { useDLFocusable } from '../../lib/spatial';
import { t } from '../../lib/i18n';
import { useViewStateStore } from '../../store/viewState';
import { playRecording } from '../player/actions';
import { ChannelStack } from './ChannelMark';
import { ShowDetail } from './ShowDetail';
import { UpcomingView } from './UpcomingView';
import { UPCOMING, groupShows, initialSelection, type Show } from './shows';
import classes from './Recordings.module.css';

function listSub(show: Show): string {
  if (show.recordingNow) return t('rec.list.recordingNow');
  const n = show.recordings.length;
  const list = n === 0 ? t('rec.list.none') : n === 1 ? t('rec.list.one') : t('rec.list.many', { n });
  return show.upcoming.length ? t('rec.list.upcoming', { list, n: show.upcoming.length }) : list;
}

export interface ListItemProps {
  active: boolean;
  onSelect: () => void;
  focusKey: string;
  /** Storybook: forced state props. */
  demo?: Record<string, unknown>;
}

/** One show in the list: channel marks, title, state line, recording dot and new count. */
export function ShowItem({ show, active, onSelect, focusKey, demo }: ListItemProps & { show: Show }) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ focusKey, onEnterPress: onSelect });
  return (
    <UnstyledButton ref={ref} className={classes.item} data-active={active || undefined} aria-current={active ? 'true' : undefined} onClick={onSelect} {...focusProps} {...demo}>
      <span className={classes.itemLead}>{show.channels.length > 0 && <ChannelStack channels={show.channels} size="sm" />}</span>
      <span className={classes.itemMain}>
        <span className={classes.itemTitle}>{show.title}</span>
        <span className={classes.itemSub}>{listSub(show)}</span>
      </span>
      {show.recordingNow && <span className={classes.recDot} role="img" aria-label={t('rec.list.recordingNow')} />}
      {show.newCount > 0 && (
        <span className={classes.newChip} aria-label={t('rec.newCount', { n: show.newCount })}>
          {show.newCount}
        </span>
      )}
    </UnstyledButton>
  );
}

/** The "Coming up" entry at the top of the list. */
export function ComingUpItem({ count, active, onSelect, focusKey, demo }: ListItemProps & { count: number }) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ focusKey, onEnterPress: onSelect });
  return (
    <UnstyledButton ref={ref} className={classes.item} data-active={active || undefined} aria-current={active ? 'true' : undefined} onClick={onSelect} {...focusProps} {...demo}>
      <span className={classes.itemLead}>
        <span className={classes.upIcon}>
          <Icon icon={IconCalendarTime} size={20} />
        </span>
      </span>
      <span className={classes.itemMain}>
        <span className={classes.itemTitle}>{t('rec.comingUp')}</span>
        <span className={classes.itemSub}>{t('rec.scheduled', { n: count })}</span>
      </span>
    </UnstyledButton>
  );
}

export function RecordingsPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { loaded, hasSource } = useCaps();
  const offline = useOffline();
  const now = useNowEpoch(30_000);
  const [failure, setFailure] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const saved = useViewStateStore((s) => s.recordings);
  const save = useViewStateStore((s) => s.saveRecordings);
  const recs = useRecordings();
  const rules = useQuery({ queryKey: ['catalog', 'recordingRules'], queryFn: () => invoke('list_recording_rules', {}) });
  const act = useMutation({
    mutationFn: (fn: () => Promise<unknown>) => fn(),
    onSuccess: () => {
      setFailure(null);
      return invalidateRecordings(qc);
    },
    onError: (e) => setFailure(e instanceof Error ? e.message : (e as { message?: string }).message || t('rec.actionFailed')),
  });
  const run = (fn: () => Promise<unknown>) => act.mutate(fn);
  const busy = act.isPending || offline;
  const onPlay = (r: Recording) =>
    run(async () => {
      const err = await playRecording(r.id);
      if (!err) void navigate('/player');
    });

  const shows = useMemo(() => (recs.data ? groupShows(recs.data, rules.data ?? []) : []), [recs.data, rules.data]);
  if (loaded && !hasSource) return <EmptySource />;

  const selected = initialSelection(shows, saved);
  const q = query.trim().toLowerCase();
  const visible = q ? shows.filter((s) => s.title.toLowerCase().includes(q)) : shows;
  const show = shows.find((s) => s.key === selected);
  const savedCount = shows.reduce((n, s) => n + s.recordings.length, 0);
  const upcomingCount = shows.reduce((n, s) => n + s.upcoming.length, 0);

  return (
    <div className={classes.page}>
      <aside className={classes.aside}>
        <div className={classes.asideHead}>
          <div className={classes.titleRow}>
            <h1 className={classes.h1}>{t('rec.title')}</h1>
            {recs.data && <span className={classes.sub}>{t('rec.saved', { n: savedCount })}</span>}
          </div>
          <TextInput
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            placeholder={t('rec.filter')}
            aria-label={t('rec.filter')}
            leftSection={<Icon icon={IconSearch} size={16} />}
          />
        </div>
        <nav className={classes.list} aria-label={t('rec.shows')}>
          {recs.data && (
            <>
              <ComingUpItem focusKey="REC_UPCOMING" count={upcomingCount} active={selected === UPCOMING} onSelect={() => save(UPCOMING)} />
              <div className={classes.label}>{t('rec.shows')}</div>
              {visible.map((s) => (
                <ShowItem key={s.key} focusKey={`REC_${s.key}`} show={s} active={s.key === selected} onSelect={() => save(s.key)} />
              ))}
              {q && visible.length === 0 && <p className={classes.msg}>{t('rec.noMatch', { q: query.trim() })}</p>}
              {!q && shows.length === 0 && <p className={classes.msg}>{t('rec.empty.all')}</p>}
            </>
          )}
        </nav>
      </aside>
      <main className={classes.main}>
        {offline && <div className={classes.toast} role="status">{t('offline.recording')}</div>}
        {failure && <div className={classes.toast} role="alert">{failure}</div>}
        {recs.isLoading ? (
          <div className={classes.detail}>
            <SkeletonList rows={5} label={t('loading.generic')} />
          </div>
        ) : recs.isError || !recs.data ? (
          <div className={classes.detail}>
            <ErrorState message={t('rec.error')} onRetry={() => void recs.refetch()} />
          </div>
        ) : show ? (
          <ShowDetail key={show.key} show={show} now={now} run={run} busy={busy} onPlay={onPlay} />
        ) : (
          <UpcomingView shows={shows} now={now} run={run} busy={busy} />
        )}
      </main>
    </div>
  );
}
