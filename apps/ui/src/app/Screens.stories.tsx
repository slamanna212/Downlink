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

export const Channels: Story = { args: { path: '/channels' } };
export const Guide: Story = { args: { path: '/guide' } };
export const Player: Story = { args: { path: '/player' } };
export const AddSource: Story = { args: { path: '/sources/add' } };
export const SettingsPlayback: Story = { args: { path: '/settings/playback' } };
export const SettingsAbout: Story = { args: { path: '/settings/about' } };
