import type { Meta, StoryObj } from '@storybook/react-vite';
import { Group, Stack, Text } from '@mantine/core';
import { ChannelLogo } from './ChannelLogo';

// A tiny inline SVG (no network) stands in for a real logo.
const LOGO =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 72 36'><rect x='8' y='8' width='56' height='20' rx='4' fill='currentColor' opacity='.0'/><text x='36' y='24' font-family='sans-serif' font-size='16' font-weight='700' text-anchor='middle' fill='white'>DEMO</text></svg>",
  );

const NAMES = ['Harbor News 24', 'Meridian One', 'Cobalt Sports', 'Lantern Cinema', 'Orchard Kids', 'Northwind Docs', 'Summit Music', 'Ember Drama'];

const meta: Meta<typeof ChannelLogo> = {
  title: 'Primitives/ChannelLogo',
  component: ChannelLogo,
  args: { channelId: 101, name: 'Harbor News 24', src: null },
};
export default meta;
type Story = StoryObj<typeof ChannelLogo>;

export const Fallback: Story = {};
export const RealLogo: Story = { args: { src: LOGO } };
export const BrokenImage: Story = { args: { src: 'about:blank#missing.png' } };

export const Palette: Story = {
  render: () => (
    <Stack gap="md">
      <Text size="sm" c="dimmed">Deterministic: palette[hash(channelId) % 8]</Text>
      <Group gap="md">
        {NAMES.map((n, i) => (
          <ChannelLogo key={n} channelId={100 + i} name={n} />
        ))}
      </Group>
      <Group gap="md">
        {(['compact', 'comfortable', 'tv'] as const).map((d) => (
          <ChannelLogo key={d} density={d} channelId={101} name="Harbor News 24" src={d === 'tv' ? LOGO : null} />
        ))}
      </Group>
    </Stack>
  ),
};
