import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useMutation, useQueries, useQueryClient } from '@tanstack/react-query';
import { useMergedRef } from '@mantine/hooks';
import { Menu, SegmentedControl, Select } from '@mantine/core';
import { IconCalendar, IconChevronLeft, IconChevronRight, IconFilter } from '@tabler/icons-react';
import { setFocus } from '@noriginmedia/norigin-spatial-navigation';
import { invoke } from '../../api/ipc';
import { invalidateFavourites, useCaps, useCounts, useGroups, useOffline, useSources } from '../../api/queries';
import type { Channel, Programme } from '../../api/types';
import { dl } from '../../theme';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { EmptySource } from '../../pages/EmptySource';
import { TICK_SECONDS, guideNowAnchor, addDays, dayLabel, formatClock, formatDay, nowEpoch, progress, startOfDay } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { useDLFocusable } from '../../lib/spatial';
import { useInteractive } from '../../lib/perf';
import { t } from '../../lib/i18n';
import { useDetailStore } from '../../store/detail';
import { openChannelMenu } from '../../store/contextMenu';
import { openInfo } from '../../store/info';
import { useUiStore } from '../../store/ui';
import { freshGuideView, useViewStateStore, type GuideView } from '../../store/viewState';
import { playCatchup, playChannel } from '../player/actions';
import { GuideBlock } from './GuideBlock';
import { GuidePreview, GuideRecordItems } from './GuidePreview';
import {
  CHANNEL_CHUNK, SPANS_HOURS, TIME_CHUNK, blockState, enterAction, indexAt, isCovered, mergeProgrammes, moveFocus,
  neededChunks, ppsFor, scrollToReveal, sliceRange, tickMarks, timeToX, totalHeight, totalWidth, visibleRange, withMargin, xToTime,
  type Focus, type FocusKey, type Geometry, type RenderRange, type SpanHours, type Viewport,
} from './guideModel';
import classes from './Guide.module.css';

const FWD_DAYS = 7;
const NAV_KEYS = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'PageUp', 'PageDown'];

/** A callback with a stable identity that always calls the latest closure (keeps memoised children quiet). */
function useStable<A extends unknown[], R>(fn: (...a: A) => R): (...a: A) => R {
  const ref = useRef(fn);
  useEffect(() => {
    ref.current = fn;
  });
  return useCallback((...a: A) => ref.current(...a), []);
}

interface ControlsProps {
  span: SpanHours;
  onSpan: (s: SpanHours) => void;
  groupId: number | null;
  groups: { id: number; name: string }[];
  onGroup: (id: number | null) => void;
  leftTime: number;
  days: number[];
  onJump: (t: number) => void;
  onNow: () => void;
}

/** Toolbar between the hero and the grid: day stepper, Now, channel filter, visible span. */
const GuideControls = memo(function GuideControls({ span, onSpan, groupId, groups, onGroup, leftTime, days, onJump, onNow }: ControlsProps) {
  const first = days[0] ?? leftTime;
  const last = days.at(-1) ?? leftTime;
  const step = (n: number) => onJump(Math.min(addDays(last, 1) - 1, Math.max(first, addDays(leftTime, n))));
  return (
    <div className={classes.toolbar}>
      <div className={classes.stepper}>
        <button type="button" className={classes.stepBtn} aria-label={t('guide.prevDay')} disabled={startOfDay(leftTime) <= first} onClick={() => step(-1)}>
          <IconChevronLeft size={16} stroke={dl.iconStroke} aria-hidden />
        </button>
        <Menu position="bottom-start">
          <Menu.Target>
            <button type="button" className={classes.stepDay} aria-label={`${formatDay(leftTime)}, ${t('guide.jumpDay')}`}>
              <IconCalendar size={16} stroke={dl.iconStroke} aria-hidden className={classes.dimIcon} />
              {dayLabel(leftTime)}, {formatDay(leftTime)}
            </button>
          </Menu.Target>
          <Menu.Dropdown>
            {days.map((d) => (
              <Menu.Item key={d} onClick={() => onJump(d)}>
                {dayLabel(d)} · {formatDay(d)}
              </Menu.Item>
            ))}
          </Menu.Dropdown>
        </Menu>
        <button type="button" className={classes.stepBtn} aria-label={t('guide.nextDay')} disabled={startOfDay(leftTime) >= last} onClick={() => step(1)}>
          <IconChevronRight size={16} stroke={dl.iconStroke} aria-hidden />
        </button>
      </div>
      <button type="button" className={classes.toolBtn} onClick={onNow}>
        <span className={classes.liveDot} aria-hidden />
        {t('guide.now')}
      </button>
      <span className={classes.toolSpacer} />
      <Select
        size="xs"
        w={180}
        aria-label={t('guide.group')}
        allowDeselect={false}
        leftSection={<IconFilter size={14} stroke={dl.iconStroke} />}
        classNames={{ input: classes.filterInput }}
        value={groupId == null ? 'all' : String(groupId)}
        data={[{ value: 'all', label: t('guide.allGroups') }, ...groups.map((g) => ({ value: String(g.id), label: g.name }))]}
        onChange={(v) => onGroup(v && v !== 'all' ? Number(v) : null)}
      />
      <SegmentedControl
        size="xs"
        aria-label={t('guide.span')}
        value={String(span)}
        onChange={(v) => onSpan(Number(v) as SpanHours)}
        data={SPANS_HOURS.map((h) => ({ value: String(h), label: `${h}h` }))}
      />
    </div>
  );
});

const GuideLabel = memo(function GuideLabel({ ch, row, y, h }: { ch: Channel | undefined; row: number; y: number; h: number }) {
  return (
    <div className={classes.label} style={{ top: y, height: h }} aria-hidden={!ch} data-tune={ch ? row : undefined} title={ch ? t('guide.tune') : undefined}>
      {ch && (
        <>
          <ChannelLogo channelId={ch.id} name={ch.name} src={ch.logo} density="compact" />
          <span className={classes.labelText}>
            <span className={classes.labelNumber}>{ch.number}</span>
            <span className={classes.labelName}>{ch.name}</span>
          </span>
        </>
      )}
    </div>
  );
});

interface GuideRowProps {
  row: number;
  ch: Channel;
  list: Programme[] | undefined;
  geo: Geometry;
  t0: number;
  t1: number;
  now: number;
  /** Focused programme id when the focus is in this row, else null (keeps other rows memoised). */
  focusPid: string | null;
  loaded: boolean;
}

/** One row's blocks. Memoised so a vertical scroll only mounts the rows that scrolled into range. */
const GuideRow = memo(function GuideRow({ row, ch, list, geo, t0, t1, now, focusPid, loaded }: GuideRowProps) {
  const y = row * geo.rowH;
  const visible = useMemo(() => sliceRange(list ?? [], t0, t1), [list, t0, t1]);
  if (visible.length === 0)
    return loaded ? (
      <div className={classes.noData} data-tune={row} title={t('guide.tune')} style={{ top: y, height: geo.rowH, left: geo.colW, width: totalWidth(geo) - geo.colW }}>
        <span className={classes.noDataText}>{t('guide.noData')}</span>
      </div>
    ) : null;
  return (
    <div role="row" aria-rowindex={row + 1} className={classes.rowWrap}>
      {visible.map((p) => {
        const x = timeToX(geo, p.startUtc);
        const state = blockState(p, ch, now);
        return (
          <GuideBlock
            key={p.id}
            p={p}
            row={row}
            state={state}
            x={x}
            y={y}
            w={Math.max(timeToX(geo, p.endUtc) - x, 4)}
            h={geo.rowH}
            pct={state === 'now' ? progress(p.startUtc, p.endUtc, now) : 0}
            focused={p.id === focusPid}
            tabbable={p.id === focusPid}
            channelName={ch.name}
          />
        );
      })}
    </div>
  );
});

const GuideTicks = memo(function GuideTicks({ geo, t0, t1 }: { geo: Geometry; t0: number; t1: number }) {
  return (
    <>
      {tickMarks(geo, t0, t1).map((m) => (
        <div key={m.t} className={classes.tick} data-day={m.isDay || undefined} style={{ left: timeToX(geo, m.t), width: TICK_SECONDS * geo.pps }}>
          {m.label}
        </div>
      ))}
    </>
  );
});

export function GuidePage() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { loaded, hasSource, caps } = useCaps();
  const density = useUiStore((s) => s.density);
  const showDetail = useDetailStore((s) => s.show);
  const now = useNowEpoch(30_000);
  const groups = useGroups();
  const counts = useCounts();

  // Coming back from the player: reopen where the user left (unless it is stale; then scroll to now).
  const [restore] = useState(() => freshGuideView(useViewStateStore.getState().guide, nowEpoch()));
  const [span, setSpan] = useState<SpanHours>(() => (SPANS_HOURS.includes(restore?.span as SpanHours) ? (restore?.span as SpanHours) : 3));
  const [groupId, setGroupId] = useState<number | null>(restore?.groupId ?? null);
  const [size, setSize] = useState({ w: 1200, h: 700 });
  const [range, setRange] = useState<RenderRange | null>(null);
  const [leftTime, setLeftTime] = useState(() => nowEpoch());
  const [focus, setFocusState] = useState<Focus>(restore?.focus ?? { row: 0, t: nowEpoch() });
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const scrollPos = useRef({ left: 0, top: 0 });
  const raf = useRef(0);
  const cached = useRef<RenderRange | null>(null);
  const pendingFocus = useRef(false);

  const sources = useSources();
  const offline = useOffline();
  useInteractive(!!counts.data);
  const backDays = Math.max(1, sources.data?.[0]?.archiveDays ?? 1);
  const rowH = dl.density[density].guideRow;
  const scale = density === 'tv' ? dl.density.tv.typeScale : 1;
  const COL_W = Math.round(dl.guide.channelColumn * scale);
  const HEADER_H = Math.round(dl.guide.headerHeight * scale);
  const totalRows = groupId == null ? (counts.data?.all ?? 0) : (groups.data?.find((g) => g.id === groupId)?.count ?? 0);

  // Geometry is derived from numbers only; nothing here reads layout.
  const geo: Geometry = useMemo(() => {
    const dayStart = startOfDay(nowEpoch());
    const rangeStart = addDays(dayStart, -backDays);
    const rangeEnd = addDays(dayStart, FWD_DAYS);
    return { rangeStart, rangeEnd, pps: ppsFor(size.w, COL_W, span), colW: COL_W, headerH: HEADER_H, rowH };
  }, [size.w, span, rowH, backDays, COL_W, HEADER_H]);

  const viewport = useCallback((): Viewport => ({ scrollLeft: scrollPos.current.left, scrollTop: scrollPos.current.top, viewW: size.w, viewH: size.h }), [size]);

  // Recompute the render range only when the visible area leaves the cached one.
  const update = useCallback(() => {
    raf.current = 0;
    const vis = visibleRange(geo, viewport(), totalRows);
    setLeftTime((prev) => (startOfDay(prev) === startOfDay(vis.t0) ? prev : vis.t0));
    if (!isCovered(cached.current, vis)) {
      cached.current = withMargin(vis, totalRows);
      setRange(cached.current);
    }
  }, [geo, totalRows, viewport]);

  const schedule = useCallback(() => {
    if (!raf.current) raf.current = requestAnimationFrame(update);
  }, [update]);

  const onScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      // scrollLeft/scrollTop from the event target are the only reads; no layout is forced.
      scrollPos.current = { left: e.currentTarget.scrollLeft, top: e.currentTarget.scrollTop };
      schedule();
    },
    [schedule],
  );

  // Size via ResizeObserver (cached in state).
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setSize((s) => (Math.abs(s.w - r.width) < 1 && Math.abs(s.h - r.height) < 1 ? s : { w: r.width, h: r.height }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [loaded, hasSource]);

  const scrollToTime = useCallback(
    (t: number) => {
      const el = scrollRef.current;
      if (!el) return;
      el.scrollTo({ left: Math.max(0, timeToX(geo, t) - COL_W), top: el.scrollTop });
    },
    [geo, COL_W],
  );

  // Geometry change (first layout, span, resize): keep the same time at the left edge.
  const lastGeo = useRef<Geometry | null>(null);
  const updateRef = useRef(update);
  useEffect(() => {
    updateRef.current = update;
  });
  // The scroller only mounts once the channel count is known (it renders a skeleton before that).
  const hasScroller = totalRows > 0;
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const first = !lastGeo.current;
    const t = lastGeo.current ? xToTime(lastGeo.current, scrollPos.current.left + COL_W) : (restore?.leftTime ?? guideNowAnchor());
    el.scrollLeft = Math.max(0, timeToX(geo, t) - COL_W);
    if (first && restore) el.scrollTop = Math.max(0, restore.topRows * geo.rowH);
    scrollPos.current = { left: el.scrollLeft, top: el.scrollTop };
    lastGeo.current = geo;
    cached.current = null;
    updateRef.current();
  }, [geo, loaded, hasSource, COL_W, hasScroller, restore]);
  // Remember the position for the next visit; written on leaving the page.
  const snapshot = useRef<() => GuideView | null>(() => null);
  useEffect(() => {
    snapshot.current = () => {
      const g = lastGeo.current;
      if (!g) return null;
      return {
        leftTime: xToTime(g, scrollPos.current.left + COL_W),
        topRows: scrollPos.current.top / g.rowH,
        span,
        groupId,
        focus,
        savedAt: nowEpoch(),
      };
    };
  });
  useEffect(
    () => () => {
      const v = snapshot.current();
      if (v) useViewStateStore.getState().saveGuide(v);
    },
    [],
  );
  // Row count or density change: refresh the render range without touching the scroll position.
  useEffect(() => {
    cached.current = null;
    schedule();
  }, [totalRows, rowH, schedule]);

  // ---- data: chunks of (channels x time) ----
  const chunks = useMemo(() => (range ? neededChunks(range, geo.rangeStart) : { c: [], t: [] }), [range, geo.rangeStart]);
  const channelPages = useQueries({
    queries: chunks.c.map((c) => ({
      queryKey: ['catalog', 'channels', 'guide', groupId, c],
      queryFn: () => invoke('list_channels', { groupId, offset: c * CHANNEL_CHUNK, limit: CHANNEL_CHUNK }),
      staleTime: 60_000,
    })),
  });
  const queries = useQueries({
    queries: chunks.c.flatMap((c, page) => {
      const channelIds = channelPages[page]?.data?.map((ch) => ch.id) ?? [];
      return chunks.t.map((ti) => {
        const start = geo.rangeStart + ti * TIME_CHUNK;
        return {
          queryKey: ['catalog', 'guideProgrammes', c, channelIds, start],
          queryFn: () => invoke('get_programme_window', { start, end: start + TIME_CHUNK, channelIds }),
          enabled: channelIds.length > 0,
          staleTime: 60_000,
        };
      });
    }),
  });
  const dataKey = [...channelPages, ...queries].map((q) => q.dataUpdatedAt).join(',');
  const { byChannel, rowChannel, loadedChunks } = useMemo(() => {
    const list: Programme[][] = [];
    const rc = new Map<number, Channel>();
    const done = new Set<string>();
    let i = 0;
    for (const [page, c] of chunks.c.entries()) {
      const channels = channelPages[page]?.data;
      channels?.forEach((ch, k) => rc.set(c * CHANNEL_CHUNK + k, ch));
      for (const ti of chunks.t) {
        const programmes = queries[i++]?.data;
        if (programmes) list.push(programmes);
        if (programmes || channels?.length === 0) done.add(`${c}:${ti}`);
      }
    }
    return { byChannel: mergeProgrammes(list), rowChannel: rc, loadedChunks: done };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataKey, chunks, groupId, geo.rangeStart]);

  const channelFor = useCallback((row: number) => rowChannel.get(row), [rowChannel]);
  const listFor = useCallback((row: number) => {
    const ch = rowChannel.get(row);
    return ch ? byChannel.get(ch.id) : undefined;
  }, [rowChannel, byChannel]);

  // ---- focus ----
  const focusProgramme = useMemo(() => {
    const list = listFor(focus.row);
    const i = list ? indexAt(list, focus.t) : -1;
    return i >= 0 && list ? (list[i] ?? null) : null;
  }, [listFor, focus]);
  const focusChannel = channelFor(focus.row);
  const focusState = focusProgramme ? blockState(focusProgramme, focusChannel, now) : null;

  const focusDom = useCallback((pid: string | undefined) => {
    let tries = 0;
    const attempt = () => {
      const el = pid ? scrollRef.current?.querySelector<HTMLElement>(`[data-pid="${CSS.escape(pid)}"]`) : null;
      if (el) el.focus({ preventScroll: true });
      else if (tries++ < 10) requestAnimationFrame(attempt);
    };
    requestAnimationFrame(attempt);
  }, []);

  const moveTo = useCallback(
    (f: Focus, opts: { reveal?: boolean } = { reveal: true }) => {
      setFocusState(f);
      const list = listFor(f.row);
      const i = list ? indexAt(list, f.t) : -1;
      const p = i >= 0 ? list?.[i] : undefined;
      const el = scrollRef.current;
      if (el && opts.reveal !== false) {
        const reveal = scrollToReveal(geo, viewport(), { start: p?.startUtc ?? f.t, end: p?.endUtc ?? f.t + TICK_SECONDS, row: f.row });
        if (reveal.left !== undefined || reveal.top !== undefined) el.scrollTo({ left: reveal.left ?? el.scrollLeft, top: reveal.top ?? el.scrollTop });
      }
      pendingFocus.current = true;
      focusDom(p?.id);
    },
    [geo, viewport, listFor, focusDom],
  );

  const favourite = useMutation({
    mutationFn: (v: { channelId: number; favourite: boolean }) => invoke('set_favourite', v),
    onSuccess: () => invalidateFavourites(qc),
  });
  const record = useMutation({
    mutationFn: (v: { programmeId: string; kind: 'once' | 'series' }) => invoke('schedule_recording', v),
  });

  const tuneRow = useCallback(
    (row: number) => {
      const ch = rowChannel.get(row);
      if (ch) void playChannel(ch.id).then((err) => (err ? undefined : navigate('/player')));
    },
    [rowChannel, navigate],
  );

  const activate = useCallback(
    (p: Programme | null, ch = rowChannel.get(focus.row)) => {
      // No programme under the cursor (channel without guide data, or a gap): tune the channel live.
      if (!p) return tuneRow(focus.row);
      const action = enterAction(blockState(p, ch, nowEpoch()));
      if (action === 'play-live') void playChannel(p.channelId).then((err) => (err ? undefined : navigate('/player')));
      else if (action === 'play-catchup') void playCatchup(p.id).then((err) => (err ? undefined : navigate('/player')));
      else if (action === 'details') openInfo({ kind: 'programme', programme: p, channel: ch });
    },
    [rowChannel, focus.row, navigate, tuneRow],
  );

  const pageRows = Math.max(1, Math.floor((size.h - HEADER_H) / rowH) - 1);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (NAV_KEYS.includes(e.key)) {
      e.preventDefault();
      e.stopPropagation();
      const next = moveFocus(focus, e.key as FocusKey, totalRows, pageRows, listFor, xToLeft());
      if (e.key === 'ArrowLeft' && next === focus) setFocus('RAIL');
      else moveTo(next);
    } else if (e.key === 'Home') {
      e.preventDefault();
      scrollToTime(guideNowAnchor());
      moveTo({ row: focus.row, t: nowEpoch() }, { reveal: false });
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      activate(focusProgramme);
    } else if ((e.key === 'r' || e.key === 'R') && caps.dvr && focusProgramme && (focusState === 'now' || focusState === 'future')) {
      // Recording lives in the details modal (and the right-click menu).
      e.preventDefault();
      openInfo({ kind: 'programme', programme: focusProgramme, channel: focusChannel ?? undefined });
    } else if ((e.key === 'i' || e.key === 'I') && focusProgramme) {
      e.preventDefault();
      openInfo({ kind: 'programme', programme: focusProgramme, channel: focusChannel ?? undefined });
    } else if ((e.key === 'f' || e.key === 'F') && focusChannel) {
      e.preventDefault();
      favourite.mutate({ channelId: focusChannel.id, favourite: !focusChannel.isFavourite });
    }
  };
  function xToLeft() {
    return visibleRange(geo, viewport(), totalRows).t0;
  }

  const { ref: spatialRef, focusProps } = useDLFocusable<object, HTMLDivElement>({
    focusKey: 'GUIDE_GRID',
    onFocus: () => moveTo(focus),
  });

  const mergedRef = useMergedRef(scrollRef, spatialRef);

  const onClick = (e: React.MouseEvent) => {
    const tune = (e.target as HTMLElement).closest<HTMLElement>('[data-tune]');
    if (tune) {
      const row = Number(tune.dataset.tune);
      setFocusState({ row, t: focus.t });
      tuneRow(row);
      return;
    }
    const el = (e.target as HTMLElement).closest<HTMLElement>('[data-pid]');
    if (!el) return;
    const row = Number(el.dataset.row);
    const p = listFor(row)?.find((x) => x.id === el.dataset.pid);
    if (!p) return;
    setFocusState({ row, t: p.startUtc });
    el.focus({ preventScroll: true });
    // A click opens the details (play from there); Enter on a focused block still plays straight away.
    openInfo({ kind: 'programme', programme: p, channel: channelFor(row) });
  };
  const onContextMenu = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    const el = target.closest<HTMLElement>('[data-pid]');
    if (el) {
      const row = Number(el.dataset.row);
      const ch = channelFor(row);
      const p = listFor(row)?.find((x) => x.id === el.dataset.pid);
      if (!ch) return;
      if (!p || !caps.dvr) return openChannelMenu(e, ch);
      setFocusState({ row, t: p.startUtc });
      openChannelMenu(
        e,
        ch,
        <GuideRecordItems offline={offline} onRecord={(kind) => record.mutate({ programmeId: p.id, kind })} />,
      );
      return;
    }
    const lab = target.closest<HTMLElement>('[data-tune]');
    const ch = lab ? channelFor(Number(lab.dataset.tune)) : undefined;
    if (ch) openChannelMenu(e, ch);
  };

  const onPrimary = useStable((p: Programme, ch: Channel) => activate(p, ch));
  const onDetails = useStable((p: Programme) => showDetail(p.channelId, p.id));
  const onInfo = useStable((p: Programme, ch: Channel) => openInfo({ kind: 'programme', programme: p, channel: ch }));
  const onFavourite = useStable((ch: Channel) => favourite.mutate({ channelId: ch.id, favourite: !ch.isFavourite }));
  const onGroup = useStable((id: number | null) => {
    setGroupId(id);
    setFocusState({ row: 0, t: focus.t });
    cached.current = null;
    scrollRef.current?.scrollTo({ top: 0, left: scrollRef.current.scrollLeft });
  });
  const onJump = useStable((t: number) => scrollToTime(t));
  const goNow = useStable(() => scrollToTime(guideNowAnchor()));
  const groupList = useMemo(() => groups.data ?? [], [groups.data]);
  const jumpDays = useMemo(() => Array.from({ length: backDays + FWD_DAYS }, (_, i) => addDays(geo.rangeStart, i)), [geo.rangeStart, backDays]);

  if (loaded && !hasSource) return <EmptySource />;

  // ---- render ----
  const W = totalWidth(geo);
  const H = totalHeight(geo, totalRows);
  const blocks: React.ReactNode[] = [];
  const labels: React.ReactNode[] = [];
  const fillers: React.ReactNode[] = [];
  const focusPid = focusProgramme?.id;
  if (range) {
    for (let row = range.r0; row <= range.r1; row++) {
      const y = row * rowH;
      const ch = rowChannel.get(row);
      labels.push(<GuideLabel key={`l${row}`} ch={ch} row={row} y={y} h={rowH} />);
      if (!ch) {
        fillers.push(
          <div key={`s${row}`} className={classes.skeleton} style={{ top: y, height: rowH, left: timeToX(geo, range.t0 + (range.t1 - range.t0) / 3), width: 320 }}>
            <div className={classes.skeletonBar} />
          </div>,
        );
        continue;
      }
      const t0Chunk = Math.max(0, Math.floor((range.t0 - geo.rangeStart) / TIME_CHUNK));
      blocks.push(
        <GuideRow
          key={`r${row}`}
          row={row}
          ch={ch}
          list={byChannel.get(ch.id)}
          geo={geo}
          t0={range.t0}
          t1={range.t1}
          now={now}
          focusPid={row === focus.row ? (focusPid ?? null) : null}
          loaded={loadedChunks.has(`${Math.floor(row / CHANNEL_CHUNK)}:${t0Chunk}`)}
        />,
      );
    }
  }

  const nowX = timeToX(geo, now);


  return (
    <div className={classes.page} style={{ ['--gcol' as string]: `${COL_W}px`, ['--ghead' as string]: `${HEADER_H}px`, ['--gscale' as string]: scale }}>
      <GuidePreview
        programme={focusProgramme}
        channel={focusChannel}
        state={focusState}
        onPrimary={onPrimary}
        onDetails={onDetails}
        onInfo={onInfo}
        onFavourite={onFavourite}
      />
      <GuideControls
        span={span}
        onSpan={setSpan}
        groupId={groupId}
        groups={groupList}
        onGroup={onGroup}
        leftTime={leftTime}
        days={jumpDays}
        onJump={onJump}
        onNow={goNow}
      />

      {totalRows === 0 && counts.isLoading ? (
        <SkeletonList rows={8} label={t('loading.guide')} />
      ) : counts.isError ? (
        <ErrorState onRetry={() => void counts.refetch()} />
      ) : totalRows === 0 ? (
        <ErrorState message={t('channels.empty')} onRetry={() => void counts.refetch()} />
      ) : (
        <div
          ref={mergedRef}
          className={classes.scroll}
          role="grid"
          aria-label={t('guide.grid')}
          aria-rowcount={totalRows}
          aria-busy={blocks.length === 0 || undefined}
          tabIndex={0}
          onScroll={onScroll}
          onKeyDown={onKeyDown}
          onClick={onClick}
          onContextMenu={onContextMenu}
          {...focusProps}
        >
          <div className={classes.canvas} style={{ width: W, height: H }} role="presentation">
            <div className={classes.header} style={{ width: W }} aria-hidden>
              <div className={classes.corner}>{dayLabel(leftTime, now)}</div>
              {range && <GuideTicks geo={geo} t0={range.t0} t1={range.t1} />}
              {nowX > geo.colW && nowX < W && (
                <div className={classes.nowPill} style={{ left: nowX }}>
                  {formatClock(now)}
                </div>
              )}
            </div>
            <div className={classes.body} style={{ height: totalRows * rowH }}>
              <div className={classes.col} style={{ height: totalRows * rowH }} aria-hidden>
                {labels}
              </div>
              {fillers}
              {blocks}
              <div className={classes.nowLine} style={{ left: nowX }} aria-hidden />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
