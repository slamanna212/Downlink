import { useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Menu, Modal } from '@mantine/core';
import { IconChevronDown } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps, useChannel, useOffline } from '../../api/queries';
import type { Channel, MediaTarget, Programme, Recording } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { notifyError } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import { useNowEpoch } from '../../lib/useNow';
import { useDetailStore } from '../../store/detail';
import { useInfoStore, type InfoSubject } from '../../store/info';
import { blockState } from '../guide/guideModel';
import { playCatchup, playChannel, playRecording } from '../player/actions';
import { InfoView, type InfoBase, type InfoTimeState } from './InfoView';
import classes from './Info.module.css';

function targetOf(s: InfoSubject): MediaTarget {
  return s.kind === 'programme' ? { kind: 'programme', id: s.programme.id } : { kind: 'recording', id: s.recording.id };
}

function baseOf(s: InfoSubject, channel: Channel | undefined): InfoBase {
  if (s.kind === 'programme') {
    const p = s.programme;
    return {
      title: p.title ?? channel?.name ?? '',
      episodeTitle: p.subtitle,
      season: p.season,
      episode: p.episode,
      description: p.description,
      isNew: p.isNew,
      isLive: p.isLive,
      isPremiere: p.isPremiere,
      isFinale: p.isFinale,
    };
  }
  const r = s.recording;
  return {
    title: r.title,
    episodeTitle: r.subtitle,
    season: r.season,
    episode: r.episode,
    description: r.description,
    isNew: false,
    isLive: false,
    isPremiere: false,
    isFinale: false,
  };
}

function ProgrammeActions({ p, channel, state, onDone }: { p: Programme; channel: Channel | undefined; state: InfoTimeState; onDone: () => void }) {
  const navigate = useNavigate();
  const { caps } = useCaps();
  const offline = useOffline();
  const qc = useQueryClient();
  const showChannel = useDetailStore((s) => s.show);
  const channelOpen = useDetailStore((s) => s.open?.channelId === p.channelId);
  const record = useMutation({
    mutationFn: (kind: 'once' | 'series') => invoke('schedule_recording', { programmeId: p.id, kind }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['catalog'] }),
    onError: notifyError,
  });
  const go = (run: () => Promise<unknown>) => {
    onDone();
    void run().then((err) => (err ? undefined : navigate('/player')));
  };
  const canRecord = caps.dvr && (state === 'now' || state === 'future') && !p.isScheduled;
  return (
    <>
      {state === 'now' && (
        <DLButton variant="filled" data-autofocus onClick={() => go(() => playChannel(p.channelId))}>
          {t('guide.watchLive')}
        </DLButton>
      )}
      {state === 'past-archive' && (
        <DLButton variant="filled" data-autofocus onClick={() => go(() => playCatchup(p.id))}>
          {t('guide.playFromStart')}
        </DLButton>
      )}
      {canRecord && (
        <Menu position="bottom-start" withinPortal>
          <Menu.Target>
            <DLButton variant="default" loading={record.isPending}>
              {t('guide.record')} <IconChevronDown size={14} />
            </DLButton>
          </Menu.Target>
          <Menu.Dropdown>
            {offline && <Menu.Label>{t('offline.recording')}</Menu.Label>}
            <Menu.Item disabled={offline} onClick={() => record.mutate('once')}>{t('guide.recordOnce')}</Menu.Item>
            <Menu.Item disabled={offline} onClick={() => record.mutate('series')}>{t('guide.recordSeries')}</Menu.Item>
          </Menu.Dropdown>
        </Menu>
      )}
      {channel && !channelOpen && (
        <DLButton
          variant="subtle"
          onClick={() => {
            onDone();
            showChannel(p.channelId, p.id);
          }}
        >
          {t('details.channel')}
        </DLButton>
      )}
    </>
  );
}

function RecordingActions({ r, onDone }: { r: Recording; onDone: () => void }) {
  const navigate = useNavigate();
  if (r.status !== 'completed' && r.status !== 'recording') return null;
  return (
    <DLButton
      variant="filled"
      data-autofocus
      onClick={() => {
        onDone();
        void playRecording(r.id).then((err) => (err ? notifyError(err) : navigate('/player')));
      }}
    >
      {t('details.play')}
    </DLButton>
  );
}

/** The app-wide details modal (mounted once in the shell; opened through `store/info`). */
export function InfoModal() {
  const subject = useInfoStore((s) => s.subject);
  const close = useInfoStore((s) => s.close);
  const now = useNowEpoch(30_000);
  const channelId = subject?.kind === 'programme' ? subject.programme.channelId : subject?.recording.channelId ?? null;
  const fetched = useChannel(subject?.kind === 'programme' && subject.channel ? null : channelId);
  const channel = (subject?.kind === 'programme' ? subject.channel : undefined) ?? fetched.data ?? undefined;
  const target = subject ? targetOf(subject) : null;
  const q = useQuery({
    queryKey: ['details', target],
    queryFn: () => invoke('media_details', { target: target! }),
    enabled: !!target,
    staleTime: 10 * 60_000,
    retry: false,
  });

  const base = subject ? baseOf(subject, channel) : null;
  const slot = subject?.kind === 'programme' ? subject.programme : subject?.recording;
  const timeState: InfoTimeState | null =
    subject?.kind === 'programme' ? blockState(subject.programme, channel, now) : null;
  const recording =
    subject?.kind === 'programme' ? subject.programme.isScheduled : subject?.recording.status === 'recording' || subject?.recording.status === 'scheduled';

  return (
    <Modal
      opened={!!subject}
      onClose={close}
      size={880}
      padding={0}
      withCloseButton={false}
      classNames={{ content: classes.modalContent, body: classes.modalBody }}
      transitionProps={{ transition: 'fade', duration: 160 }}
      aria-label={base ? t('details.dialog', { title: base.title }) : undefined}
    >
      {subject && base && (
        <InfoView
          base={base}
          details={q.data}
          loading={q.isLoading}
          error={q.isError}
          onRetry={() => void q.refetch()}
          onClose={close}
          slot={slot ? { startUtc: slot.startUtc, endUtc: slot.endUtc } : null}
          channel={channel ? { id: channel.id, name: channel.name, logo: channel.logo } : subject.kind === 'recording' && subject.recording.channelId != null ? { id: subject.recording.channelId, name: subject.recording.channelName, logo: null } : null}
          timeState={timeState}
          recording={recording}
          now={now}
          onOpenLink={(url) => void invoke('open_external_link', { url }).catch(notifyError)}
          actions={
            subject.kind === 'programme' && timeState ? (
              <ProgrammeActions p={subject.programme} channel={channel} state={timeState} onDone={close} />
            ) : subject.kind === 'recording' ? (
              <RecordingActions r={subject.recording} onDone={close} />
            ) : null
          }
        />
      )}
    </Modal>
  );
}
