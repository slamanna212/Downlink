import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { useHotkeys } from '@mantine/hooks';
import { dl } from '../theme';
import type { Source } from '../api/types';
import { useCaps, useSettings, useSources } from '../api/queries';
import { useUiStore } from '../store/ui';
import { usePlayerStore } from '../store/player';
import { t } from '../lib/i18n';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { invoke, listen } from '../api/ipc';
import { playCatchup, playChannel } from '../features/player/actions';
import { Banner } from '../components/Banner/Banner';
import { formatUpdated } from '../lib/time';
import { readTvSignals, wantsTv } from '../lib/tv';
import { Rail, type RailMode } from './Rail';
import { TitleBar } from '../components/TitleBar/TitleBar';
import { isMac } from '../lib/platform';
import { preloadRoutes } from './routes';
import { useDetailStore } from '../store/detail';
import { useInfoStore } from '../store/info';
import { whenIdle } from '../lib/perf';
import { openPalette } from '../lib/palette';
import classes from './Shell.module.css';

const CommandPalette = lazy(() => import('../features/palette/CommandPalette').then((m) => ({ default: m.CommandPalette })));
const InfoModal = lazy(() => import('../features/info/InfoModal').then((m) => ({ default: m.InfoModal })));
const ChannelDetail = lazy(() => import('../features/detail/ChannelDetail').then((m) => ({ default: m.ChannelDetail })));

/** Mounts the code-split palette once the app is idle (a queued open request is replayed on mount). */
function IdleMount({ children, when }: { children: React.ReactNode; when?: boolean }) {
  const [ready, setReady] = useState(false);
  useEffect(() => whenIdle(() => setReady(true)), []);
  if (when && !ready) setReady(true); // latch: once opened, stay mounted so the exit transition can run
  return ready ? <Suspense fallback={null}>{children}</Suspense> : null;
}

/** Remounted per path (via key) so every route change fades in. */
/**
 * `instant` skips the fade: leaving a video route, the native surface jumps straight to its new rect (e.g. the
 * guide's mini player) and can't fade with the webview, so a fading page would reveal it on an empty window.
 */
function RouteTransition({ children, instant }: { children: React.ReactNode; instant: boolean }) {
  const [entered, setEntered] = useState(instant);
  useEffect(() => {
    if (instant) return;
    const id = requestAnimationFrame(() => requestAnimationFrame(() => setEntered(true)));
    return () => cancelAnimationFrame(id);
  }, [instant]);
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

/** Picks tv density by window size / `--tv` unless the user chose a density; tracks keyboard vs pointer input. */
function useAutoDensity() {
  useEffect(() => {
    const apply = () => {
      const st = useUiStore.getState();
      if (st.densityExplicit) return;
      const want = wantsTv(readTvSignals()) ? 'tv' : 'comfortable';
      if (want !== st.density) st.setAutoDensity(want);
    };
    apply();
    window.addEventListener('resize', apply);
    const root = document.documentElement;
    const keys = () => (root.dataset.input = 'keys');
    const pointer = () => (root.dataset.input = 'pointer');
    window.addEventListener('keydown', keys, true);
    window.addEventListener('mousemove', pointer, true);
    return () => {
      window.removeEventListener('resize', apply);
      window.removeEventListener('keydown', keys, true);
      window.removeEventListener('mousemove', pointer, true);
    };
  }, []);
}

/**
 * "Resume last channel on launch": the shell starts playback itself, so when the setting is on and playback
 * begins during the first seconds on Home, open the player. Never steals focus from a screen the user chose.
 */
function useResumeOnLaunch() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { data: settings } = useSettings();
  const state = usePlayerStore((s) => s.player.state);
  const done = useRef(false);
  const active = ['Resolving', 'Connecting', 'Buffering', 'Playing'].includes(state);
  useEffect(() => {
    if (done.current || !settings?.resumeLastChannel || !active || pathname !== '/') return;
    if (performance.now() > 15_000) return;
    done.current = true;
    void navigate('/player', { replace: true });
  }, [settings?.resumeLastChannel, active, pathname, navigate]);
}

/** Esc = back (PLAN §8.4) on every screen except Home and the player, unless an overlay or a text field owns it. */
function useEscapeBack() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  useEffect(() => {
    if (pathname === '/' || pathname.startsWith('/player') || pathname.startsWith('/multiview')) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)) return;
      if (document.querySelector('[role="dialog"], [role="menu"], [aria-expanded="true"]')) return;
      void navigate(-1);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [navigate, pathname]);
}

/** `app://deeplink` (second instance / OS URL handler): play a channel or a catch-up programme. */
function useDeepLinks() {
  const navigate = useNavigate();
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    void listen('app://deeplink', (link) => {
      void (async () => {
        const uuid = link.kind === 'channel' ? link.uuid : link.channel;
        const ch = await invoke('get_channel_by_uuid', { uuid });
        if (!ch) return;
        const err = link.kind === 'play' && link.programme ? await playCatchup(link.programme) : await playChannel(ch.id);
        if (!err) void navigate('/player');
      })();
    }).then((s) => (cancelled ? s() : (stop = s)));
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [navigate]);
}

const isVideoRoute = (path: string) => path.startsWith('/player') || path.startsWith('/multiview');
/**
 * Routes whose content runs under the title bar (heroes and video). Everywhere else the bar takes its own
 * 36 px row above the page, so no page control ever sits inside the drag region.
 */
const underTitleBar = (path: string) => path === '/' || path === '/guide' || isVideoRoute(path);

export function Shell() {
  useResumeOnLaunch();
  const detailOpen = useDetailStore((s) => s.open !== null);
  const infoOpen = useInfoStore((s) => s.subject !== null);
  useEffect(() => {
    const id = window.setTimeout(preloadRoutes, 1500);
    return () => window.clearTimeout(id);
  }, []);
  useAutoDensity();
  useDeepLinks();
  useEscapeBack();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const density = useUiStore((s) => s.density);
  const railPref = useUiStore((s) => s.railPref);
  const setRailPref = useUiStore((s) => s.setRailPref);
  const fullscreen = useUiStore((s) => s.fullscreen);
  const mini = useUiStore((s) => s.mini);
  const videoHole = useUiStore((s) => s.videoHole);
  const firstSync = useUiStore((s) => s.firstSync);
  const { loaded, hasSource } = useCaps();
  const { data: sources } = useSources();
  const qc = useQueryClient();
  const retry = useMutation({
    mutationFn: (id: number) => invoke('sync_source', { sourceId: id }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['catalog', 'sources'] }),
  });

  // Same rail size on every route; only the user's explicit toggle collapses it.
  const collapsed = railPref === 'collapsed';
  const mode: RailMode = density === 'tv' ? 'tv' : collapsed ? 'collapsed' : 'expanded';
  const width = mode === 'tv' ? dl.layout.railTv : mode === 'collapsed' ? dl.layout.railCollapsed : dl.layout.railExpanded;
  // Video routes leave the video rects transparent so the native surfaces show through.
  const route = isVideoRoute(pathname) ? 'player' : 'app';
  // Previous pathname, tracked in state (React's "adjust state on prop change" pattern) so the new route's
  // first render already knows whether it came from a video route.
  const [prevPath, setPrevPath] = useState(pathname);
  const [fromVideo, setFromVideo] = useState(false);
  if (prevPath !== pathname) {
    setPrevPath(pathname);
    setFromVideo(isVideoRoute(prevPath));
  }
  useEffect(() => {
    if (!isVideoRoute(pathname)) useUiStore.getState().setLastAppPath(pathname);
  }, [pathname]);

  // Keyboard map (PLAN §8.4): G guide, L live list, / search.
  useHotkeys([
    ['G', () => void navigate('/guide')],
    ['L', () => void navigate('/channels')],
    ['/', () => pathname !== '/channels' && void navigate('/search')],
  ]);
  // Ctrl/Cmd+K opens the command palette from anywhere, text fields included. The palette itself is
  // code-split; an open request that arrives before it mounts is queued (lib/palette).
  useHotkeys([['mod+K', () => openPalette(), { preventDefault: true }]], []);

  return (
    <div className={`dl-shell ${classes.shell}`} data-route={route} data-video-hole={(videoHole && route !== 'player') || undefined} data-allow-blur={String(dl.allowBlur)} data-mac={isMac() || undefined}>
      <a href="#main" className={classes.skip}>
        {t('a11y.skipToContent')}
      </a>
      {!((fullscreen || mini) && route === 'player') && !(pathname === '/sources/add' && ((loaded && !hasSource) || firstSync)) && (
        <aside className={classes.rail} style={{ width }}>
          <Rail mode={mode} onToggle={() => setRailPref(collapsed ? 'expanded' : 'collapsed')} />
        </aside>
      )}
      <div className={classes.content} data-under-bar={underTitleBar(pathname) || undefined}>
      {!fullscreen && !mini && (
        <TitleBar onVideo={route === 'player'} inFlow={!underTitleBar(pathname)} passive={pathname.startsWith('/multiview')} />
      )}
      <main id="main" className={classes.main} tabIndex={-1}>
        {route !== 'player' && !firstSync && hasSource && sources?.[0] && (
          <StatusBanner source={sources[0]} onRetry={() => retry.mutate(sources[0]!.id)} onSettings={() => void navigate('/settings/sources')} />
        )}
        <RouteTransition key={pathname} instant={fromVideo && route === 'app'}>
          <Outlet />
        </RouteTransition>
      </main>
      </div>
      <IdleMount when={detailOpen}><ChannelDetail /></IdleMount>
      <IdleMount when={infoOpen}><InfoModal /></IdleMount>
      <IdleMount><CommandPalette /></IdleMount>
    </div>
  );
}
