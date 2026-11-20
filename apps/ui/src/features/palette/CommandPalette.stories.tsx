import { useEffect } from 'react';
import { MemoryRouter } from 'react-router';
import { QueryClientProvider } from '@tanstack/react-query';
import { spotlight } from '@mantine/spotlight';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { createQueryClient } from '../../api/queries';
import { setBackend } from '../../api/ipc';
import { createMockBackend } from '../../api/ipc.mock';
import { CommandPalette } from './CommandPalette';

function Open({ query }: { query?: string }) {
  setBackend(createMockBackend());
  useEffect(() => {
    spotlight.open();
    if (query) setTimeout(() => (document.querySelector('[aria-label="Command palette"]') as HTMLInputElement | null)?.focus(), 50);
  }, [query]);
  return <CommandPalette />;
}

const meta: Meta<typeof Open> = {
  title: 'Overlays/CommandPalette',
  component: Open,
  decorators: [(Story) => <QueryClientProvider client={createQueryClient()}><MemoryRouter><Story /></MemoryRouter></QueryClientProvider>],
};
export default meta;
type Story = StoryObj<typeof Open>;

// Rest = open with screens and actions; the highlighted row is the focused state. Hover, pressed, disabled
// and loading (results fetching) use the stock Mantine action states.
export const Open_: Story = { name: 'Open (rest, focus on first row)' };
