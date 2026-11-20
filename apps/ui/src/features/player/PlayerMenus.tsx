import { Menu } from '@mantine/core';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { IconArrowsShuffle, IconBadgeCc, IconCheck, IconLanguage } from '@tabler/icons-react';
import { invoke, toAppError } from '../../api/ipc';
import type { Track } from '../../api/types';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { notifyError } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import classes from './Player.module.css';

/** "ENG · Main · AC3"; falls back to "Track 2". */
export function trackLabel(track: Track): string {
  const parts = [track.lang?.toUpperCase(), track.title, track.codec?.replace(/_/g, ' ').toUpperCase()].filter(
    (p): p is string => !!p,
  );
  return parts.length ? parts.join(' · ') : t('player.track', { n: track.id });
}

interface MenuProps {
  opened: boolean;
  onChange: (open: boolean) => void;
}

function Check({ on }: { on: boolean }) {
  return <span className={classes.menuCheck}>{on && <Icon icon={IconCheck} size={16} />}</span>;
}

/** Audio track menu (`A`). Hidden by the caller when the file has fewer than two audio tracks. */
export function AudioMenu({ tracks, opened, onChange }: MenuProps & { tracks: Track[] }) {
  const audio = tracks.filter((x) => x.kind === 'audio');
  return (
    <Menu opened={opened} onChange={onChange} position="top-end" withinPortal>
      <Menu.Target>
        <DLIconButton label={t('player.audioTracks')} icon={IconLanguage} variant="overlay" size="lg" pressed={opened} />
      </Menu.Target>
      <Menu.Dropdown aria-label={t('player.audioTracks')}>
        <Menu.Label>{t('player.audioTracks')}</Menu.Label>
        {audio.map((x) => (
          <Menu.Item
            key={x.id}
            leftSection={<Check on={x.selected} />} aria-current={x.selected || undefined}
            onClick={() => void invoke('player_command', { command: { kind: 'setAudioTrack', id: x.id } })}
          >
            {trackLabel(x)}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

/** Subtitle menu (`T`): Off plus every subtitle track (teletext and DVB subtitles included). */
export function SubtitleMenu({ tracks, opened, onChange }: MenuProps & { tracks: Track[] }) {
  const subs = tracks.filter((x) => x.kind === 'sub');
  const none = !subs.some((x) => x.selected);
  const pick = (id: number | null) => void invoke('player_command', { command: { kind: 'setSubtitleTrack', id } });
  return (
    <Menu opened={opened} onChange={onChange} position="top-end" withinPortal>
      <Menu.Target>
        <DLIconButton label={t('player.subtitles')} icon={IconBadgeCc} variant="overlay" size="lg" pressed={opened || !none} />
      </Menu.Target>
      <Menu.Dropdown aria-label={t('player.subtitles')}>
        <Menu.Label>{t('player.subtitles')}</Menu.Label>
        <Menu.Item leftSection={<Check on={none} />} aria-current={none || undefined} onClick={() => pick(null)}>
          {t('player.subtitlesOff')}
        </Menu.Item>
        {subs.map((x) => (
          <Menu.Item key={x.id} leftSection={<Check on={x.selected} />} aria-current={x.selected || undefined} onClick={() => pick(x.id)}>
            {trackLabel(x)}
          </Menu.Item>
        ))}
      </Menu.Dropdown>
    </Menu>
  );
}

/**
 * Stream picker (sources with `Cap.StreamSwitch` only, live playback). Switching moves the server's channel
 * session, so the menu says it affects everyone watching (PLAN §3.4, D18). The list is SQLite plus a
 * status call: nothing here requests a stream endpoint.
 */
export function StreamMenu({ channelId, opened, onChange, onDenied }: MenuProps & { channelId: number; onDenied: () => void }) {
  const qc = useQueryClient();
  const key = ['player', 'streams', channelId];
  const list = useQuery({ queryKey: key, queryFn: () => invoke('get_channel_streams', { channelId }), enabled: opened, staleTime: 30_000 });
  const current = list.data?.sourceIndex != null ? list.data.sourceIndex - 1 : null;
  const pick = (index: number) => {
    if (index === current) return;
    invoke('switch_stream', { channelId, index })
      .then(() => qc.invalidateQueries({ queryKey: key }))
      .catch((e: unknown) => {
        // The server refused: this account can't switch after all. Hide the picker for the session.
        if (toAppError(e).code === 'auth') onDenied();
        notifyError(e);
      });
  };
  return (
    <Menu opened={opened} onChange={onChange} position="top-end" withinPortal>
      <Menu.Target>
        <DLIconButton label={t('player.streams')} icon={IconArrowsShuffle} variant="overlay" size="lg" pressed={opened} />
      </Menu.Target>
      <Menu.Dropdown aria-label={t('player.streams')} className={classes.streamMenu}>
        <Menu.Label>{t('player.streams')}</Menu.Label>
        {list.isLoading && <Menu.Item disabled>{t('player.streamsLoading')}</Menu.Item>}
        {list.data?.streams.map((s) => (
          <Menu.Item
            key={s.streamId}
            leftSection={<Check on={s.index === current} />} aria-current={s.index === current || undefined}
            onClick={() => pick(s.index)}
          >
            {s.name}
          </Menu.Item>
        ))}
        <Menu.Divider />
        <div className={classes.menuNote}>{t('player.streamsNote')}</div>
      </Menu.Dropdown>
    </Menu>
  );
}
