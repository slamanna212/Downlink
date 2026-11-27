import { useState } from 'react';
import { useProgrammeArt } from '../../api/queries';
import { resolveImg } from '../../lib/dlimg';
import { logoColor } from '../../lib/logo';
import classes from './ProgrammeArt.module.css';

export interface ProgrammeArtProps {
  /** Guide programme to look up (core: guide icon, then server, then TMDB; cached). */
  programmeId?: string | null;
  /** Artwork already known (e.g. a continue-watching item); skips the lookup. */
  src?: string | null;
  /** Tints the gradient fallback with the channel's palette colour. */
  channelId?: number | string | null;
  className?: string;
  /** `backdrop` (heroes): picture weighted to the right. `cover` (cards): fills the box. */
  fit?: 'backdrop' | 'cover';
  /** Leave no empty art slot when there is no usable image. */
  hideFallback?: boolean;
  children?: React.ReactNode;
}

/**
 * Programme artwork with a gradient fallback. The gradient always paints first and stays when there is
 * no picture or it fails to load, so the box is never grey or broken (DESIGN §2, logo fallback).
 */
export function ProgrammeArt({ programmeId, src, channelId, className, fit = 'cover', hideFallback = false, children }: ProgrammeArtProps) {
  const art = useProgrammeArt(src ? null : programmeId);
  const url = resolveImg(src ?? art.data ?? null);
  const [failed, setFailed] = useState<string | null>(null);
  const tint = channelId != null ? logoColor(channelId) : undefined;
  if (hideFallback && (!url || failed === url)) return null;
  return (
    <div className={`${classes.art} ${className ?? ''}`} data-fit={fit} style={tint ? { ['--art-tint' as string]: tint } : undefined}>
      {url && failed !== url && <img className={classes.img} src={url} alt="" loading="lazy" draggable={false} onError={() => setFailed(url)} />}
      {children}
    </div>
  );
}
