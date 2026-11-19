import type { Meta, StoryObj } from '@storybook/react-vite';
import { IconHistory, IconPlayerPlay, IconSettings, IconStar } from '@tabler/icons-react';
import { Group, Stack, Text } from '@mantine/core';
import { Icon } from './Icon';

const meta: Meta<typeof Icon> = {
  title: 'Primitives/Icon',
  component: Icon,
  args: { icon: IconSettings, size: 20 },
  argTypes: { size: { control: 'select', options: [14, 16, 20, 24] } },
};
export default meta;
type Story = StoryObj<typeof Icon>;

export const Playground: Story = {};

export const Sizes: Story = {
  render: () => (
    <Stack gap="md">
      <Group gap="xl">
        {([14, 16, 20, 24] as const).map((s) => (
          <Stack key={s} gap={4} align="center">
            <Icon icon={IconHistory} size={s} />
            <Text size="xs" c="dimmed">{s}</Text>
          </Stack>
        ))}
      </Group>
      <Group gap="xl">
        <Icon icon={IconPlayerPlay} size={24} filled />
        <Icon icon={IconStar} size={24} filled />
        <Text size="xs" c="dimmed">filled: play, pause, star-on only. Switch density to tv in the toolbar for 26 px.</Text>
      </Group>
    </Stack>
  ),
};
