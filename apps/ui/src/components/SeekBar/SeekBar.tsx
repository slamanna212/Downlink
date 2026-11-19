import { useRef, useState } from 'react';
import { formatDuration } from '../../lib/time';
import classes from './SeekBar.module.css';

/** Seconds for a pointer ratio, clamped to the duration. Pure (unit-tested). */
export function positionFromRatio(ratio: number, duration: number): number {
  return Math.min(Math.max(ratio, 0), 1) * Math.max(duration, 0);
}

export interface SeekBarProps {
  position: number;
  duration: number;
  /** Buffered-up-to position (seconds). */
  buffered?: number;
  disabled?: boolean;
  label: string;
  onSeek: (seconds: number) => void;
}

/** Seekable timeline: position in accent, buffered in white 32%, thumb and a time tooltip on hover. */
export function SeekBar({ position, duration, buffered = 0, disabled, label, onSeek }: SeekBarProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [hover, setHover] = useState<number | null>(null); // ratio
  const [dragging, setDragging] = useState(false);
  const pct = (s: number) => (duration > 0 ? Math.min(100, Math.max(0, (s / duration) * 100)) : 0);

  const ratioOf = (clientX: number) => {
    const r = ref.current?.getBoundingClientRect();
    return r && r.width > 0 ? (clientX - r.left) / r.width : 0;
  };

  return (
    <div
      ref={ref}
      className={classes.root}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Math.round(position)}
      aria-valuetext={formatDuration(position)}
      aria-disabled={disabled || undefined}
      data-disabled={disabled || undefined}
      data-dragging={dragging || undefined}
      onPointerMove={(e) => {
        const r = Math.min(Math.max(ratioOf(e.clientX), 0), 1);
        setHover(r);
        if (dragging) onSeek(positionFromRatio(r, duration));
      }}
      onPointerLeave={() => setHover(null)}
      onPointerDown={(e) => {
        if (disabled) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(true);
        onSeek(positionFromRatio(ratioOf(e.clientX), duration));
      }}
      onPointerUp={() => setDragging(false)}
    >
      <div className={classes.track}>
        <div className={classes.buffered} style={{ width: `${pct(buffered)}%` }} />
        <div className={classes.played} style={{ width: `${pct(position)}%` }} />
        <div className={classes.thumb} style={{ left: `${pct(position)}%` }} />
      </div>
      {hover !== null && !disabled && (
        <div className={classes.tip} style={{ left: `${hover * 100}%` }}>
          {formatDuration(positionFromRatio(hover, duration))}
        </div>
      )}
    </div>
  );
}
