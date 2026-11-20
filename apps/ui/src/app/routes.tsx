import { createBrowserRouter, createMemoryRouter, type RouteObject } from 'react-router';
import type { ComponentType } from 'react';
import { Shell } from './Shell';
import { AddSourcePage } from '../features/sources/AddSourcePage';
import { HomePage } from '../features/home/HomePage';
import { ChannelsPage } from '../features/channels/ChannelsPage';
import { NotFoundPage } from '../pages/pages';
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
  multiview: () => import('../features/multiview/MultiviewPage'),
} as const;

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

// Routes use the router's own `lazy`: the router fetches the chunk BEFORE it renders the route, so there is
// no Suspense fallback (React holds a resolved reveal back ~300 ms after committing a fallback).
const page = (load: () => Promise<{ [k: string]: unknown }>, name: string): Pick<RouteObject, 'lazy'> => ({
  lazy: async () => ({ Component: (await load())[name] as ComponentType }),
});
const withProps = (load: () => Promise<{ [k: string]: unknown }>, name: string, props: Record<string, unknown>): Pick<RouteObject, 'lazy'> => ({
  lazy: async () => {
    const C = (await load())[name] as ComponentType<Record<string, unknown>>;
    return { Component: () => <C {...props} /> };
  },
});

export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    HydrateFallback: () => null,
    children: [
      { index: true, element: <HomePage /> },
      { path: 'channels', element: <ChannelsPage /> },
      { path: 'guide', ...page(importers.guide, 'GuidePage') },
      { path: 'movies', ...withProps(importers.vodBrowse, 'VodBrowse', { kind: 'movie' }) },
      { path: 'movies/:id', ...page(importers.vodDetail, 'MovieDetail') },
      { path: 'series', ...withProps(importers.vodBrowse, 'VodBrowse', { kind: 'series' }) },
      { path: 'series/:id', ...page(importers.vodDetail, 'SeriesDetailPage') },
      { path: 'recordings', ...page(importers.recordings, 'RecordingsPage') },
      { path: 'settings', ...page(importers.settings, 'SettingsPage') },
      { path: 'settings/:section', ...page(importers.settings, 'SettingsPage') },
      { path: 'search', ...page(importers.search, 'SearchPage') },
      { path: 'player', ...page(importers.player, 'PlayerPage') },
      { path: 'multiview', ...page(importers.multiview, 'MultiviewPage') },
      { path: 'sources/add', element: <AddSourcePage /> },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

export const createAppRouter = () => createBrowserRouter(routes);
export const createTestRouter = (initial: string[]) => createMemoryRouter(routes, { initialEntries: initial });
