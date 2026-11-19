import { useEffect, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { useHotkeys } from '@mantine/hooks';
import { dl } from '../theme';
import type { Source } from '../api/types';
import { useCaps, useSources } from '../api/queries';
import { useUiStore } from '../store/ui';
import { t } from '../lib/i18n';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { invoke } from '../api/ipc';
import { Banner } from '../components/Banner/Banner';
import { formatUpdated } from '../lib/time';
import { Rail, type RailMode } from './Rail';
import { isCollapsedRoute } from './nav';
import classes from './Shell.module.css';

/** Remounted per path (via key) so every route change fades in. */
function RouteTransition({ children }: { children: React.ReactNode }) {
  const [entered, setEntered] = useState(false);
  useEffect(() => {
    const id = requestAnimationFrame(() => requestAnimationFrame(() => setEntered(true)));
    return () => cancelAnimationFrame(id);
  }, []);
  return <div className={`${classes.route} ${entered ? classes.routeIn : ''}`}>{children}</div>;
}

/** Degraded states are quiet persistent banners, never modals (PLAN §8.7). */
function StatusBanner({ source, onRetry, onSettings }: { source: Source; onRetry: () => void; onSettings: () => void }) {
  const time = source.lastSyncAt ? formatUpdated(source.lastSyncAt) : '–';
  let banner: { tone: 'warn' | 'error'; message: string; action: string; run: () => void } | null = null;
  if (source.status === 'offline') banner = { tone: 'warn', message: t('status.offlineBanner', { time }), action: t('banner.retry'), run: onRetry };
  else if (source.status === 'stale') banner = { tone: 'warn', message: t('status.staleBanner', { time }), action: t('status.sync'), run: onRetry };
  else if (source.status === 'auth_error') banner = { tone: 'error', message: t('status.authBanner', { name: source.name }), action: t('status.authAction'), run: onSettings };
  if (!banner) return null;
  return (
    <div className={classes.banner}>
      <Banner tone={banner.tone} message={banner.message} actionLabel={banner.action} onAction={banner.run} />
    </div>
  );
}

export function Shell() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const density = useUiStore((s) => s.density);
  const railPref = useUiStore((s) => s.railPref);
  const setRailPref = useUiStore((s) => s.setRailPref);
  const fullscreen = useUiStore((s) => s.fullscreen);
  const { loaded, hasSource } = useCaps();
  const { data: sources } = useSources();
  const qc = useQueryClient();
  const retry = useMutation({
    mutationFn: (id: number) => invoke('sync_source', { sourceId: id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'sources'] }),
  });

  const autoCollapsed = isCollapsedRoute(pathname);
  const collapsed = railPref === 'auto' ? autoCollapsed : railPref === 'collapsed';
  const mode: RailMode = density === 'tv' ? 'tv' : collapsed ? 'collapsed' : 'expanded';
  const width = mode === 'tv' ? dl.layout.railTv : mode === 'collapsed' ? dl.layout.railCollapsed : dl.layout.railExpanded;
  const route = pathname.startsWith('/player') ? 'player' : 'app';

  // Keyboard map (PLAN §8.4): G guide, L live list, / or mod+K search.
  useHotkeys([
    ['G', () => void navigate('/guide')],
    ['L', () => void navigate('/channels')],
    ['/', () => pathname !== '/channels' && void navigate('/search')],
    ['mod+K', () => void navigate('/search')],
  ]);

  return (
    <div className={`dl-shell ${classes.shell}`} data-route={route} data-allow-blur={String(dl.allowBlur)}>
      <a href="#main" className={classes.skip}>
        {t('a11y.skipToContent')}
      </a>
      {!(fullscreen && route === 'player') && !(pathname === '/sources/add' && loaded && !hasSource) && (
        <aside className={classes.rail} style={{ width }}>
          <Rail mode={mode} onToggle={() => setRailPref(collapsed ? 'expanded' : 'collapsed')} />
        </aside>
      )}
      <main id="main" className={classes.main} tabIndex={-1}>
        {route !== 'player' && hasSource && sources?.[0] && (
          <StatusBanner source={sources[0]} onRetry={() => retry.mutate(sources[0]!.id)} onSettings={() => void navigate('/settings/sources')} />
        )}
        <RouteTransition key={pathname}>
          <Outlet />
        </RouteTransition>
      </main>
    </div>
  );
}
