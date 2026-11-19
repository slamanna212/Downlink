import { createBrowserRouter, createMemoryRouter, type RouteObject } from 'react-router';
import { Shell } from './Shell';
import { SettingsPage } from '../features/settings/SettingsPage';
import { AddSourcePage } from '../features/sources/AddSourcePage';
import { PlayerPage } from '../features/player/PlayerPage';
import { ChannelsPage } from '../features/channels/ChannelsPage';
import {
  GuidePage,
  HomePage,
  MoviesPage,
  NotFoundPage,
  RecordingsPage,
  SearchPage,
  SeriesPage,
} from '../pages/pages';

export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    children: [
      { index: true, element: <HomePage /> },
      { path: 'channels', element: <ChannelsPage /> },
      { path: 'guide', element: <GuidePage /> },
      { path: 'movies', element: <MoviesPage /> },
      { path: 'series', element: <SeriesPage /> },
      { path: 'recordings', element: <RecordingsPage /> },
      { path: 'settings', element: <SettingsPage /> },
      { path: 'settings/:section', element: <SettingsPage /> },
      { path: 'search', element: <SearchPage /> },
      { path: 'player', element: <PlayerPage /> },
      { path: 'sources/add', element: <AddSourcePage /> },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

export const createAppRouter = () => createBrowserRouter(routes);
export const createTestRouter = (initial: string[]) => createMemoryRouter(routes, { initialEntries: initial });
