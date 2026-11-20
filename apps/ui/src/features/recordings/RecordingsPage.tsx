import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Menu, SegmentedControl } from '@mantine/core';
import { IconChevronDown, IconRepeat } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps } from '../../api/queries';
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
import classes from './Recordings.module.css';

type Tab = 'upcoming' | 'inProgress' | 'completed' | 'rules';

const initials = (name: string) =>
  name.split(/\s+/).slice(0, 2).map((w) => w[0]?.toUpperCase() ?? '').join('');

function Row({ r, now, tab, run, busy }: { r: Recording; now: number; tab: Exclude<Tab, 'rules'>; run: (fn: () => Promise<unknown>) => void; busy: boolean }) {
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  const watch = () => run(async () => {
    const err = await playRecording(r.id);
    if (err) throw err;
    void navigate('/player');
  });
  return (
    <div className={classes.row} role="listitem">
      <span className={classes.day}>{dayLabel(r.startUtc, now)}</span>
      <span className={classes.ini} aria-hidden>{initials(r.channelName)}</span>
      <span className={classes.main}>
        <span className={classes.title}>{r.title}</span>
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
                <DLButton variant="default" size="sm">{t('rec.extend')} <IconChevronDown size={14} /></DLButton>
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
              <DLButton variant="default" size="sm" onClick={() => setConfirming(true)}>{t('rec.delete')}</DLButton>
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

export function RecordingsPage() {
  const qc = useQueryClient();
  const { loaded, hasSource } = useCaps();
  const now = useNowEpoch(30_000);
  const [tab, setTab] = useState<Tab>('upcoming');
  const [failure, setFailure] = useState<string | null>(null);
  const recs = useQuery({ queryKey: ['catalog', 'recordings'], queryFn: () => invoke('list_recordings', {}), refetchInterval: 30_000 });
  const rules = useQuery({ queryKey: ['catalog', 'recordingRules'], queryFn: () => invoke('list_recording_rules', {}) });
  const act = useMutation({
    mutationFn: (fn: () => Promise<unknown>) => fn(),
    onSuccess: () => {
      setFailure(null);
      return qc.invalidateQueries({ queryKey: ['catalog'] });
    },
    onError: (e) => setFailure(e instanceof Error ? e.message : (e as { message?: string }).message || t('rec.actionFailed')),
  });
  const run = (fn: () => Promise<unknown>) => act.mutate(fn);

  if (loaded && !hasSource) return <EmptySource />;
  const d = recs.data;
  const counts: Record<Tab, number> = { upcoming: d?.upcoming.length ?? 0, inProgress: d?.inProgress.length ?? 0, completed: d?.completed.length ?? 0, rules: rules.data?.length ?? 0 };
  const tabs: Tab[] = ['upcoming', 'inProgress', 'completed', 'rules'];
  const list = tab === 'rules' ? rules.data : d?.[tab];
  const loading = tab === 'rules' ? rules.isLoading : recs.isLoading;
  const errored = tab === 'rules' ? rules.isError : recs.isError;

  return (
    <div className={classes.page}>
      <header className={classes.head}>
        <h1 className={classes.h1}>{t('rec.title')}</h1>
        <SegmentedControl
          value={tab}
          onChange={(v) => setTab(v as Tab)}
          aria-label={t('rec.title')}
          data={tabs.map((k) => ({
            value: k,
            label: <span>{t(`rec.tab.${k}`)}<span className={classes.count}>{counts[k]}</span></span>,
          }))}
        />
      </header>
      {failure && <div className={classes.toast} role="alert">{failure}</div>}
      {loading ? (
        <SkeletonList rows={5} label={t('loading.generic')} />
      ) : errored ? (
        <ErrorState message={t('rec.error')} onRetry={() => void (tab === 'rules' ? rules.refetch() : recs.refetch())} />
      ) : !list || list.length === 0 ? (
        <p className={classes.msg} style={{ paddingInline: '2.5rem' }}>{t(`rec.empty.${tab}`)}</p>
      ) : (
        <div className={classes.list} role="list" aria-label={t(`rec.tab.${tab}`)}>
          {tab === 'rules'
            ? (list as RecordingRule[]).map((r) => <RuleRow key={r.id} r={r} run={run} busy={act.isPending} />)
            : (list as Recording[]).map((r) => <Row key={r.id} r={r} now={now} tab={tab} run={run} busy={act.isPending} />)}
        </div>
      )}
    </div>
  );
}
