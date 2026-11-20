import { createBrowserRouter, createMemoryRouter, type RouteObject } from 'react-router';
import { Shell } from './Shell';
import { SettingsPage } from '../features/settings/SettingsPage';
import { AddSourcePage } from '../features/sources/AddSourcePage';
import { GuidePage } from '../features/guide/GuidePage';
import { HomePage } from '../features/home/HomePage';
import { SearchPage } from '../features/search/SearchPage';
import { MovieDetail, SeriesDetailPage } from '../features/vod/VodDetail';
import { VodBrowse } from '../features/vod/VodBrowse';
import { RecordingsPage } from '../features/recordings/RecordingsPage';
import { PlayerPage } from '../features/player/PlayerPage';
import { ChannelsPage } from '../features/channels/ChannelsPage';
import {
  NotFoundPage,
} from '../pages/pages';

export const routes: RouteObject[] = [
  {
    path: '/',
    element: <Shell />,
    children: [
      { index: true, element: <HomePage /> },
      { path: 'channels', element: <ChannelsPage /> },
      { path: 'guide', element: <GuidePage /> },
      { path: 'movies', element: <VodBrowse kind="movie" /> },
      { path: 'movies/:id', element: <MovieDetail /> },
      { path: 'series', element: <VodBrowse kind="series" /> },
      { path: 'series/:id', element: <SeriesDetailPage /> },
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
