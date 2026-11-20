import { useState } from 'react';
import classes from './Poster.module.css';

/** Stable small hash (djb2) so the same title always gets the same fallback pattern. */
export function titleHash(s: string): number {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
  return h;
}

export interface PosterProps {
  src: string | null;
  title: string;
  /** 2:3 poster (default) or 16:9 still. */
  shape?: 'poster' | 'wide';
  children?: React.ReactNode;
}

/** Artwork (`dlimg://`) with a deterministic title-based fallback when missing or failing to load. */
export function Poster({ src, title, shape = 'poster', children }: PosterProps) {
  const [failed, setFailed] = useState(false);
  const show = !!src && !failed;
  return (
    <div className={`${classes.root} ${shape === 'wide' ? classes.wide : ''}`}>
      {show ? (
        <img className={classes.img} src={src} alt="" loading="lazy" decoding="async" onError={() => setFailed(true)} />
      ) : (
        <div className={`${classes.fallback} ${classes[`v${titleHash(title) % 4}`]}`} aria-hidden>
          <span className={classes.label}>{title}</span>
        </div>
      )}
      {children}
    </div>
  );
}
