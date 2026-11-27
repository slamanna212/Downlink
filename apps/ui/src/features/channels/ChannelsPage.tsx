import { useCallback, useDeferredValue, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Center, SegmentedControl, Text, TextInput, VisuallyHidden } from '@mantine/core';
import { IconLayoutList, IconList, IconSearch } from '@tabler/icons-react';
import { setFocus } from '@noriginmedia/norigin-spatial-navigation';
import { invoke } from '../../api/ipc';
import type { NowNext, RecordKind } from '../../api/types';
import { invalidateFavourites, useCaps, useCounts, useGroups, useNowNextWindow, useOffline } from '../../api/queries';
import { dl } from '../../theme';
import { Icon } from '../../components/Icon/Icon';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { SkeletonList, SkeletonRow } from '../../components/Skeleton/SkeletonRow';
import { EmptySource } from '../../pages/EmptySource';
import { isEditableTarget, isListKey, isTypeaheadKey, nextIndex } from '../../lib/listNav';
import { useDLFocusable } from '../../lib/spatial';
import { useNowEpoch } from '../../lib/useNow';
import { useInteractive } from '../../lib/perf';
import { useSparseList } from '../../lib/useSparseList';
import { t } from '../../lib/i18n';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { useDetailStore } from '../../store/detail';
import { useViewStateStore } from '../../store/viewState';
import { playChannel } from '../player/actions';
import { ChannelRow } from './ChannelRow';
import { GroupSidebar, selectionKey, type GroupSelection } from './GroupSidebar';
import classes from './Channels.module.css';

const WINDOW = 50;
const CHANNEL_PAGE = 150;
const RECENT_PROGRAMMES = 9 * WINDOW;

export function ChannelsPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const qc = useQueryClient();
  const { loaded, hasSource, caps } = useCaps();
  const offline = useOffline();
  const showDetail = useDetailStore((s) => s.show);
  const density = useUiStore((s) => s.density);
  const setDensity = useUiStore((s) => s.setDensity);
  const player = usePlayerStore((s) => s.player);
  const now = useNowEpoch();

  const [restore] = useState(() => useViewStateStore.getState().channels);
  const [selection, setSelection] = useState<GroupSelection>(restore?.selection ?? { kind: 'all' });
  const [query, setQuery] = useState(restore?.query ?? '');
  const deferred = useDeferredValue(query);
  const [active, setActive] = useState(restore?.active ?? 0);
  // Scroll offset still to be re-applied once the rows exist; null when nothing is pending.
  const pendingScroll = useRef<number | null>(restore && restore.scrollTop > 0 ? restore.scrollTop : null);
  const scrollTopRef = useRef(restore?.scrollTop ?? 0);
  const [firstVisible, setFirstVisible] = useState(0);
  const [lastVisible, setLastVisible] = useState(0);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);

  const groups = useGroups();
  const counts = useCounts();
  const search = deferred.trim() || null;
  // Without a filter the list is laid out at its known size and only the pages on screen load;
  // a filtered list grows page by page (its size is not known up front).
  const knownTotal = search
    ? null
    : selection.kind === 'all'
      ? counts.data?.all
      : selection.kind === 'favourites'
        ? counts.data?.favourites
        : groups.data?.find((g) => g.id === selection.id)?.count;
  const listArgs = {
    groupId: selection.kind === 'group' ? selection.id : null,
    favouritesOnly: selection.kind === 'favourites',
    query: search,
  };
  const list = useSparseList({
    queryKey: ['catalog', 'channels', 'sparse', listArgs],
    pageSize: CHANNEL_PAGE,
    fetchPage: async (offset, limit) => ({ items: await invoke('list_channels', { ...listArgs, offset, limit }) }),
    start: firstVisible,
    end: lastVisible + 35,
    total: knownTotal,
    // The size is needed before laying out; a filter does not wait for it.
    enabled: hasSource && (search != null || knownTotal !== undefined),
    staleTime: 5 * 60_000,
  });
  const count = list.count;
  const channelAt = list.get;
  useInteractive(count > 0);
  const rowH = dl.density[density].channelRow;

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollerRef.current,
    estimateSize: () => rowH,
    overscan: 8,
    onChange: (v) => {
      setFirstVisible(v.range?.startIndex ?? 0);
      setLastVisible(v.range?.endIndex ?? 0);
      if (pendingScroll.current === null) scrollTopRef.current = v.scrollOffset ?? 0;
    },
  });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);

  // Re-apply the saved scroll before paint (no flash of the top); keeps trying while more pages load.
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    const target = pendingScroll.current;
    if (!el || target === null || count === 0) return;
    el.scrollTop = target;
    if (el.scrollTop >= target - 1 || list.exact) pendingScroll.current = null;
  }, [count, list.exact, list.isLoading]);

  // Remember selection, filter, focused row and scroll for the next visit.
  const snapshot = useRef({ selection, query, active });
  useEffect(() => {
    snapshot.current = { selection, query, active };
  });
  useEffect(
    () => () => {
      const pending = pendingScroll.current;
      useViewStateStore.getState().saveChannels({ ...snapshot.current, scrollTop: pending ?? scrollTopRef.current });
    },
    [],
  );

  // Now/next for a window around the viewport (one window above, the current one, one below).
  const windowIndex = Math.floor(firstVisible / WINDOW);
  const windowIds: number[] = [];
  for (let i = Math.max(0, windowIndex - 1) * WINDOW; i < Math.min(count, (windowIndex + 2) * WINDOW); i++) {
    const c = channelAt(i);
    if (c) windowIds.push(c.id);
  }
  const nowNext = useNowNextWindow(windowIds, `${selectionKey(selection)}|${deferred}|${windowIndex}`);
  // Keep recent windows so fast scrolling doesn't flash empty programmes while a request loads.
  const [merged, setMerged] = useState<{ src: NowNext[] | undefined; byId: Map<number, NowNext> }>({ src: undefined, byId: new Map() });
  if (nowNext.data && !nowNext.isPlaceholderData && nowNext.data !== merged.src) {
    const byId = new Map(merged.byId);
    for (const n of nowNext.data) {
      byId.delete(n.channelId);
      byId.set(n.channelId, n);
    }
    for (const id of byId.keys()) {
      if (byId.size <= RECENT_PROGRAMMES) break;
      byId.delete(id);
    }
    setMerged({ src: nowNext.data, byId });
  }
  const nowNextById = merged.byId;

  const favourite = useMutation({
    mutationFn: (v: { channelId: number; favourite: boolean }) => invoke('set_favourite', v),
    onSuccess: () => invalidateFavourites(qc),
  });
  const record = useMutation({
    mutationFn: (v: { programmeId: string; kind: RecordKind }) => invoke('schedule_recording', v),
  });

  // Scroll the row into the virtual window, then give it DOM focus once it is mounted (retries a few frames).
  const focusRow = useCallback(
    (index: number) => {
      virtualizer.scrollToIndex(index);
      setActive(index);
      let tries = 0;
      const attempt = () => {
        const el = scrollerRef.current?.querySelector<HTMLElement>(`[data-index="${index}"]`);
        if (el) el.focus({ preventScroll: true });
        else if (tries++ < 10) requestAnimationFrame(attempt);
      };
      requestAnimationFrame(attempt);
    },
    [virtualizer],
  );

  const { ref: spatialRef, focusProps: listFocusProps } = useDLFocusable<object, HTMLDivElement>({
    focusKey: 'CHANNEL_LIST',
    onFocus: () => {
      if (count > 0) focusRow(Math.min(active, count - 1));
    },
  });

  // "Save and open Live TV" lands here with the first channel focused.
  const wantsFirst = (location.state as { focusFirst?: boolean } | null)?.focusFirst === true;
  useEffect(() => {
    if (wantsFirst && count > 0) {
      focusRow(0);
      void navigate(location.pathname, { replace: true, state: null });
    }
  }, [wantsFirst, count, focusRow, navigate, location.pathname]);

  // Type-ahead: `/` or any printable key moves to the filter box.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target) || e.defaultPrevented) return;
      if (e.key === '/' || isTypeaheadKey(e)) {
        searchRef.current?.focus();
        if (e.key === '/') e.preventDefault();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const play = useCallback(
    (i: number) => {
      const c = channelAt(i);
      if (!c) return;
      void playChannel(c.id).then((err) => (err ? undefined : navigate('/player')));
    },
    [channelAt, navigate],
  );

  const onListKeyDown = (e: React.KeyboardEvent) => {
    if (isListKey(e.key)) {
      e.preventDefault();
      e.stopPropagation();
      const page = Math.max(1, Math.floor((scrollerRef.current?.clientHeight ?? 400) / rowH) - 1);
      focusRow(nextIndex(active, e.key, count, page));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      play(active);
    } else if (e.key === 'f' || e.key === 'F') {
      e.preventDefault();
      e.stopPropagation();
      const c = channelAt(active);
      if (c) favourite.mutate({ channelId: c.id, favourite: !c.isFavourite });
    } else if (e.key === 'ArrowLeft') {
      e.stopPropagation();
      setFocus(`GROUP_${selectionKey(selection)}`);
    } else if (e.key === 'Escape' && query) {
      setQuery('');
    }
  };

  if (loaded && !hasSource)
    return (
      <div className={classes.main}>
        <EmptySource />
      </div>
    );

  const total = counts.data?.all ?? 0;
  const favCount = counts.data?.favourites ?? 0;
  const title =
    selection.kind === 'all'
      ? t('channels.all')
      : selection.kind === 'favourites'
        ? t('channels.favourites')
        : (groups.data?.find((g) => g.id === selection.id)?.name ?? t('channels.all'));
  const visible = virtualizer.getVirtualItems();

  return (
    <div className={classes.page}>
      <GroupSidebar
        groups={groups.data ?? []}
        total={total}
        favourites={favCount}
        selection={selection}
        onSelect={(s) => {
          setSelection(s);
          setActive(0);
        }}
      />
      <section className={classes.main} aria-label={title}>
        <header className={classes.header}>
          <div className={classes.headerRow}>
            <h1 className={classes.title}>
              {title}
              <span className={classes.count}>{`${count}${list.exact ? '' : '+'}`}</span>
            </h1>
            <div className={classes.tools}>
              <TextInput
                ref={searchRef}
                className={classes.search}
                aria-label={t('channels.filter')}
                placeholder={t('channels.filter')}
                value={query}
                onChange={(e) => setQuery(e.currentTarget.value)}
                leftSection={<Icon icon={IconSearch} size={16} />}
                rightSection={<span className={classes.kbd}>/</span>}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    setQuery('');
                    e.currentTarget.blur();
                    if (count) focusRow(active);
                  } else if ((e.key === 'ArrowDown' || e.key === 'Enter') && count) {
                    e.preventDefault();
                    e.stopPropagation();
                    focusRow(0);
                  }
                }}
              />
              <SegmentedControl
                value={density === 'compact' ? 'compact' : 'comfortable'}
                onChange={(v) => setDensity(v === 'compact' ? 'compact' : 'comfortable')}
                data={[
                  { value: 'comfortable', label: <Center><VisuallyHidden>{t('channels.density.comfortable')}</VisuallyHidden><Icon icon={IconLayoutList} size={16} /></Center> },
                  { value: 'compact', label: <Center><VisuallyHidden>{t('channels.density.compact')}</VisuallyHidden><Icon icon={IconList} size={16} /></Center> },
                ]}
              />
            </div>
          </div>
        </header>
        <div className={classes.columns} aria-hidden style={{ ['--logo-w' as string]: `${dl.density[density].logo[0]}px` }}>
          <span>{t('channels.col.no')}</span>
          <span>{t('channels.col.channel')}</span>
          <span>{t('channels.col.now')}</span>
          <span>{t('channels.col.next')}</span>
        </div>
        {list.isLoading ? (
          <SkeletonList rows={10} label={t('loading.channels')} />
        ) : list.isError ? (
          <ErrorState onRetry={() => void list.refetch()} />
        ) : count === 0 ? (
          <Text className={classes.empty}>{t('channels.empty')}</Text>
        ) : (
          <div
            ref={(n) => {
              scrollerRef.current = n;
              (spatialRef as React.MutableRefObject<HTMLDivElement | null>).current = n;
            }}
            className={classes.scroller}
            role="list"
            aria-label={t('channels.list')}
            onKeyDown={onListKeyDown}
            {...listFocusProps}
          >
            <div className={classes.inner} style={{ height: virtualizer.getTotalSize(), ['--logo-w' as string]: `${dl.density[density].logo[0]}px` }}>
              {visible.map((v) => {
                const c = channelAt(v.index);
                if (!c)
                  return (
                    <div key={`s${v.index}`} data-index={v.index} className={classes.placeholder} style={{ height: v.size, transform: `translateY(${v.start}px)` }}>
                      <SkeletonRow />
                    </div>
                  );
                return (
                  <ChannelRow
                    key={c.id}
                    data-index={v.index}
                    channel={c}
                    nowNext={nowNextById.get(c.id)}
                    now={now}
                    active={v.index === active}
                    playing={player.channelId === c.id && player.state !== 'Idle' && player.state !== 'Stopped'}
                    style={{ top: 0, height: v.size, transform: `translateY(${v.start}px)` }}
                    onPlay={() => play(v.index)}
                    onToggleFavourite={() => favourite.mutate({ channelId: c.id, favourite: !c.isFavourite })}
                    onFocusRow={() => setActive(v.index)}
                    canRecord={caps.dvr}
                    offline={offline}
                    onRecord={(programmeId, kind) => record.mutate({ programmeId, kind })}
                    onDetails={() => showDetail(c.id)}
                  />
                );
              })}
            </div>
            {list.pageError && <ErrorState onRetry={() => list.refetch()} />}
          </div>
        )}
      </section>
    </div>
  );
}
