import { createBrowserRouter, createMemoryRouter, type RouteObject } from 'react-router';
import { lazy, Suspense, type ReactNode } from 'react';
import { Shell } from './Shell';
import { AddSourcePage } from '../features/sources/AddSourcePage';
import { HomePage } from '../features/home/HomePage';
import { ChannelsPage } from '../features/channels/ChannelsPage';
import { SkeletonList } from '../components/Skeleton/SkeletonRow';
import { NotFoundPage } from '../pages/pages';
import { t } from '../lib/i18n';
import { whenIdle } from '../lib/perf';

// Shell + Home + Channels + Add source stay in the initial chunk; everything else is split per route
// (PLAN §8.8 start budget) and preloaded on idle (see `preloadRoutes`).
const importers = {
  guide: () => import('../features/guide/GuidePage'),
  vodBrowse: () => import('../features/vod/VodBrowse'),
  vodDetail: () => import('../features/vod/VodDetail'),
  recordings: () => import('../features/recordings/RecordingsPage'),
  settings: () => import('../features/settings/SettingsPage'),
  search: () => import('../features/search/SearchPage'),
  player: () => import('../features/player/PlayerPage'),
} as const;

const Guide = lazy(() => importers.guide().then((m) => ({ default: m.GuidePage })));
const VodBrowse = lazy(() => importers.vodBrowse().then((m) => ({ default: m.VodBrowse })));
const MovieDetail = lazy(() => importers.vodDetail().then((m) => ({ default: m.MovieDetail })));
const SeriesDetailPage = lazy(() => importers.vodDetail().then((m) => ({ default: m.SeriesDetailPage })));
const RecordingsPage = lazy(() => importers.recordings().then((m) => ({ default: m.RecordingsPage })));
const SettingsPage = lazy(() => importers.settings().then((m) => ({ default: m.SettingsPage })));
const SearchPage = lazy(() => importers.search().then((m) => ({ default: m.SearchPage })));
const PlayerPage = lazy(() => importers.player().then((m) => ({ default: m.PlayerPage })));

/** Warm the chunks the user is most likely to open next. Called once, after the first list is up. */
export function preloadRoutes(): void {
  whenIdle(() => {
    void importers.player();
    void importers.guide();
    whenIdle(() => {
      void importers.search();
      void importers.vodBrowse();
      void importers.recordings();
      void importers.settings();
      void importers.vodDetail();
    }, 3000);
  }, 3000);
}

const suspended = (node: ReactNode) => <Suspense fallback={<SkeletonList rows={6} label={t('loading.generic')} />}>{node}</Suspense>;
export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    children: [
      { index: true, element: <HomePage /> },
      { path: 'channels', element: <ChannelsPage /> },
      { path: 'guide', element: suspended(<Guide />) },
      { path: 'movies', element: suspended(<VodBrowse kind="movie" />) },
      { path: 'movies/:id', element: suspended(<MovieDetail />) },
      { path: 'series', element: suspended(<VodBrowse kind="series" />) },
      { path: 'series/:id', element: suspended(<SeriesDetailPage />) },
      { path: 'recordings', element: suspended(<RecordingsPage />) },
      { path: 'settings', element: suspended(<SettingsPage />) },
      { path: 'settings/:section', element: suspended(<SettingsPage />) },
      { path: 'search', element: suspended(<SearchPage />) },
      { path: 'player', element: suspended(<PlayerPage />) },
      { path: 'sources/add', element: <AddSourcePage /> },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

export const createAppRouter = () => createBrowserRouter(routes);
export const createTestRouter = (initial: string[]) => createMemoryRouter(routes, { initialEntries: initial });
