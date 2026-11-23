import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import type { Timeline as TimelineState } from '../../api/types';
import { epochAtRatio, formatClockSeconds, hoursToSeconds, programmeSegments, ratioOfEpoch, timelineWindow, type Epoch } from '../../lib/time';
import { smoothStep } from '../../lib/liveEdge';
import classes from './Timeline.module.css';

/** Whole bar: one hour, the playhead always in the middle. */
export const TIMELINE_SPAN_SECONDS = hoursToSeconds(1);
/** Pointer travel (px) below which a press is a click, not a drag. */
const DRAG_THRESHOLD_PX = 4;

export interface TimelineProgramme {
  startUtc: Epoch;
  endUtc: Epoch;
  title: string | null;
}

export interface TimelineProps {
  timeline: TimelineState;
  /** Guide entries around the playhead; the bar is split at their boundaries. Empty = no gaps. */
  programmes: TimelineProgramme[];
  /** A seek in flight: centre on this moment until the player gets there. */
  pendingUtc?: Epoch | null;
  spanSeconds?: number;
  disabled?: boolean;
  /** Media is playing: the bar glides at real time between player updates instead of stepping. */
  playing?: boolean;
  label: string;
  onSeekTo: (utc: Epoch) => void;
}

/**
 * Playhead and live edge shown on the bar. Player updates arrive a few times a second and are noisy (the cache
 * end moves in chunks, the anchor re-bases), so while playing both advance at real time on animation frames and
 * drift gently towards the reported values; they never move backwards unless the user seeks.
 */
function useGlidingTimeline(timeline: TimelineState, playing: boolean): { playhead: number; edge: number } {
  const raw = useRef({ timeline, at: 0 });
  const [shown, setShown] = useState({ playhead: timeline.playheadUtc, edge: timeline.liveEdgeUtc });
  useEffect(() => {
    raw.current = { timeline, at: performance.now() };
  }, [timeline, playing]);
  useEffect(() => {
    if (!playing) return undefined;
    let frame = 0;
    let last = performance.now();
    let prev: { playhead: number; edge: number } | null = null;
    const tick = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      const { timeline: tl, at } = raw.current;
      const age = (now - at) / 1000;
      const playhead = smoothStep(prev?.playhead ?? null, tl.playheadUtc + age, dt, true);
      const edge = Math.max(playhead, smoothStep(prev?.edge ?? null, tl.liveEdgeUtc + age, dt, true));
      prev = { playhead, edge };
      setShown(prev);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing]);
  return playing ? shown : { playhead: timeline.playheadUtc, edge: timeline.liveEdgeUtc };
}

const clamp01 = (r: number) => Math.min(1, Math.max(0, r));
const pct = (r: number) => `${(r * 100).toFixed(4)}%`;

/**
 * Channel timeline: a window of wall-clock time that slides under a fixed centre thumb. Left of the thumb is
 * what has been watched or can be rewound to (local buffer, then the archive); right of it, up to the live
 * edge, is what can be skipped ahead to. Gaps mark programme boundaries.
 */
export function Timeline({ timeline, programmes, pendingUtc, spanSeconds = TIMELINE_SPAN_SECONDS, disabled, playing = false, label, onSeekTo }: TimelineProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const drag = useRef<{ x: number; moved: boolean } | null>(null);
  const [dragSecs, setDragSecs] = useState(0);
  const [hover, setHover] = useState<number | null>(null);

  const glide = useGlidingTimeline(timeline, playing);
  const center = (pendingUtc ?? glide.playhead) + dragSecs;
  const win = timelineWindow(center, spanSeconds);
  const oldest = timeline.archiveStartUtc ?? timeline.bufferStartUtc ?? timeline.playheadUtc;
  // Windows move every frame; segment math only needs the window rounded to the second.
  const winStart = Math.floor(win.start);
  const segments = useMemo(
    () => programmeSegments(programmes, timelineWindow(winStart + spanSeconds / 2, spanSeconds)),
    [programmes, winStart, spanSeconds],
  );
  const available = clamp01(ratioOfEpoch(win, oldest));
  const edge = clamp01(ratioOfEpoch(win, glide.edge));
  const width = () => ref.current?.getBoundingClientRect().width ?? 0;
  const ratioOf = (clientX: number) => {
    const r = ref.current?.getBoundingClientRect();
    return r && r.width > 0 ? clamp01((clientX - r.left) / r.width) : 0.5;
  };
  const hoverUtc = hover === null ? null : epochAtRatio(win, hover);
  const hoverTitle = hover === null ? null : (segments.find((s) => hover >= s.start && hover < s.end)?.title ?? null);

  return (
    <div
      ref={ref}
      className={classes.root}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={Math.round(oldest)}
      aria-valuemax={Math.round(timeline.liveEdgeUtc)}
      aria-valuenow={Math.round(center)}
      aria-valuetext={formatClockSeconds(Math.round(center))}
      aria-disabled={disabled || undefined}
      data-disabled={disabled || undefined}
      data-dragging={dragSecs !== 0 || undefined}
      onPointerMove={(e) => {
        setHover(ratioOf(e.clientX));
        const d = drag.current;
        if (!d) return;
        const dx = e.clientX - d.x;
        if (!d.moved && Math.abs(dx) < DRAG_THRESHOLD_PX) return;
        d.moved = true;
        const w = width();
        // Dragging the track right pulls the past under the thumb.
        if (w > 0) setDragSecs((-dx / w) * spanSeconds);
      }}
      onPointerLeave={() => setHover(null)}
      onPointerDown={(e) => {
        if (disabled) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        drag.current = { x: e.clientX, moved: false };
      }}
      onPointerUp={(e) => {
        const d = drag.current;
        drag.current = null;
        if (!d || disabled) return;
        if (d.moved) onSeekTo(center);
        else onSeekTo(epochAtRatio(win, ratioOf(e.clientX)));
        setDragSecs(0);
      }}
      onPointerCancel={() => {
        drag.current = null;
        setDragSecs(0);
      }}
    >
      <div className={classes.track}>
        {segments.map((s, i) => (
          <div
            key={`${i}:${s.title ?? ''}`}
            className={classes.segment}
            data-first={i === 0 || undefined}
            data-last={i === segments.length - 1 || undefined}
            data-hovered={(hover !== null && hover >= s.start && hover < s.end) || undefined}
            style={{ '--seg-start': s.start, '--seg-end': s.end } as CSSProperties}
          >
            <div className={classes.layers}>
              {edge > available && <div className={classes.available} style={{ left: pct(available), width: pct(edge - available) }} />}
              {available < 0.5 && <div className={classes.played} style={{ left: pct(available), width: pct(0.5 - available) }} />}
              {edge > 0.5 && <div className={classes.ahead} style={{ left: '50%', width: pct(edge - 0.5) }} />}
            </div>
          </div>
        ))}
        {edge > 0 && edge < 1 && <div className={classes.liveEdge} style={{ left: pct(edge) }} aria-hidden />}
        <div className={classes.thumb} aria-hidden />
      </div>
      {hover !== null && hoverUtc !== null && !disabled && (
        <div className={classes.tip} style={{ left: pct(hover) }}>
          {formatClockSeconds(Math.round(hoverUtc))}
          {hoverTitle && <span className={classes.tipTitle}>{hoverTitle}</span>}
        </div>
      )}
    </div>
  );
}
