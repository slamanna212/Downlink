import type { Meta, StoryObj } from '@storybook/react-vite';
import { App } from './App';
import { createTestRouter } from './routes';
import { createQueryClient } from '../api/queries';
import { setBackend } from '../api/ipc';
import { createMockBackend } from '../api/ipc.mock';

// Whole screens against the mock backend (synthetic data). Interactive: rows, tabs and settings work.
function Screen({ path }: { path: string }) {
  setBackend(createMockBackend());
  return <App router={createTestRouter([path])} client={createQueryClient()} />;
}

const meta: Meta<typeof Screen> = {
  title: 'Screens/App',
  component: Screen,
  decorators: [
    (Story) => (
      <div style={{ height: '100vh', margin: -24 }}>
        <Story />
      </div>
    ),
  ],
  parameters: { layout: 'fullscreen' },
};
export default meta;
type Story = StoryObj<typeof Screen>;

export const Home: Story = { args: { path: '/' } };
export const Search: Story = { args: { path: '/search' } };
export const Channels: Story = { args: { path: '/channels' } };
export const Guide: Story = { args: { path: '/guide' } };
export const Movies: Story = { args: { path: '/movies' } };
export const MovieDetailScreen: Story = { args: { path: '/movies/6' } };
export const SeriesBrowse: Story = { args: { path: '/series' } };
export const SeriesDetailScreen: Story = { args: { path: '/series/4' } };
export const Recordings: Story = { args: { path: '/recordings' } };
export const Player: Story = { args: { path: '/player' } };
export const AddSource: Story = { args: { path: '/sources/add' } };
export const SettingsPlayback: Story = { args: { path: '/settings/playback' } };
export const SettingsAbout: Story = { args: { path: '/settings/about' } };
