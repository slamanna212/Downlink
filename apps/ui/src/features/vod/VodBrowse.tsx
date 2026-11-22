import { useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { TextInput, UnstyledButton } from '@mantine/core';
import { IconSearch } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps } from '../../api/queries';
import type { Movie, Series, VodKind } from '../../api/types';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { Icon } from '../../components/Icon/Icon';
import { PosterTile, type Tile } from '../../components/Poster/PosterTile';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { EmptySource } from '../../pages/EmptySource';
import { useUiStore } from '../../store/ui';
import { t } from '../../lib/i18n';
import { gridColumns, progressOf } from './vod';
import classes from './Vod.module.css';

const MIN_TILE = 160;
const TV_SCALE = 1.25;
const GAP = 16; // matches --mantine-spacing-lg
const TEXT_H = 52;
/** Titles fetched per request; the next page loads as the grid nears the end. */
const PAGE = 200;

const movieTile = (m: Movie): Tile => ({ id: m.id, title: m.title, poster: m.poster, sub: m.year ? String(m.year) : null, progress: progressOf(m.resumeSecs, m.durationSecs), watched: m.watched, rating: m.rating });
const seriesTile = (s: Series): Tile => ({
  id: s.id, title: s.title, poster: s.poster, sub: [s.year, s.seasonCount === 1 ? t('vod.season1') : t('vod.seasons', { count: s.seasonCount })].filter(Boolean).join(' · '), progress: null, watched: false, rating: s.rating,
});

/** Movies / Series browse: categories sidebar + virtualised poster grid (keyboard: arrows, Enter). */
export function VodBrowse({ kind }: { kind: VodKind }) {
  const navigate = useNavigate();
  const { loaded, hasSource } = useCaps();
  const [categoryId, setCategoryId] = useState<number | null>(null);
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query.trim());
  const [active, setActive] = useState(0);
  const [width, setWidth] = useState(0);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const [innerEl, setInnerEl] = useState<HTMLDivElement | null>(null);
  const activeCat = useRef<HTMLButtonElement | null>(null);

  const cats = useQuery({
    queryKey: ['catalog', 'vodCats', kind],
    queryFn: () => invoke('list_vod_categories', { kind, withCounts: false }),
  });
  const needsCounts = cats.data?.some((c) => c.countPending) ?? false;
  const counts = useQuery({
    queryKey: ['catalog', 'vodCatCounts', kind, cats.data?.map((c) => c.id)],
    queryFn: () => invoke('list_vod_categories', { kind, withCounts: true }),
    enabled: needsCounts,
  });
  const categories = counts.data ?? cats.data ?? [];
  // Keep a category selected while its delayed count resolves, even if it turns out to be empty.
  const selectedCategory = cats.data?.find((c) => c.id === categoryId);
  const shownCategories = selectedCategory && !categories.some((c) => c.id === categoryId)
    ? [...categories, selectedCategory]
    : categories;
  const list = useInfiniteQuery({
    queryKey: ['catalog', 'vod', kind, categoryId, deferred],
    initialPageParam: 0,
    queryFn: async ({ pageParam }): Promise<Tile[]> => {
      const args = { categoryId, query: deferred || null, offset: pageParam, limit: PAGE, compact: true };
      return kind === 'movie' ? (await invoke('list_movies', args)).map(movieTile) : (await invoke('list_series', args)).map(seriesTile);
    },
    getNextPageParam: (last, pages) => (last.length < PAGE ? undefined : pages.length * PAGE),
    placeholderData: (prev) => prev,
  });
  const tiles = useMemo(() => {
    // Pages can overlap when the library changes between requests: keep the first copy of an id.
    const seen = new Set<number>();
    return (list.data?.pages ?? []).flat().filter((x) => !seen.has(x.id) && (seen.add(x.id), true));
  }, [list.data]);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = list;

  useEffect(() => {
    if (!innerEl) return;
    const ro = new ResizeObserver(([e]) => e && setWidth(Math.floor(e.contentRect.width)));
    ro.observe(innerEl);
    return () => ro.disconnect();
  }, [innerEl]);

  const inner = width;
  const tvScale = useUiStore((st) => (st.density === 'tv' ? TV_SCALE : 1));
  const cols = gridColumns(inner, MIN_TILE * tvScale, GAP);
  const tileW = cols > 0 ? (inner - GAP * (cols - 1)) / cols : MIN_TILE;
  const rowH = Math.round(tileW * 1.5 + TEXT_H * tvScale + GAP);
  const rows = Math.ceil(tiles.length / cols);

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({ count: rows, getScrollElement: () => scrollerRef.current, estimateSize: () => rowH, overscan: 3 });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);
  // Load the next page when the last rendered row (or the keyboard cursor) is within a few rows of the end.
  const lastRow = virtualizer.getVirtualItems().at(-1)?.index ?? 0;
  const activeRow = cols > 0 ? Math.floor(active / cols) : 0;
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && rows > 0 && Math.max(lastRow, activeRow) >= rows - 4) void fetchNextPage();
  }, [lastRow, activeRow, rows, hasNextPage, isFetchingNextPage, fetchNextPage]);
  useEffect(() => setActive(0), [categoryId, deferred, kind]);

  const open = (i: number) => {
    const tile = tiles[i];
    if (tile) void navigate(`/${kind === 'movie' ? 'movies' : 'series'}/${tile.id}`);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!tiles.length) return;
    const move = (to: number) => {
      e.preventDefault();
      const n = Math.max(0, Math.min(tiles.length - 1, to));
      setActive(n);
      virtualizer.scrollToIndex(Math.floor(n / cols), { align: 'auto' });
    };
    if (e.key === 'ArrowRight') move(active + 1);
    else if (e.key === 'ArrowLeft') {
      if (active % cols === 0) {
        e.preventDefault();
        activeCat.current?.focus();
      } else move(active - 1);
    } else if (e.key === 'ArrowDown') move(active + cols);
    else if (e.key === 'ArrowUp') move(active - cols);
    else if (e.key === 'PageDown') move(active + cols * 3);
    else if (e.key === 'PageUp') move(active - cols * 3);
    else if (e.key === 'Home') move(0);
    else if (e.key === 'End') move(tiles.length - 1);
    else if (e.key === 'Enter') {
      e.preventDefault();
      open(active);
    }
  };

  if (loaded && !hasSource) return <EmptySource />;
  const label = kind === 'movie' ? t('vod.grid.movies') : t('vod.grid.series');
  const total = needsCounts && !counts.data ? null : categories.reduce((n, c) => n + c.count, 0);
  // While pages remain, the heading shows the category's server count (exact unless searching).
  const known = deferred ? null : categoryId == null ? total : categories.find((c) => c.id === categoryId)?.count;
  const shown = !hasNextPage ? String(tiles.length) : known ? String(known) : `${tiles.length}+`;

  return (
    <div className={classes.page}>
      <nav className={classes.sidebar} aria-label={t('vod.categories')}>
        <div className={classes.sidebarLabel}>{t('vod.categories')}</div>
        {[{ id: null as number | null, name: t('vod.all'), count: total }, ...shownCategories].map((c) => (
          <UnstyledButton
            key={c.id ?? 'all'}
            ref={c.id === categoryId ? activeCat : undefined}
            className={classes.cat}
            data-active={c.id === categoryId || undefined}
            aria-current={c.id === categoryId ? 'true' : undefined}
            onClick={() => setCategoryId(c.id)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowRight') {
                e.preventDefault();
                scrollerRef.current?.focus();
              }
            }}
          >
            <span>{c.name}</span>
            <span className={classes.catCount}>{'countPending' in c && c.countPending ? null : c.count}</span>
          </UnstyledButton>
        ))}
      </nav>
      <section className={classes.main} aria-label={label}>
        <header className={classes.header}>
          <h1 className={classes.h1}>
            {label}
            {list.data && <span className={classes.count}>{shown}</span>}
          </h1>
          <TextInput
            className={classes.search}
            aria-label={kind === 'movie' ? t('vod.search.movies') : t('vod.search.series')}
            placeholder={kind === 'movie' ? t('vod.search.movies') : t('vod.search.series')}
            leftSection={<Icon icon={IconSearch} size={16} />}
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' && tiles.length) {
                e.preventDefault();
                e.stopPropagation();
                scrollerRef.current?.focus();
              } else if (e.key === 'Escape') setQuery('');
            }}
          />
        </header>
        {list.isLoading ? (
          <SkeletonList rows={6} label={t('loading.generic')} />
        ) : list.isError ? (
          <ErrorState message={t('vod.error')} onRetry={() => void list.refetch()} />
        ) : tiles.length === 0 ? (
          <p className={classes.msg} style={{ paddingInline: 'var(--mantine-spacing-xl)' }}>{deferred || categoryId != null ? t('vod.empty') : t('vod.emptyLibrary')}</p>
        ) : (
          <div
            ref={scrollerRef}
            className={classes.scroller}
            role="grid"
            tabIndex={0}
            aria-label={label}
            aria-rowcount={rows}
            aria-colcount={cols}
            aria-activedescendant={`vt-${active}`}
            onKeyDown={onKeyDown}
          >
            <div className={classes.pad}>
            <div ref={setInnerEl} className={classes.inner} style={{ height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((v) => (
                <div key={v.key} role="row" aria-rowindex={v.index + 1} className={classes.row} style={{ height: rowH - GAP, transform: `translateY(${v.start}px)`, gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`, gap: GAP }}>
                  {tiles.slice(v.index * cols, v.index * cols + cols).map((tile, k) => {
                    const i = v.index * cols + k;
                    return (
                      <PosterTile
                        key={tile.id}
                        id={`vt-${i}`}
                        tile={tile}
                        active={i === active}
                        onOpen={() => {
                          setActive(i);
                          open(i);
                        }}
                      />
                    );
                  })}
                </div>
              ))}
            </div>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
