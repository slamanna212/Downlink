import { createBrowserRouter, createMemoryRouter, type RouteObject } from 'react-router';
import { Shell } from './Shell';
import {
  AddSourcePage,
  ChannelsPage,
  GuidePage,
  HomePage,
  MoviesPage,
  NotFoundPage,
  PlayerPage,
  RecordingsPage,
  SearchPage,
  SeriesPage,
  SettingsPage,
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
      { path: 'search', element: <SearchPage /> },
      { path: 'player', element: <PlayerPage /> },
      { path: 'sources/add', element: <AddSourcePage /> },
      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

export const createAppRouter = () => createBrowserRouter(routes);
export const createTestRouter = (initial: string[]) => createMemoryRouter(routes, { initialEntries: initial });
