// Pure guide geometry and navigation logic: no DOM reads, unit-tested. The view only calls these
// with numbers it already has (scroll offsets from the scroll event, sizes from ResizeObserver).
import type { Channel, Programme } from '../../api/types';
import { floorToTick, formatClock, formatDay, startOfDay } from '../../lib/time';

export const CHANNEL_CHUNK = 40; // channels per get_guide call
export const TIME_CHUNK = 6 * 3600; // seconds per get_guide call
export const SPANS_HOURS = [3, 6, 12, 24] as const;
export type SpanHours = (typeof SPANS_HOURS)[number];

export interface Geometry {
  /** UTC seconds at x = colW. */
  rangeStart: number;
  rangeEnd: number;
  /** Pixels per second. */
  pps: number;
  colW: number;
  headerH: number;
  rowH: number;
}

export const timeToX = (g: Geometry, t: number) => g.colW + (t - g.rangeStart) * g.pps;
export const xToTime = (g: Geometry, x: number) => g.rangeStart + (x - g.colW) / g.pps;
export const totalWidth = (g: Geometry) => timeToX(g, g.rangeEnd);
export const totalHeight = (g: Geometry, rows: number) => g.headerH + rows * g.rowH;
export const rowTop = (g: Geometry, row: number) => row * g.rowH;

/** Pixels per second so that `spanHours` fit in the scrollable area right of the channel column. */
export const ppsFor = (viewW: number, colW: number, spanHours: number) => Math.max(viewW - colW, 100) / (spanHours * 3600);

export interface Viewport {
  scrollLeft: number;
  scrollTop: number;
  viewW: number;
  viewH: number;
}

export interface RenderRange {
  t0: number;
  t1: number;
  r0: number;
  r1: number;
}

/** What is on screen (exact, no margin). */
export function visibleRange(g: Geometry, v: Viewport, rows: number): RenderRange {
  const r0 = Math.max(0, Math.floor(v.scrollTop / g.rowH));
  const r1 = Math.min(rows - 1, Math.ceil((v.scrollTop + v.viewH - g.headerH) / g.rowH));
  return {
    t0: xToTime(g, v.scrollLeft + g.colW),
    t1: xToTime(g, v.scrollLeft + v.viewW),
    r0,
    r1,
  };
}

/** The range to render: the visible one plus half a viewport of time and a few rows of margin. */
export function withMargin(r: RenderRange, rows: number, rowMargin = 10): RenderRange {
  const dt = (r.t1 - r.t0) / 2;
  return { t0: r.t0 - dt, t1: r.t1 + dt, r0: Math.max(0, r.r0 - rowMargin), r1: Math.min(rows - 1, r.r1 + rowMargin) };
}

/** True while `visible` is still inside the cached render range, so no re-render is needed. */
export const isCovered = (cached: RenderRange | null, visible: RenderRange): boolean =>
  cached !== null && visible.t0 >= cached.t0 && visible.t1 <= cached.t1 && visible.r0 >= cached.r0 && visible.r1 <= cached.r1;

/** Chunk indices needed to cover a render range. */
export function neededChunks(r: RenderRange, rangeStart: number): { c: number[]; t: number[] } {
  const range = (a: number, b: number) => Array.from({ length: Math.max(0, b - a + 1) }, (_, i) => a + i);
  return {
    c: range(Math.floor(r.r0 / CHANNEL_CHUNK), Math.floor(r.r1 / CHANNEL_CHUNK)),
    t: range(Math.max(0, Math.floor((r.t0 - rangeStart) / TIME_CHUNK)), Math.max(0, Math.floor((r.t1 - rangeStart) / TIME_CHUNK))),
  };
}

export type BlockState = 'past-archive' | 'past-none' | 'now' | 'future';

export function archiveAvailable(p: Programme, ch: Channel | undefined, now: number): boolean {
  return !!ch && ch.isCatchup && ch.catchupDays > 0 && p.endUtc <= now && p.startUtc >= now - ch.catchupDays * 86400;
}

export function blockState(p: Programme, ch: Channel | undefined, now: number): BlockState {
  if (p.startUtc <= now && p.endUtc > now) return 'now';
  if (p.startUtc > now) return 'future';
  return archiveAvailable(p, ch, now) ? 'past-archive' : 'past-none';
}

/** What Enter does on a block (PLAN §8.2). */
export type EnterAction = 'play-live' | 'play-catchup' | 'details' | 'none';
export function enterAction(state: BlockState): EnterAction {
  return state === 'now' ? 'play-live' : state === 'past-archive' ? 'play-catchup' : state === 'future' ? 'details' : 'none';
}

/** Index of the programme covering `t` in a start-sorted list, or the nearest following one; -1 if none. */
export function indexAt(list: readonly Programme[], t: number): number {
  let lo = 0;
  let hi = list.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const p = list[mid]!;
    if (p.endUtc <= t) lo = mid + 1;
    else {
      ans = mid;
      hi = mid - 1;
    }
  }
  return ans;
}

/** Programmes of a list that intersect [t0, t1). */
export function sliceRange(list: readonly Programme[], t0: number, t1: number): Programme[] {
  const i = indexAt(list, t0);
  if (i < 0) return [];
  const out: Programme[] = [];
  for (let k = i; k < list.length && list[k]!.startUtc < t1; k++) out.push(list[k]!);
  return out;
}

/** Merge programme chunks into per-channel sorted, de-duplicated lists. */
export function mergeProgrammes(chunks: readonly (readonly Programme[])[]): Map<number, Programme[]> {
  const byId = new Map<string, Programme>();
  for (const c of chunks) for (const p of c) byId.set(p.id, p);
  const out = new Map<number, Programme[]>();
  for (const p of byId.values()) {
    const l = out.get(p.channelId);
    if (l) l.push(p);
    else out.set(p.channelId, [p]);
  }
  for (const l of out.values()) l.sort((a, b) => a.startUtc - b.startUtc);
  return out;
}

export type FocusKey = 'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown' | 'PageUp' | 'PageDown';

export interface Focus {
  row: number;
  /** Focus time: the programme covering it in the row is focused. */
  t: number;
}

/**
 * Next focus for a key. Left/right hop to the neighbouring programme in the row; up/down keep the
 * focus time (clamped to the visible window start) and change row; PgUp/PgDn move `page` rows.
 */
export function moveFocus(
  f: Focus,
  key: FocusKey,
  rows: number,
  page: number,
  listFor: (row: number) => readonly Programme[] | undefined,
  viewLeftTime: number,
): Focus {
  const list = listFor(f.row) ?? [];
  const i = indexAt(list, f.t);
  switch (key) {
    case 'ArrowRight': {
      const next = i >= 0 ? list[i + 1] : undefined;
      return next ? { row: f.row, t: next.startUtc } : f;
    }
    case 'ArrowLeft': {
      const cur = i >= 0 ? list[i] : undefined;
      const prev = cur ? list[i - 1] : list[list.length - 1];
      return prev ? { row: f.row, t: prev.startUtc } : f;
    }
    default: {
      const delta = key === 'ArrowUp' ? -1 : key === 'ArrowDown' ? 1 : key === 'PageUp' ? -page : page;
      const row = Math.min(Math.max(f.row + delta, 0), rows - 1);
      return { row, t: Math.max(f.t, viewLeftTime) };
    }
  }
}

/** Scroll offsets that bring a block fully into view (with padding), or null when already visible. */
export function scrollToReveal(
  g: Geometry,
  v: Viewport,
  block: { start: number; end: number; row: number },
  pad = 16,
): { left?: number; top?: number } {
  const out: { left?: number; top?: number } = {};
  const x0 = timeToX(g, block.start);
  const x1 = timeToX(g, block.end);
  if (x0 < v.scrollLeft + g.colW + pad) out.left = Math.max(0, x0 - g.colW - pad);
  else if (x1 > v.scrollLeft + v.viewW - pad) out.left = Math.max(0, Math.min(x1 - v.viewW + pad, x0 - g.colW - pad));
  const y0 = block.row * g.rowH;
  const y1 = y0 + g.rowH;
  if (y0 < v.scrollTop) out.top = y0;
  else if (y1 > v.scrollTop + v.viewH - g.headerH) out.top = y1 - (v.viewH - g.headerH);
  return out;
}

export interface TickMark {
  t: number;
  /** Day label at local midnight, clock label otherwise. */
  label: string;
  isDay: boolean;
}

/** 30-minute ticks in [t0, t1] clamped to the guide range; a day separator at each local midnight. */
export function tickMarks(g: Pick<Geometry, 'rangeStart' | 'rangeEnd'>, t0: number, t1: number): TickMark[] {
  const out: TickMark[] = [];
  for (let t = floorToTick(t0); t <= t1; t += 1800) {
    if (t < g.rangeStart || t > g.rangeEnd) continue;
    const isDay = t === startOfDay(t);
    out.push({ t, isDay, label: isDay ? formatDay(t) : formatClock(t) });
  }
  return out;
}
