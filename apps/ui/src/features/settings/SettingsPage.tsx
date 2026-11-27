import { useEffect, useState } from 'react';
import { Link, Navigate, useNavigate, useParams } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Modal, NavLink, PasswordInput, SegmentedControl, Select, Slider, Switch, Text } from '@mantine/core';
import { IconInfoSquareRounded, IconPalette, IconPlayerPlay, IconPlugConnected, IconRefresh, IconServer2, IconStethoscope, IconTrophy } from '@tabler/icons-react';
import { invoke, toAppError } from '../../api/ipc';
import { useMediaServerStatus, useSettings, useSources, useUpdateStatus, useUpdateWaiting } from '../../api/queries';
import type { AudioOutput, Deinterlace, DensitySetting, HdrOutput, HwdecSetting, PlexPin, Source, UpdateChannel } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Icon, type TablerIcon } from '../../components/Icon/Icon';
import { SettingRow } from '../../components/SettingRow/SettingRow';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { formatUpdated, nowEpoch, splitHoursMinutes } from '../../lib/time';
import { t, type MessageKey } from '../../lib/i18n';
import { useTasksStore } from '../../store/tasks';
import { useUiStore } from '../../store/ui';
import { ReleaseNotes } from './ReleaseNotes';
import { Section } from './Section';
import { SportsSection } from './SportsSection';
import { copyDiagnostics } from '../diagnostics/copyDiagnostics';
import { notifyError } from '../../components/Notify/notify';
import classes from './Settings.module.css';

const SECTIONS: { id: string; label: MessageKey; icon: TablerIcon }[] = [
  { id: 'sources', label: 'settings.sources', icon: IconPlugConnected },
  { id: 'playback', label: 'settings.playback', icon: IconPlayerPlay },
  { id: 'sports', label: 'settings.sports', icon: IconTrophy },
  { id: 'metadata', label: 'settings.metadata', icon: IconInfoSquareRounded },
  { id: 'mediaserver', label: 'settings.mediaServer', icon: IconServer2 },
  { id: 'appearance', label: 'settings.appearance', icon: IconPalette },
  { id: 'updates', label: 'settings.updates', icon: IconRefresh },
  { id: 'about', label: 'settings.about', icon: IconStethoscope },
];

function SourceRow({ source, onRemove }: { source: Source; onRemove: (s: Source) => void }) {
  const qc = useQueryClient();
  const task = useTasksStore((s) => s.bySource[source.id]);
  const sync = useMutation({
    mutationFn: () => invoke('sync_source', { sourceId: source.id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'sources'] }),
  });
  const hls = useMutation({
    mutationFn: (hlsTransport: boolean) => invoke('set_source_options', { sourceId: source.id, hlsTransport }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'sources'] }),
  });
  const syncing = source.status === 'syncing' || !!task || sync.isPending;
  const help = task
    ? t('settings.sources.syncing', { phase: task.phase })
    : source.lastSyncAt
      ? t('settings.sources.lastSync', { time: formatUpdated(source.lastSyncAt) })
      : t('settings.sources.never');
  return (
    <>
    <SettingRow
      label={source.name}
      help={`${source.kind} · ${source.host ?? ''} · ${t(`source.status.${source.status}`)} · ${help}`}
      wide
    >
      <div className={classes.actions}>
        <DLButton variant="default" size="sm" loading={syncing} onClick={() => sync.mutate()}>
          {t('settings.sources.sync')}
        </DLButton>
        <DLButton variant="danger" size="sm" onClick={() => onRemove(source)}>
          {t('settings.sources.remove')}
        </DLButton>
      </div>
    </SettingRow>
    {source.supportsHlsTransport && (
      <SettingRow label={t('settings.hls')} help={t('settings.hls.help')} wide>
        <Switch
          aria-label={`${t('settings.hls')}: ${source.name}`}
          checked={source.hlsTransport ?? false}
          disabled={hls.isPending}
          onChange={(e) => hls.mutate(e.currentTarget.checked)}
        />
      </SettingRow>
    )}
    </>
  );
}

const REFRESH_HOURS = [0, 1, 2, 4, 6, 12, 24];
const DEFAULT_REFRESH_HOURS = 4;

function refreshLabel(hours: number): string {
  if (hours === 0) return t('settings.autoRefresh.off');
  return hours === 1 ? t('settings.autoRefresh.every1') : t('settings.autoRefresh.everyN', { n: hours });
}

/** Background refresh interval and whether it may run during playback (machine-local settings). */
function AutoRefreshRows() {
  const { data } = useSettings();
  const patch = useSettingsPatch();
  if (!data) return null;
  const hours = data.autoRefreshHours ?? DEFAULT_REFRESH_HOURS;
  const options = (REFRESH_HOURS.includes(hours) ? REFRESH_HOURS : [...REFRESH_HOURS, hours].sort((a, b) => a - b)).map((h) => ({
    value: String(h),
    label: refreshLabel(h),
  }));
  return (
    <>
      <SettingRow label={t('settings.autoRefresh')} help={t('settings.autoRefresh.help')}>
        <Select
          w="100%"
          aria-label={t('settings.autoRefresh')}
          data={options}
          value={String(hours)}
          allowDeselect={false}
          onChange={(v) => v !== null && patch.mutate({ autoRefreshHours: Number(v) })}
        />
      </SettingRow>
      <SettingRow label={t('settings.refreshDuringPlayback')} help={t('settings.refreshDuringPlayback.help')} wide>
        <Switch
          aria-label={t('settings.refreshDuringPlayback')}
          checked={data.refreshDuringPlayback ?? true}
          disabled={hours === 0}
          onChange={(e) => patch.mutate({ refreshDuringPlayback: e.currentTarget.checked })}
        />
      </SettingRow>
    </>
  );
}

function SourcesSection() {
  const qc = useQueryClient();
  const { data, isLoading } = useSources();
  const navigate = useNavigate();
  const [removing, setRemoving] = useState<Source | null>(null);
  const remove = useMutation({
    mutationFn: (id: number) => invoke('remove_source', { sourceId: id }),
    onSuccess: () => {
      setRemoving(null);
      void qc.invalidateQueries({ queryKey: ['catalog'] });
    },
  });
  return (
    <Section title={t('settings.sources')} help={t('settings.sources.help')}>
      <div className={classes.toolbar}>
        <DLButton variant="filled" onClick={() => void navigate('/sources/add')}>{t('settings.sources.add')}</DLButton>
      </div>
      {isLoading ? <SkeletonList rows={2} label={t('loading.generic')} /> : (data ?? []).map((s) => (
        <SourceRow key={s.id} source={s} onRemove={setRemoving} />
      ))}
      <AutoRefreshRows />
      <Modal opened={!!removing} onClose={() => setRemoving(null)} title={t('settings.sources.removeAction')}>
        <Text size="md" mb="lg">{removing ? t('settings.sources.removeConfirm', { name: removing.name }) : ''}</Text>
        <div className={`${classes.actions} ${classes.end}`}>
          <DLButton variant="subtle" onClick={() => setRemoving(null)}>{t('common.cancel')}</DLButton>
          <DLButton variant="danger" loading={remove.isPending} onClick={() => removing && remove.mutate(removing.id)}>
            {t('settings.sources.removeAction')}
          </DLButton>
        </div>
      </Modal>
    </Section>
  );
}

const HWDEC: { value: HwdecSetting; label: string }[] = [
  { value: 'auto-safe', label: t('settings.hwdec.autoSafe') },
  { value: 'auto', label: t('settings.hwdec.auto') },
  { value: 'no', label: t('settings.hwdec.no') },
  { value: 'vaapi', label: 'VA-API' },
  { value: 'nvdec', label: 'NVDEC' },
  { value: 'd3d11va', label: 'D3D11VA' },
  { value: 'videotoolbox', label: 'VideoToolbox' },
];

function useSettingsPatch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: Parameters<typeof invoke<'set_settings'>>[1]['patch']) => invoke('set_settings', { patch }),
    onSuccess: (s) => qc.setQueryData(['settings'], s),
  });
}

const AUDIO_AUTO = 'auto';

/** Always lead with the mpv `auto` device; keep a stored-but-missing device selectable. */
export function audioDeviceOptions(list: { id: string; name: string }[], stored: string | null | undefined) {
  const opts = [{ value: AUDIO_AUTO, label: t('settings.audioDevice.default') }];
  for (const d of list) {
    if (d.id !== AUDIO_AUTO && d.id !== 'default') opts.push({ value: d.id, label: d.name });
  }
  if (stored && stored !== 'default' && !opts.some((o) => o.value === stored)) {
    opts.push({ value: stored, label: t('settings.audioDevice.missing', { name: stored }) });
  }
  return opts;
}

/** "Off", "45 min", "1 h", "1 h 30 min". */
function timeshiftLabel(minutes: number): string {
  if (minutes <= 0) return t('settings.timeshift.off');
  const { h, m } = splitHoursMinutes(minutes * 60);
  if (h === 0) return t('settings.timeshift.minutes', { m });
  return m === 0 ? t('settings.timeshift.hours', { h }) : t('settings.timeshift.hoursMinutes', { h, m });
}

/** Disk the buffer may use at its size cap (about 12 Mbit/s, matching the player's limit). */
const timeshiftDiskGb = (minutes: number): string => ((minutes * 60 * 1.5) / 1000).toFixed(1);

function PlaybackSection() {
  const { data } = useSettings();
  const patch = useSettingsPatch();
  const diag = useQuery({ queryKey: ['diagnostics', 'hwdec'], queryFn: () => invoke('get_diagnostics', {}) });
  const devices = useQuery({ queryKey: ['audioDevices'], queryFn: () => invoke('list_audio_devices', {}) });
  const video = useQuery({ queryKey: ['videoCaps'], queryFn: () => invoke('get_video_caps', {}), staleTime: Infinity });
  const [buffer, setBuffer] = useState<number | null>(null);
  const [timeshift, setTimeshift] = useState<number | null>(null);
  const [subSize, setSubSize] = useState<number | null>(null);
  if (!data) return <SkeletonList rows={3} label={t('loading.generic')} />;
  const shown = buffer ?? data.liveBufferSecs;
  const shownTimeshift = timeshift ?? data.timeshiftMinutes ?? 60;
  const current = diag.data?.hwdecCurrent;
  const rtx = video.data?.rtxVideo ?? false;
  const hdrOn = (data.hdrOutput ?? 'auto') === 'auto';
  return (
    <Section title={t('settings.playback')} help={t('settings.playback.help')}>
      <SettingRow label={t('settings.hwdec')} help={current ? t('settings.hwdec.current', { current }) : t('settings.hwdec.help')}>
        <Select
          w="100%"
          aria-label={t('settings.hwdec')}
          data={HWDEC}
          value={data.hwdec}
          allowDeselect={false}
          onChange={(v) => v && patch.mutate({ hwdec: v as HwdecSetting })}
        />
      </SettingRow>
      <SettingRow label={t('settings.performanceMode')} help={t('settings.performanceMode.help')} wide>
        <Switch
          aria-label={t('settings.performanceMode')}
          checked={data.performanceMode ?? false}
          onChange={(e) => patch.mutate({ performanceMode: e.currentTarget.checked })}
        />
      </SettingRow>
      <SettingRow label={t('settings.resumeLast')} help={t('settings.resumeLast.help')} wide>
        <Switch
          aria-label={t('settings.resumeLast')}
          checked={data.resumeLastChannel ?? false}
          onChange={(e) => patch.mutate({ resumeLastChannel: e.currentTarget.checked })}
        />
      </SettingRow>
      <SettingRow label={t('settings.buffer')} help={t('settings.buffer.help')}>
        <div className={classes.sliderRow}>
          <Slider
            min={3}
            max={10}
            step={1}
            value={shown}
            label={null}
            thumbLabel={t('settings.buffer')}
            onChange={setBuffer}
            onChangeEnd={(v) => {
              patch.mutate({ liveBufferSecs: v });
              setBuffer(null);
            }}
          />
          <span className={classes.value}>{shown} s</span>
        </div>
      </SettingRow>
      <SettingRow
        label={t('settings.timeshift')}
        help={shownTimeshift > 0 ? t('settings.timeshift.help', { gb: timeshiftDiskGb(shownTimeshift) }) : t('settings.timeshift.offHelp')}
      >
        <div className={classes.sliderRow}>
          <Slider
            min={0}
            max={180}
            step={15}
            value={shownTimeshift}
            label={null}
            thumbLabel={t('settings.timeshift')}
            onChange={setTimeshift}
            onChangeEnd={(v) => {
              patch.mutate({ timeshiftMinutes: v });
              setTimeshift(null);
            }}
          />
          <span className={classes.value}>{timeshiftLabel(shownTimeshift)}</span>
        </div>
      </SettingRow>
      <SettingRow label={t('settings.audioDevice')} help={t('settings.audioDevice.help')}>
        <Select
          w="100%"
          aria-label={t('settings.audioDevice')}
          data={audioDeviceOptions(devices.data ?? [], data.audioDevice)}
          value={data.audioDevice && data.audioDevice !== 'default' ? data.audioDevice : AUDIO_AUTO}
          allowDeselect={false}
          onChange={(v) => v && patch.mutate({ audioDevice: v === AUDIO_AUTO ? null : v })}
        />
      </SettingRow>
      <SettingRow label={t('settings.audioOutput')} help={t('settings.audioOutput.help')} wide>
        <SegmentedControl
          aria-label={t('settings.audioOutput')}
          value={data.audioOutput}
          onChange={(v) => patch.mutate({ audioOutput: v as AudioOutput })}
          data={[
            { value: 'stereo', label: t('settings.audioOutput.stereo') },
            { value: 'surround', label: t('settings.audioOutput.surround') },
            { value: 'passthrough', label: t('settings.audioOutput.passthrough') },
          ]}
        />
      </SettingRow>
      <SettingRow label={t('settings.deinterlace')} help={t('settings.deinterlace.help')} wide>
        <SegmentedControl
          aria-label={t('settings.deinterlace')}
          value={data.deinterlace}
          onChange={(v) => patch.mutate({ deinterlace: v as Deinterlace })}
          data={[
            { value: 'auto', label: t('settings.deinterlace.auto') },
            { value: 'yes', label: t('settings.deinterlace.yes') },
            { value: 'no', label: t('settings.deinterlace.no') },
          ]}
        />
      </SettingRow>
      <SettingRow label={t('settings.hdrOutput')} help={t('settings.hdrOutput.help')} wide>
        <SegmentedControl
          aria-label={t('settings.hdrOutput')}
          value={data.hdrOutput ?? 'auto'}
          onChange={(v) => patch.mutate({ hdrOutput: v as HdrOutput })}
          data={[
            { value: 'auto', label: t('settings.hdrOutput.auto') },
            { value: 'off', label: t('settings.hdrOutput.off') },
          ]}
        />
      </SettingRow>
      <SettingRow
        label={t('settings.rtxSuperResolution')}
        help={rtx ? t('settings.rtxSuperResolution.help') : t('settings.rtx.unavailable')}
        wide
      >
        <Switch
          aria-label={t('settings.rtxSuperResolution')}
          checked={rtx && (data.rtxSuperResolution ?? false)}
          disabled={!rtx}
          onChange={(e) => patch.mutate({ rtxSuperResolution: e.currentTarget.checked })}
        />
      </SettingRow>
      <SettingRow
        label={t('settings.rtxHdr')}
        help={!rtx ? t('settings.rtx.unavailable') : hdrOn ? t('settings.rtxHdr.help') : t('settings.rtxHdr.needsHdr')}
        wide
      >
        <Switch
          aria-label={t('settings.rtxHdr')}
          checked={rtx && hdrOn && (data.rtxHdr ?? false)}
          disabled={!rtx || !hdrOn}
          onChange={(e) => patch.mutate({ rtxHdr: e.currentTarget.checked })}
        />
      </SettingRow>
      {rtx && (
        <p className={classes.rtxNote}>{t('settings.rtx.note', { adapter: video.data?.rtxAdapter ?? 'NVIDIA' })}</p>
      )}
      <SettingRow label={t('settings.subtitleSize')} help={t('settings.subtitleSize.help')}>
        <div className={classes.sliderRow}>
          <Slider
            min={50}
            max={150}
            step={10}
            value={subSize ?? data.subtitleSize}
            label={null}
            thumbLabel={t('settings.subtitleSize')}
            onChange={setSubSize}
            onChangeEnd={(v) => {
              patch.mutate({ subtitleSize: v });
              setSubSize(null);
            }}
          />
          <span className={classes.value}>{subSize ?? data.subtitleSize}%</span>
        </div>
      </SettingRow>
      <SettingRow label={t('settings.subtitleBackground')} help={t('settings.subtitleBackground.help')} wide>
        <Switch
          aria-label={t('settings.subtitleBackground')}
          checked={data.subtitleBackground}
          onChange={(e) => patch.mutate({ subtitleBackground: e.currentTarget.checked })}
        />
      </SettingRow>
    </Section>
  );
}

function UpdatesSection() {
  const qc = useQueryClient();
  const status = useUpdateStatus();
  const setChannel = useMutation({
    mutationFn: (channel: UpdateChannel) => invoke('set_update_channel', { channel }),
    onSuccess: (s) => qc.setQueryData(['update'], s),
  });
  const check = useMutation({
    mutationFn: () => invoke('check_for_updates', {}),
    onSuccess: (s) => qc.setQueryData(['update'], s),
  });
  const install = useMutation({
    mutationFn: () => invoke('install_update', {}),
    onError: (e) => notifyError(e),
  });
  const s = status.data;
  if (!s) return <SkeletonList rows={2} label={t('loading.generic')} />;
  const help = s.available
    ? s.ready
      ? t('settings.updates.available', { available: s.available })
      : t('settings.updates.downloading', { available: s.available })
    : s.lastChecked
      ? t('settings.updates.upToDate', { time: formatUpdated(s.lastChecked) })
      : t('settings.updates.never');
  return (
    <Section title={t('settings.updates')} help={t('settings.updates.help')}>
      {s.available && (
        <div className={classes.updateCard} role="status" data-testid="update-card">
          <div className={classes.updateHead}>
            <span className={classes.updateDot} aria-hidden />
            <div className={classes.updateText}>
              <h3 className={classes.updateTitle}>
                {t(s.ready ? 'settings.updates.ready.title' : 'settings.updates.available.title', { available: s.available })}
              </h3>
              <p className={classes.updateBody}>
                {s.ready ? t('settings.updates.ready.body') : t('settings.updates.downloading.body')}
              </p>
            </div>
            <DLButton
              variant="filled"
              loading={install.isPending || !s.ready}
              disabled={!s.ready}
              onClick={() => install.mutate()}
            >
              {t('settings.updates.install')}
            </DLButton>
          </div>
          <ReleaseNotes version={s.available} fallback={s.notes} />
        </div>
      )}
      <SettingRow label={t('settings.updates.channel')} help={t('settings.updates.channel.help')} wide>
        <SegmentedControl
          aria-label={t('settings.updates.channel')}
          value={s.channel}
          onChange={(v) => setChannel.mutate(v as UpdateChannel)}
          data={[
            { value: 'stable', label: t('settings.updates.stable') },
            { value: 'dev', label: t('settings.updates.dev') },
          ]}
        />
      </SettingRow>
      <SettingRow label={t('settings.updates.status')} help={`${t('settings.updates.current', { current: s.current })} ${help}`} wide>
        <DLButton variant="default" loading={check.isPending} onClick={() => check.mutate()}>
          {t('settings.updates.check')}
        </DLButton>
      </SettingRow>
    </Section>
  );
}

/** How long a Plex sign-in waits for approval in the browser. */
const PLEX_PIN_TIMEOUT_SECS = 5 * 60;
const PLEX_PIN_POLL_MS = 2000;

function MediaServerSection() {
  const qc = useQueryClient();
  const status = useMediaServerStatus();
  const plex = status.data?.plex;
  const [pin, setPin] = useState<PlexPin | null>(null);
  const [expired, setExpired] = useState(false);
  const refresh = () => qc.invalidateQueries({ queryKey: ['mediaServer'] });
  const start = useMutation({
    mutationFn: () => invoke('plex_sign_in_start', {}),
    onSuccess: (p) => {
      setExpired(false);
      setPin(p);
    },
  });
  const servers = useQuery({
    queryKey: ['mediaServer', 'servers'],
    queryFn: () => invoke('plex_servers', {}),
    enabled: !!plex?.signedIn,
  });
  const select = useMutation({
    mutationFn: (serverId: string) => invoke('plex_select_server', { serverId }),
    onSuccess: () => void refresh(),
  });
  const signOut = useMutation({
    mutationFn: () => invoke('media_server_sign_out', {}),
    onSuccess: () => void refresh(),
  });

  // Poll plex.tv until the user approves the sign-in in the browser (or it times out).
  useEffect(() => {
    if (!pin) return;
    let stopped = false;
    const startedAt = nowEpoch();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const tick = async () => {
      if (stopped) return;
      if (nowEpoch() - startedAt > PLEX_PIN_TIMEOUT_SECS) {
        setPin(null);
        setExpired(true);
        return;
      }
      try {
        if (await invoke('plex_sign_in_check', { pinId: pin.id })) {
          if (stopped) return;
          setPin(null);
          void qc.invalidateQueries({ queryKey: ['mediaServer'] });
          return;
        }
      } catch (e) {
        if (stopped) return;
        setPin(null);
        notifyError(e);
        return;
      }
      if (!stopped) timer = setTimeout(() => void tick(), PLEX_PIN_POLL_MS);
    };
    timer = setTimeout(() => void tick(), PLEX_PIN_POLL_MS);
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, [pin, qc]);

  if (!plex) return <SkeletonList rows={2} label={t('loading.generic')} />;
  const accountHelp = plex.signedIn
    ? plex.accountName ? t('settings.plex.signedInAs', { name: plex.accountName }) : t('settings.plex.signedIn')
    : pin ? t('settings.plex.waiting', { code: pin.code })
    : expired ? t('settings.plex.expired')
    : t('settings.plex.help');
  const serverList = servers.data ?? [];
  return (
    <Section title={t('settings.mediaServer')} help={t('settings.mediaServer.help')}>
      <SettingRow label={t('settings.plex')} help={accountHelp} wide>
        {plex.signedIn ? (
          <DLButton variant="danger" size="sm" loading={signOut.isPending} onClick={() => signOut.mutate()}>
            {t('settings.plex.signOut')}
          </DLButton>
        ) : pin ? (
          <DLButton variant="subtle" onClick={() => setPin(null)}>{t('settings.plex.cancel')}</DLButton>
        ) : (
          <DLButton variant="filled" loading={start.isPending} onClick={() => start.mutate()}>
            {t('settings.plex.signIn')}
          </DLButton>
        )}
      </SettingRow>
      {plex.signedIn && (
        <SettingRow
          label={t('settings.plex.server')}
          help={plex.serverName ? t('settings.plex.server.connected', { name: plex.serverName }) : t('settings.plex.server.help')}
        >
          {servers.isLoading ? (
            <SkeletonList rows={1} label={t('loading.generic')} />
          ) : serverList.length === 0 ? (
            <Text size="sm">{t('settings.plex.server.none')}</Text>
          ) : (
            <Select
              w="100%"
              aria-label={t('settings.plex.server')}
              placeholder={t('settings.plex.server.pick')}
              data={serverList.map((x) => ({ value: x.id, label: x.owned ? x.name : t('settings.plex.server.shared', { name: x.name }) }))}
              value={plex.serverId}
              allowDeselect={false}
              disabled={select.isPending}
              onChange={(v) => v && v !== plex.serverId && select.mutate(v)}
            />
          )}
        </SettingRow>
      )}
    </Section>
  );
}

const TMDB_SIGNUP = 'https://www.themoviedb.org/settings/api';

function MetadataSection() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: ['tmdb', 'status'], queryFn: () => invoke('tmdb_status', {}) });
  const [key, setKey] = useState('');
  const [error, setError] = useState<string | null>(null);
  const done = () => {
    void qc.invalidateQueries({ queryKey: ['tmdb'] });
    // Details and VOD pages fetched without (or with another) key should look again.
    void qc.invalidateQueries({ queryKey: ['details'] });
    void qc.invalidateQueries({ queryKey: ['catalog'] });
  };
  const save = useMutation({
    mutationFn: (k: string) => invoke('tmdb_set_key', { key: k }),
    onSuccess: () => {
      setKey('');
      setError(null);
      done();
    },
    onError: (e: unknown) => setError(toAppError(e).message),
  });
  const remove = useMutation({ mutationFn: () => invoke('tmdb_clear_key', {}), onSuccess: done, onError: notifyError });
  if (!status.data) return <SkeletonList rows={1} label={t('loading.generic')} />;
  const configured = status.data.configured;
  return (
    <Section title={t('settings.metadata')} help={t('settings.metadata.help')}>
      <SettingRow label={t('settings.tmdb')} help={error ?? (configured ? t('settings.tmdb.saved') : t('settings.tmdb.help'))} wide>
        {configured ? (
          <DLButton variant="danger" size="sm" loading={remove.isPending} onClick={() => remove.mutate()}>
            {t('settings.tmdb.remove')}
          </DLButton>
        ) : (
          <form
            className={classes.inlineForm}
            onSubmit={(e) => {
              e.preventDefault();
              if (key.trim()) save.mutate(key.trim());
            }}
          >
            <PasswordInput
              aria-label={t('settings.tmdb')}
              placeholder={t('settings.tmdb.placeholder')}
              value={key}
              error={!!error}
              autoComplete="off"
              spellCheck={false}
              onChange={(e) => {
                setKey(e.currentTarget.value);
                setError(null);
              }}
            />
            <DLButton type="submit" variant="filled" loading={save.isPending} disabled={!key.trim()}>
              {t('settings.tmdb.save')}
            </DLButton>
            <DLButton variant="subtle" onClick={() => void invoke('open_external_link', { url: TMDB_SIGNUP }).catch(notifyError)}>
              {t('settings.tmdb.getKey')}
            </DLButton>
          </form>
        )}
      </SettingRow>
      <p className={classes.attribution}>{t('details.tmdbAttribution')}</p>
    </Section>
  );
}

function AppearanceSection() {
  const density = useUiStore((s) => s.density);
  const setDensity = useUiStore((s) => s.setDensity);
  return (
    <Section title={t('settings.appearance')} help={t('settings.appearance.help')}>
      <SettingRow label={t('settings.density')} help={t('settings.density.help')} wide>
        <SegmentedControl
          aria-label={t('settings.density')}
          value={density}
          onChange={(v) => setDensity(v as DensitySetting)}
          data={[
            { value: 'compact', label: t('settings.density.compact') },
            { value: 'comfortable', label: t('settings.density.comfortable') },
            { value: 'tv', label: t('settings.density.tv') },
          ]}
        />
      </SettingRow>
    </Section>
  );
}

function AboutSection() {
  const diag = useQuery({ queryKey: ['diagnostics', 'about'], queryFn: () => invoke('get_diagnostics', {}) });
  const { data: settings } = useSettings();
  const qc = useQueryClient();
  const [copied, setCopied] = useState(false);
  const [exported, setExported] = useState<{ path: string } | { error: true } | null>(null);
  const exportDiag = useMutation({
    mutationFn: () => invoke('export_diagnostics', {}),
    onSuccess: (path) => setExported({ path }),
    onError: () => setExported({ error: true }),
  });
  const debug = useMutation({
    mutationFn: (enabled: boolean) => invoke('set_debug_logging', { enabled }),
    onSuccess: (_r, enabled) => qc.setQueryData(['settings'], { ...settings, debugLogging: enabled }),
  });
  const d = diag.data;
  return (
    <Section title={t('settings.about')} help={t('settings.about.help')}>
      <SettingRow label={t('settings.about.app')} wide><span className={classes.mono}>{d?.appVersion ?? '–'}</span></SettingRow>
      <SettingRow label={t('settings.about.servers')} wide>
        <span className={classes.mono}>{d?.serverVersions.length ? d.serverVersions.join(', ') : t('settings.none')}</span>
      </SettingRow>
      <SettingRow label={t('settings.about.hwdec')} wide><span className={classes.mono}>{d?.hwdecCurrent ?? '–'}</span></SettingRow>
      <SettingRow label={t('settings.about.os')} wide><span className={classes.mono}>{d?.os ?? '–'}</span></SettingRow>
      <SettingRow label={t('settings.debug')} help={t('settings.debug.help')} wide>
        <Switch
          aria-label={t('settings.debug')}
          checked={settings?.debugLogging ?? false}
          onChange={(e) => debug.mutate(e.currentTarget.checked)}
        />
      </SettingRow>
      <SettingRow label={t('settings.copy')} help={t('settings.copy.help')} wide>
        <DLButton
          variant="default"
          onClick={() =>
            void copyDiagnostics().then((ok) => {
              setCopied(ok);
              setTimeout(() => setCopied(false), 2000);
            })
          }
        >
          {copied ? t('settings.copied') : t('settings.copy')}
        </DLButton>
      </SettingRow>
      <SettingRow label={t('settings.export')} help={t('settings.export.help')} wide>
        <div className={classes.exportRow}>
          <DLButton variant="default" loading={exportDiag.isPending} onClick={() => exportDiag.mutate()}>
            {t('settings.export')}
          </DLButton>
          {exported && 'path' in exported && (
            <span role="status" className={classes.exportResult}>
              <span className={classes.mono}>{exported.path}</span>
              <DLButton variant="subtle" size="sm" onClick={() => void invoke('reveal_path', { path: exported.path })}>
                {t('settings.export.reveal')}
              </DLButton>
            </span>
          )}
          {exported && 'error' in exported && <span role="alert">{t('settings.export.failed')}</span>}
        </div>
      </SettingRow>
    </Section>
  );
}

export function SettingsPage() {
  const { section } = useParams();
  const updateWaiting = useUpdateWaiting();
  if (!section || !SECTIONS.some((s) => s.id === section)) return <Navigate to="/settings/sources" replace />;
  return (
    <div className={classes.page}>
      <nav className={classes.nav} aria-label={t('settings.nav')}>
        <h1 className={classes.navTitle}>{t('settings.title')}</h1>
        {SECTIONS.map((s) => (
          <NavLink
            key={s.id}
            component={Link}
            to={`/settings/${s.id}`}
            active={section === s.id}
            aria-current={section === s.id ? 'page' : undefined}
            className={classes.navItem}
            label={t(s.label)}
            aria-label={s.id === 'updates' && updateWaiting ? `${t(s.label)} · ${t('settings.updates.navWaiting')}` : undefined}
            leftSection={<Icon icon={s.icon} size={20} />}
            rightSection={
              s.id === 'updates' && updateWaiting ? <span className={classes.navDot} data-testid="settings-update-dot" /> : undefined
            }
          />
        ))}
      </nav>
      <div className={classes.content}>
        <div className={classes.inner}>
          {section === 'sources' && <SourcesSection />}
          {section === 'playback' && <PlaybackSection />}
          {section === 'sports' && <SportsSection />}
          {section === 'metadata' && <MetadataSection />}
          {section === 'mediaserver' && <MediaServerSection />}
          {section === 'appearance' && <AppearanceSection />}
          {section === 'updates' && <UpdatesSection />}
          {section === 'about' && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
