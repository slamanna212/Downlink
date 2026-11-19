import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { Banner } from './Banner';

const meta: Meta<typeof Banner> = {
  title: 'Primitives/Banner',
  component: Banner,
  args: { tone: 'warn', message: 'Offline. Last updated 14:05.', actionLabel: 'Retry' },
  argTypes: { tone: { control: 'select', options: ['info', 'ok', 'warn', 'error'] } },
};
export default meta;
type Story = StoryObj<typeof Banner>;

export const Playground: Story = {};
export const Tones: Story = {
  render: () => (
    <Stack gap="md">
      <Banner tone="info" message="Guide data refreshes in the background." actionLabel="Details" />
      <Banner tone="ok" message="Connected to Demo server." />
      <Banner tone="warn" message="Offline. Last updated 14:05." actionLabel="Retry" />
      <Banner tone="error" message="Sign-in expired. Check your API key." actionLabel="Edit source" />
    </Stack>
  ),
};
