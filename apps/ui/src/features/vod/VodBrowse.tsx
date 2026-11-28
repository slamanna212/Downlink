import { useDeferredValue, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Skeleton, TextInput, UnstyledButton } from '@mantine/core';
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
import { useSparseList } from '../../lib/useSparseList';
import { t } from '../../lib/i18n';
import { gridColumns, progressOf, seasonsLabel } from './vod';
import classes from './Vod.module.css';

const MIN_TILE = 160;
const TV_SCALE = 1.25;
const GAP = 16; // matches --mantine-spacing-lg
const TEXT_H = 52;
/** Titles fetched per request; the next page loads as the grid nears the end. */
const PAGE = 200;

const movieTile = (m: Movie): Tile => ({ id: m.id, title: m.title, poster: m.poster, sub: m.year ? String(m.year) : null, progress: progressOf(m.resumeSecs, m.durationSecs), watched: m.watched, rating: m.rating });
const seriesTile = (s: Series): Tile => ({
  id: s.id, title: s.title, poster: s.poster, sub: [s.year, seasonsLabel(s.seasonCount)].filter(Boolean).join(' · '), progress: null, watched: false, rating: s.rating,
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
  // Counts come from the catalog sync's cache; `countPending` marks one the server could not give.
  const categories = cats.data ?? [];
  const shownCategories = categories;
  // Visible tile range, fed back from the virtualizer: only the pages covering it are requested.
  const [range, setRange] = useState<[number, number]>([0, 0]);
  // The grid is laid out at the total the first page reports, so the heading and scrollbar are
  // right from the first response.
  const list = useSparseList<Tile>({
    queryKey: ['catalog', 'vod', kind, categoryId, deferred],
    pageSize: PAGE,
    fetchPage: async (offset, limit) => {
      const args = { categoryId, query: deferred || null, offset, limit, compact: true };
      if (kind === 'movie') {
        const r = await invoke('list_movies', args);
        return { items: r.items.map(movieTile), total: r.total };
      }
      const r = await invoke('list_series', args);
      return { items: r.items.map(seriesTile), total: r.total };
    },
    start: range[0],
    end: range[1],
  });
  const count = list.count;

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
  const rows = cols > 0 ? Math.ceil(count / cols) : 0;

  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({ count: rows, getScrollElement: () => scrollerRef.current, estimateSize: () => rowH, overscan: 3 });
  useEffect(() => virtualizer.measure(), [rowH, virtualizer]);
  // Request the pages around the rendered rows and the keyboard cursor (a few rows ahead).
  const virtualRows = virtualizer.getVirtualItems();
  const firstRow = virtualRows[0]?.index ?? 0;
  const lastRow = virtualRows.at(-1)?.index ?? 0;
  const activeRow = cols > 0 ? Math.floor(active / cols) : 0;
  const wantStart = Math.min(firstRow, activeRow) * Math.max(cols, 1);
  const wantEnd = (Math.max(lastRow, activeRow) + 4) * Math.max(cols, 1);
  useEffect(() => {
    setRange((r) => (r[0] === wantStart && r[1] === wantEnd ? r : [wantStart, wantEnd]));
  }, [wantStart, wantEnd]);
  useEffect(() => setActive(0), [categoryId, deferred, kind]);

  const open = (i: number) => {
    const tile = list.get(i);
    if (tile) void navigate(`/${kind === 'movie' ? 'movies' : 'series'}/${tile.id}`);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!count) return;
    const move = (to: number) => {
      e.preventDefault();
      const n = Math.max(0, Math.min(count - 1, to));
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
    else if (e.key === 'End') move(count - 1);
    else if (e.key === 'Enter') {
      e.preventDefault();
      open(active);
    }
  };

  if (loaded && !hasSource) return <EmptySource />;
  const label = kind === 'movie' ? t('vod.grid.movies') : t('vod.grid.series');
  const total = categories.some((c) => c.countPending) ? null : categories.reduce((n, c) => n + c.count, 0);
  const shown = list.exact ? String(count) : `${count}+`;

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
            {!list.isLoading && <span className={classes.count}>{shown}</span>}
          </h1>
          <TextInput
            className={classes.search}
            aria-label={kind === 'movie' ? t('vod.search.movies') : t('vod.search.series')}
            placeholder={kind === 'movie' ? t('vod.search.movies') : t('vod.search.series')}
            leftSection={<Icon icon={IconSearch} size={16} />}
            value={query}
            onChange={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowDown' && count) {
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
        ) : count === 0 ? (
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
              {virtualRows.map((v) => (
                <div key={v.key} role="row" aria-rowindex={v.index + 1} className={classes.row} style={{ height: rowH - GAP, transform: `translateY(${v.start}px)`, gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`, gap: GAP }}>
                  {Array.from({ length: Math.max(0, Math.min(cols, count - v.index * cols)) }, (_, k) => {
                    const i = v.index * cols + k;
                    const tile = list.get(i);
                    if (!tile) return <Skeleton key={i} className={classes.tileSkeleton} radius="md" />;
                    return (
                      <PosterTile
                        key={i}
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
