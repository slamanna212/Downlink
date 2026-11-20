import { useEffect, useState } from 'react';
import { Link, Navigate, useNavigate, useParams } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Modal, NavLink, SegmentedControl, Select, Slider, Switch, Text } from '@mantine/core';
import { IconPalette, IconPlayerPlay, IconPlugConnected, IconRefresh, IconStethoscope } from '@tabler/icons-react';
import { invoke, listen } from '../../api/ipc';
import { useSettings, useSources } from '../../api/queries';
import type { AudioOutput, Deinterlace, DensitySetting, HwdecSetting, Source, UpdateChannel } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Icon, type TablerIcon } from '../../components/Icon/Icon';
import { SettingRow } from '../../components/SettingRow/SettingRow';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { formatUpdated } from '../../lib/time';
import { t, type MessageKey } from '../../lib/i18n';
import { useTasksStore } from '../../store/tasks';
import { useUiStore } from '../../store/ui';
import { copyDiagnostics } from '../diagnostics/copyDiagnostics';
import classes from './Settings.module.css';

const SECTIONS: { id: string; label: MessageKey; icon: TablerIcon }[] = [
  { id: 'sources', label: 'settings.sources', icon: IconPlugConnected },
  { id: 'playback', label: 'settings.playback', icon: IconPlayerPlay },
  { id: 'appearance', label: 'settings.appearance', icon: IconPalette },
  { id: 'updates', label: 'settings.updates', icon: IconRefresh },
  { id: 'about', label: 'settings.about', icon: IconStethoscope },
];

function Section({ title, help, children }: { title: string; help: string; children: React.ReactNode }) {
  return (
    <section>
      <h2 className={classes.sectionTitle}>{title}</h2>
      <p className={classes.sectionHelp}>{help}</p>
      <div className={classes.divider} />
      {children}
    </section>
  );
}

function SourceRow({ source, onRemove }: { source: Source; onRemove: (s: Source) => void }) {
  const qc = useQueryClient();
  const task = useTasksStore((s) => s.bySource[source.id]);
  const sync = useMutation({
    mutationFn: () => invoke('sync_source', { sourceId: source.id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'sources'] }),
  });
  const syncing = source.status === 'syncing' || !!task || sync.isPending;
  const help = task
    ? t('settings.sources.syncing', { phase: task.phase })
    : source.lastSyncAt
      ? t('settings.sources.lastSync', { time: formatUpdated(source.lastSyncAt) })
      : t('settings.sources.never');
  return (
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

function PlaybackSection() {
  const { data } = useSettings();
  const patch = useSettingsPatch();
  const diag = useQuery({ queryKey: ['diagnostics', 'hwdec'], queryFn: () => invoke('get_diagnostics', {}) });
  const devices = useQuery({ queryKey: ['audioDevices'], queryFn: () => invoke('list_audio_devices', {}) });
  const [buffer, setBuffer] = useState<number | null>(null);
  const [subSize, setSubSize] = useState<number | null>(null);
  if (!data) return <SkeletonList rows={3} label={t('loading.generic')} />;
  const shown = buffer ?? data.liveBufferSecs;
  const current = diag.data?.hwdecCurrent;
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
      <SettingRow label={t('settings.buffer')} help={t('settings.buffer.help')}>
        <div className={classes.sliderRow}>
          <Slider
            min={1}
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
      <SettingRow label={t('settings.audioDevice')} help={t('settings.audioDevice.help')}>
        <Select
          w="100%"
          aria-label={t('settings.audioDevice')}
          data={(devices.data ?? []).map((d) => ({ value: d.id, label: d.name }))}
          value={data.audioDevice ?? 'default'}
          allowDeselect={false}
          onChange={(v) => patch.mutate({ audioDevice: v === 'default' ? null : v })}
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
  const status = useQuery({ queryKey: ['update'], queryFn: () => invoke('get_update_status', {}), staleTime: Infinity });
  useEffect(() => {
    let off: (() => void) | null = null;
    let cancelled = false;
    void listen('update://status', (s) => qc.setQueryData(['update'], s)).then((u) => (cancelled ? u() : (off = u)));
    return () => {
      cancelled = true;
      off?.();
    };
  }, [qc]);
  const setChannel = useMutation({
    mutationFn: (channel: UpdateChannel) => invoke('set_update_channel', { channel }),
    onSuccess: (s) => qc.setQueryData(['update'], s),
  });
  const check = useMutation({
    mutationFn: () => invoke('check_for_updates', {}),
    onSuccess: (s) => qc.setQueryData(['update'], s),
  });
  const s = status.data;
  if (!s) return <SkeletonList rows={2} label={t('loading.generic')} />;
  const help = s.available
    ? t('settings.updates.available', { available: s.available })
    : s.lastChecked
      ? t('settings.updates.upToDate', { time: formatUpdated(s.lastChecked) })
      : t('settings.updates.never');
  return (
    <Section title={t('settings.updates')} help={t('settings.updates.help')}>
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
    </Section>
  );
}

export function SettingsPage() {
  const { section } = useParams();
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
            leftSection={<Icon icon={s.icon} size={20} />}
          />
        ))}
      </nav>
      <div className={classes.content}>
        <div className={classes.inner}>
          {section === 'sources' && <SourcesSection />}
          {section === 'playback' && <PlaybackSection />}
          {section === 'appearance' && <AppearanceSection />}
          {section === 'updates' && <UpdatesSection />}
          {section === 'about' && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
