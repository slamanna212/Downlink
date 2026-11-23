import type { Meta, StoryObj } from '@storybook/react-vite';
import { Group } from '@mantine/core';
import { StatusBadge, type StatusBadgeKind } from './StatusBadge';

const KINDS: StatusBadgeKind[] = ['live', 'rec', 'catchup', 'new', 'hd', '4k', 'radio', 'drm', 'premiere', 'finale', 'repeat'];

const meta: Meta<typeof StatusBadge> = {
  title: 'Primitives/StatusBadge',
  component: StatusBadge,
  args: { kind: 'live' },
  argTypes: { kind: { control: 'select', options: KINDS } },
};
export default meta;
type Story = StoryObj<typeof StatusBadge>;

export const Playground: Story = {};
export const All: Story = {
  render: () => (
    <Group gap="md">
      {KINDS.map((k) => (
        <StatusBadge key={k} kind={k} />
      ))}
      <StatusBadge kind="cert" label="TV-14" />
    </Group>
  ),
};
