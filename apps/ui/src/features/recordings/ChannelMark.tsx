import { useState } from 'react';
import { initialsOf, logoColor } from '../../lib/logo';
import { resolveImg } from '../../lib/dlimg';
import type { ShowChannel } from './shows';
import classes from './Recordings.module.css';

export type ChannelMarkSize = 'xs' | 'sm' | 'md' | 'lg';

/**
 * Small square channel mark for DVR lists (16 / 22 / 24 / 40 px): the logo on a surface-3 plate, or initials
 * on the channel's palette colour (DESIGN §2 logo fallback, never a grey box).
 */
export function ChannelMark({ channel, size }: { channel: ShowChannel; size: ChannelMarkSize }) {
  const [failed, setFailed] = useState<string | null>(null);
  const src = channel.logo && failed !== channel.logo ? resolveImg(channel.logo) : null;
  return (
    <span
      className={classes.mark}
      data-size={size}
      data-fallback={src ? undefined : 'true'}
      style={src ? undefined : { background: logoColor(channel.id ?? channel.name) }}
      aria-hidden
    >
      {src ? <img src={src} alt="" loading="lazy" draggable={false} onError={() => setFailed(channel.logo)} /> : initialsOf(channel.name)}
    </span>
  );
}

/** Up to two marks, overlapped (a show recorded from several channels). */
export function ChannelStack({ channels, size }: { channels: ShowChannel[]; size: ChannelMarkSize }) {
  return (
    <span className={classes.stack} aria-hidden>
      {channels.slice(0, 2).map((c) => (
        <ChannelMark key={c.name} channel={c} size={size} />
      ))}
    </span>
  );
}
