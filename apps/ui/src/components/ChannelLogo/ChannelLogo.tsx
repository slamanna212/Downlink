import { useState } from 'react';
import { dl } from '../../theme';
import { useUiStore } from '../../store/ui';
import { initialsOf, logoColor } from '../../lib/logo';
import { t } from '../../lib/i18n';
import classes from './ChannelLogo.module.css';

export interface ChannelLogoProps {
  channelId: string | number;
  name: string;
  /** Resolved image URL. Missing or failing images fall back to initials, never a grey box. */
  src?: string | null;
  /** Defaults to the density setting. */
  density?: keyof typeof dl.density;
}

export function ChannelLogo({ channelId, name, src, density }: ChannelLogoProps) {
  const current = useUiStore((s) => s.density);
  const [w, h] = dl.density[density ?? current].logo;
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const showImage = !!src && failedSrc !== src;

  return (
    <div
      className={classes.plate}
      style={showImage ? { width: w, height: h } : { width: w, height: h, background: logoColor(channelId) }}
      data-fallback={showImage ? undefined : 'true'}
    >
      {showImage ? (
        <img className={classes.img} src={src} alt={t('channel.logoAlt', { name })} loading="lazy" onError={() => setFailedSrc(src)} />
      ) : (
        <span className={classes.fallback} style={{ fontSize: Math.round(h * 0.4) }} aria-label={name} role="img">
          {initialsOf(name)}
        </span>
      )}
    </div>
  );
}
