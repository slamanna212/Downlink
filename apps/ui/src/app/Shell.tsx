import { useEffect, useState } from 'react';
import { Outlet, useLocation, useNavigate } from 'react-router';
import { useHotkeys } from '@mantine/hooks';
import { dl } from '../theme';
import { useUiStore } from '../store/ui';
import { t } from '../lib/i18n';
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

export function Shell() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const density = useUiStore((s) => s.density);
  const railPref = useUiStore((s) => s.railPref);
  const setRailPref = useUiStore((s) => s.setRailPref);
  const fullscreen = useUiStore((s) => s.fullscreen);

  const autoCollapsed = isCollapsedRoute(pathname);
  const collapsed = railPref === 'auto' ? autoCollapsed : railPref === 'collapsed';
  const mode: RailMode = density === 'tv' ? 'tv' : collapsed ? 'collapsed' : 'expanded';
  const width = mode === 'tv' ? dl.layout.railTv : mode === 'collapsed' ? dl.layout.railCollapsed : dl.layout.railExpanded;
  const route = pathname.startsWith('/player') ? 'player' : 'app';

  // Keyboard map (PLAN §8.4): G guide, L live list, / or mod+K search.
  useHotkeys([
    ['G', () => void navigate('/guide')],
    ['L', () => void navigate('/channels')],
    ['/', () => void navigate('/search')],
    ['mod+K', () => void navigate('/search')],
  ]);

  return (
    <div className={`dl-shell ${classes.shell}`} data-route={route} data-allow-blur={String(dl.allowBlur)}>
      <a href="#main" className={classes.skip}>
        {t('a11y.skipToContent')}
      </a>
      {!(fullscreen && route === 'player') && (
        <aside className={classes.rail} style={{ width }}>
          <Rail mode={mode} onToggle={() => setRailPref(collapsed ? 'expanded' : 'collapsed')} />
        </aside>
      )}
      <main id="main" className={classes.main} tabIndex={-1}>
        <RouteTransition key={pathname}>
          <Outlet />
        </RouteTransition>
      </main>
    </div>
  );
}
