import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Center, SegmentedControl, Text, TextInput, VisuallyHidden } from '@mantine/core';
import { IconLayoutList, IconList, IconSearch } from '@tabler/icons-react';
import { setFocus } from '@noriginmedia/norigin-spatial-navigation';
import { invoke } from '../../api/ipc';
import { useCaps, useChannels, useGroups, useNowNextWindow } from '../../api/queries';
import { dl } from '../../theme';
import { Icon } from '../../components/Icon/Icon';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { EmptySource } from '../../pages/EmptySource';
import { isEditableTarget, isListKey, isTypeaheadKey, nextIndex } from '../../lib/listNav';
import { useDLFocusable } from '../../lib/spatial';
import { useNowEpoch } from '../../lib/useNow';
import { t } from '../../lib/i18n';
import { usePlayerStore } from '../../store/player';
import { useUiStore } from '../../store/ui';
import { playChannel } from '../player/actions';
import { ChannelRow } from './ChannelRow';
import { GroupSidebar, selectionKey, type GroupSelection } from './GroupSidebar';
import classes from './Channels.module.css';

const WINDOW = 50;

export function ChannelsPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const qc = useQueryClient();
  const { loaded, hasSource } = useCaps();
  const density = useUiStore((s) => s.density);
  const setDensity = useUiStore((s) => s.setDensity);
  const player = usePlayerStore((s) => s.player);
  const now = useNowEpoch();

  const [selection, setSelection] = useState<GroupSelection>({ kind: 'all' });
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query);
  const [active, setActive] = useState(0);
  const [firstVisible, setFirstVisible] = useState(0);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);

  const groups = useGroups();
  const everything = useChannels({});
  const list = useChannels({
    groupId: selection.kind === 'group' ? selection.id : null,
    favouritesOnly: selection.kind === 'favourites',
    query: deferred.trim() || null,
  });
  const channels = useMemo(() => list.data ?? [], [list.data]);
  const rowH = dl.density[density].channelRow;

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: channels.length,
    getScrollElement: () => scrollerRef.current,
    estimateSize: () => rowH,
    overscan: 8,
    onChange: (v) => setFirstVisible(v.range?.startIndex ?? 0),
  });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);

  // Now/next for a window around the viewport.
  const windowIndex = Math.floor(firstVisible / WINDOW);
  const windowIds = useMemo(
    () => channels.slice(windowIndex * WINDOW, windowIndex * WINDOW + WINDOW * 3).map((c) => c.id),
    [channels, windowIndex],
  );
  const nowNext = useNowNextWindow(windowIds, `${selectionKey(selection)}|${deferred}|${windowIndex}`);
  const nowNextById = useMemo(() => new Map((nowNext.data ?? []).map((n) => [n.channelId, n])), [nowNext.data]);

  const favourite = useMutation({
    mutationFn: (v: { channelId: number; favourite: boolean }) => invoke('set_favourite', v),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'channels'] }),
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
      if (channels.length > 0) focusRow(Math.min(active, channels.length - 1));
    },
  });

  // "Save and open Live TV" lands here with the first channel focused.
  const wantsFirst = (location.state as { focusFirst?: boolean } | null)?.focusFirst === true;
  useEffect(() => {
    if (wantsFirst && channels.length > 0) {
      focusRow(0);
      void navigate(location.pathname, { replace: true, state: null });
    }
  }, [wantsFirst, channels.length, focusRow, navigate, location.pathname]);

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
      const c = channels[i];
      if (!c) return;
      void playChannel(c.id).then(() => navigate('/player'));
    },
    [channels, navigate],
  );

  const onListKeyDown = (e: React.KeyboardEvent) => {
    if (isListKey(e.key)) {
      e.preventDefault();
      e.stopPropagation();
      const page = Math.max(1, Math.floor((scrollerRef.current?.clientHeight ?? 400) / rowH) - 1);
      focusRow(nextIndex(active, e.key, channels.length, page));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      play(active);
    } else if (e.key === 'f' || e.key === 'F') {
      e.preventDefault();
      e.stopPropagation();
      const c = channels[active];
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

  const total = everything.data?.length ?? 0;
  const favCount = (everything.data ?? []).filter((c) => c.isFavourite).length;
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
              <span className={classes.count}>{channels.length}</span>
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
                    if (channels.length) focusRow(active);
                  } else if ((e.key === 'ArrowDown' || e.key === 'Enter') && channels.length) {
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
        ) : channels.length === 0 ? (
          <Text className={classes.empty}>{t('channels.empty')}</Text>
        ) : (
          <div
            ref={(n) => {
              scrollerRef.current = n;
              (spatialRef as React.MutableRefObject<HTMLDivElement | null>).current = n;
            }}
            className={classes.scroller}
            role="listbox"
            aria-label={t('channels.list')}
            onKeyDown={onListKeyDown}
            {...listFocusProps}
          >
            <div className={classes.inner} style={{ height: virtualizer.getTotalSize(), ['--logo-w' as string]: `${dl.density[density].logo[0]}px` }}>
              {visible.map((v) => {
                const c = channels[v.index];
                if (!c) return null;
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
                  />
                );
              })}
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
