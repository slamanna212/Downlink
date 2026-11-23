import { Menu } from '@mantine/core';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { IconArrowsExchange, IconBadgeCc, IconCheck, IconLanguage } from '@tabler/icons-react';
import { invoke, toAppError } from '../../api/ipc';
import type { AltStream, Track } from '../../api/types';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { notifyError } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import classes from './Player.module.css';

const AUDIO_CODECS: Record<string, string> = {
  ac3: 'AC-3', eac3: 'E-AC-3', aac: 'AAC', aac_latm: 'AAC', mp2: 'MP2', mp3: 'MP3', dts: 'DTS', truehd: 'TrueHD', opus: 'Opus', flac: 'FLAC', vorbis: 'Vorbis',
};

let languageNames: Intl.DisplayNames | null | undefined;

/** "eng" → "English"; unknown or invalid codes come back upper-cased ("QAA"). */
export function languageName(code: string): string {
  if (languageNames === undefined) {
    try {
      languageNames = new Intl.DisplayNames(['en'], { type: 'language' });
    } catch {
      languageNames = null;
    }
  }
  try {
    const name = languageNames?.of(code);
    if (name && name.toLowerCase() !== code.toLowerCase()) return name;
  } catch {
    // invalid language tag: fall through
  }
  return code.toUpperCase();
}

/** "English · Main · AC-3 5.1"; falls back to "Track 2". Channels follow the codec, as in the stream picker. */
export function trackLabel(track: Track): string {
  const codec = track.codec ? (AUDIO_CODECS[track.codec.toLowerCase()] ?? track.codec.replace(/_/g, ' ').toUpperCase()) : null;
  const format = [codec, track.kind === 'audio' ? track.channels : null].filter(Boolean).join(' ');
  const parts = [track.lang ? languageName(track.lang) : null, track.title, format || null].filter((p): p is string => !!p);
  return parts.length ? parts.join(' · ') : t('player.track', { n: track.id });
}

const VIDEO_CODECS: Record<string, string> = { h264: 'H.264', avc: 'H.264', hevc: 'HEVC', h265: 'HEVC', mpeg2video: 'MPEG-2', av1: 'AV1', vp9: 'VP9' };

/** Trim to at most two decimals without trailing zeros: 30 → "30", 29.97 → "29.97". */
const num = (n: number) => String(Number(n.toFixed(2)));

/** Second line of a stream picker row: "1080p · 29.97 fps · H.264 · AAC stereo · 6.5 Mbps · 1 watching". */
export function streamDetails(s: AltStream): string {
  const height = s.resolution?.match(/^\s*\d+\s*x\s*(\d+)\s*$/i)?.[1];
  const audio = [s.audioCodec?.toUpperCase(), s.audioChannels].filter(Boolean).join(' ');
  const bitrate =
    s.bitrateKbps == null
      ? null
      : s.bitrateKbps >= 1000
        ? t('player.streamMbps', { n: String(Number((s.bitrateKbps / 1000).toFixed(1))) })
        : t('player.streamKbps', { n: Math.round(s.bitrateKbps) });
  const parts = [
    s.stale ? t('player.streamStale') : null,
    height ? `${height}p` : s.resolution,
    s.fps != null ? t('player.streamFps', { n: num(s.fps) }) : null,
    s.videoCodec ? (VIDEO_CODECS[s.videoCodec.toLowerCase()] ?? s.videoCodec.toUpperCase()) : null,
    audio || null,
    bitrate,
    s.viewers ? t('player.streamViewers', { n: s.viewers }) : null,
  ].filter((p): p is string => !!p);
  return parts.length ? parts.join(' · ') : t('player.streamNoInfo');
}

interface MenuProps {
  opened: boolean;
  onChange: (open: boolean) => void;
}

function Check({ on }: { on: boolean }) {
  return <span className={classes.menuCheck}>{on && <Icon icon={IconCheck} size={16} />}</span>;
}

/** Audio track menu (`A`). Hidden by the caller when the file has no audio track; a single track is still listed so the user can see what plays. */
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
        <DLIconButton label={t('player.streams')} icon={IconArrowsExchange} variant="overlay" size="lg" pressed={opened} />
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
            <span className={classes.streamName}>
              {s.name}
              {s.provider && <span className={classes.streamProvider}> · {s.provider}</span>}
            </span>
            <span className={classes.streamMeta}>{streamDetails(s)}</span>
          </Menu.Item>
        ))}
        <Menu.Divider />
        <div className={classes.menuNote}>{t('player.streamsNote')}</div>
      </Menu.Dropdown>
    </Menu>
  );
}

/** "2 of 3" for the stream in use; null (row hidden) when the position or total is unknown. */
export function streamPosition(stats: { sourceIndex: number | null; sourceCount: number | null } | null | undefined): string | null {
  const n = stats?.sourceIndex;
  const total = stats?.sourceCount;
  if (n == null || total == null || total < 1 || n < 1 || n > total) return null;
  return t('player.info.streamOf', { n, total });
}
