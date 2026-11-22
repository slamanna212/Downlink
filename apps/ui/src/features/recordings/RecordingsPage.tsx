import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Menu, UnstyledButton } from '@mantine/core';
import { IconChevronDown, IconRepeat } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { invalidateRecordings, useCaps, useOffline } from '../../api/queries';
import type { Recording, RecordingRule } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { EmptySource } from '../../pages/EmptySource';
import { dayLabel, formatClock, formatRange } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { t } from '../../lib/i18n';
import { playRecording } from '../player/actions';
import { openInfo } from '../../store/info';
import { episodeLine } from '../../lib/episode';
import classes from './Recordings.module.css';

type Tab = 'upcoming' | 'inProgress' | 'completed' | 'rules';

const initials = (name: string) =>
  name.split(/\s+/).slice(0, 2).map((w) => w[0]?.toUpperCase() ?? '').join('');

export function RecordingRow({ r, now, tab, run, busy, demo }: { r: Recording; now: number; tab: Exclude<Tab, 'rules'>; run: (fn: () => Promise<unknown>) => void; busy: boolean; demo?: Record<string, unknown> }) {
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const watch = () => run(async () => {
    const err = await playRecording(r.id);
    if (!err) void navigate('/player');
  });
  return (
    <div className={classes.row} role="listitem" {...demo}>
      <span className={classes.day}>{dayLabel(r.startUtc, now)}</span>
      <span className={classes.ini} aria-hidden>{initials(r.channelName)}</span>
      <span className={classes.main}>
        <UnstyledButton className={classes.titleLink} onClick={() => openInfo({ kind: 'recording', recording: r })}>
          <span className={classes.title}>{r.title}</span>
        </UnstyledButton>
        {episodeLine(r) && <span className={classes.ep}>{episodeLine(r)}</span>}
        <span className={classes.sub}>{r.channelName}</span>
        {r.status === 'failed' && r.error && <span className={classes.err}>{r.error}</span>}
      </span>
      <span className={classes.time}>
        {tab === 'inProgress' ? t('rec.recordingUntil', { time: formatClock(r.endUtc) }) : formatRange(r.startUtc, r.endUtc)}
      </span>
      <span className={classes.rule}>
        {r.ruleLabel ? <><IconRepeat size={14} aria-hidden />{r.ruleLabel}</> : t('rec.once')}
      </span>
      <span className={classes.actions}>
        {tab === 'upcoming' && <DLButton variant="default" size="sm" disabled={busy} onClick={() => run(() => invoke('cancel_recording', { id: r.id }))}>{t('rec.cancel')}</DLButton>}
        {tab === 'inProgress' && (
          <>
            <StatusBadge kind="rec" />
            <DLButton variant="filled" size="sm" onClick={watch}>{t('rec.watch')}</DLButton>
            <Menu position="bottom-end">
              <Menu.Target>
                <DLButton variant="default" size="sm" disabled={busy}>{t('rec.extend')} <IconChevronDown size={14} /></DLButton>
              </Menu.Target>
              <Menu.Dropdown>
                {[15, 30, 60].map((m) => (
                  <Menu.Item key={m} onClick={() => run(() => invoke('extend_recording', { id: r.id, minutes: m }))}>
                    {t(m === 15 ? 'rec.extend15' : m === 30 ? 'rec.extend30' : 'rec.extend60')}
                  </Menu.Item>
                ))}
              </Menu.Dropdown>
            </Menu>
            <DLButton variant="danger" size="sm" disabled={busy} onClick={() => run(() => invoke('stop_recording', { id: r.id }))}>{t('rec.stop')}</DLButton>
          </>
        )}
        {tab === 'completed' &&
          (confirming ? (
            <>
              <span className={classes.confirm}>{t('rec.confirmDelete', { title: r.title })}</span>
              <DLButton variant="danger" size="sm" disabled={busy} onClick={() => run(() => invoke('delete_recording', { id: r.id }))}>{t('rec.confirmYes')}</DLButton>
              <DLButton variant="default" size="sm" onClick={() => setConfirming(false)}>{t('rec.confirmNo')}</DLButton>
            </>
          ) : (
            <>
              {r.status === 'failed' && <StatusBadge kind="drm" />}
              <DLButton variant="filled" size="sm" onClick={watch}>{t('rec.play')}</DLButton>
              <DLButton variant="default" size="sm" disabled={busy} onClick={() => setConfirming(true)}>{t('rec.delete')}</DLButton>
            </>
          ))}
      </span>
    </div>
  );
}

function RuleRow({ r, run, busy }: { r: RecordingRule; run: (fn: () => Promise<unknown>) => void; busy: boolean }) {
  return (
    <div className={`${classes.row} ${classes.rowRules}`} role="listitem">
      <span className={classes.main}>
        <span className={classes.title}>{r.title}</span>
        <span className={classes.sub}>{r.channelName}</span>
      </span>
      <span className={classes.rule}><IconRepeat size={14} aria-hidden />{r.kind === 'series' ? t('rec.rule.series') : t('rec.rule.recurring')}</span>
      <span className={classes.time}>{r.schedule}</span>
      <span className={classes.actions}>
        <DLButton variant="default" size="sm" disabled={busy} onClick={() => run(() => invoke('delete_recording_rule', { id: r.id }))}>{t('rec.deleteRule')}</DLButton>
      </span>
    </div>
  );
}

type ListSection = Exclude<Tab, 'rules'>;

function SectionHead({ id, title, count }: { id: string; title: string; count: number }) {
  return (
    <h2 className={classes.h2} id={id}>
      {title}
      <span className={classes.count}>{count}</span>
    </h2>
  );
}

export function RecordingsPage() {
  const qc = useQueryClient();
  const { loaded, hasSource } = useCaps();
  const offline = useOffline();
  const now = useNowEpoch(30_000);
  const [failure, setFailure] = useState<string | null>(null);
  const recs = useQuery({ queryKey: ['catalog', 'recordings'], queryFn: () => invoke('list_recordings', {}), refetchInterval: 30_000 });
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

  if (loaded && !hasSource) return <EmptySource />;
  const d = recs.data;
  // Everything on one page, most time-sensitive first.
  const sections: ListSection[] = ['inProgress', 'upcoming', 'completed'];

  return (
    <div className={classes.page}>
      <header className={classes.head}>
        <h1 className={classes.h1}>{t('rec.title')}</h1>
      </header>
      {offline && <div className={classes.toast} role="status">{t('offline.recording')}</div>}
      {failure && <div className={classes.toast} role="alert">{failure}</div>}
      <div className={classes.list}>
        {recs.isLoading ? (
          <SkeletonList rows={5} label={t('loading.generic')} />
        ) : recs.isError || !d ? (
          <ErrorState message={t('rec.error')} onRetry={() => void recs.refetch()} />
        ) : (
          sections.map((k) => (
            <section key={k} className={classes.section} aria-labelledby={`rec-${k}`}>
              <SectionHead id={`rec-${k}`} title={t(`rec.tab.${k}`)} count={d[k].length} />
              {d[k].length === 0 ? (
                <p className={classes.msg}>{t(`rec.empty.${k}`)}</p>
              ) : (
                <div role="list" aria-labelledby={`rec-${k}`}>
                  {d[k].map((r) => <RecordingRow key={r.id} r={r} now={now} tab={k} run={run} busy={busy} />)}
                </div>
              )}
            </section>
          ))
        )}
        {!recs.isLoading && (
          <section className={classes.section} aria-labelledby="rec-rules">
            <SectionHead id="rec-rules" title={t('rec.tab.rules')} count={rules.data?.length ?? 0} />
            {rules.isLoading ? (
              <SkeletonList rows={2} label={t('loading.generic')} />
            ) : rules.isError ? (
              <ErrorState message={t('rec.error')} onRetry={() => void rules.refetch()} />
            ) : !rules.data || rules.data.length === 0 ? (
              <p className={classes.msg}>{t('rec.empty.rules')}</p>
            ) : (
              <div role="list" aria-labelledby="rec-rules">
                {rules.data.map((r) => <RuleRow key={r.id} r={r} run={run} busy={busy} />)}
              </div>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
