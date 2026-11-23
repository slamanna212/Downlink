import type { Meta, StoryObj } from '@storybook/react-vite';
import { Group } from '@mantine/core';
import { DLIconButton } from '../DLIconButton/DLIconButton';
import { IconPlex } from './IconPlex';

const meta: Meta<typeof DLIconButton> = {
  title: 'Primitives/IconPlex',
  component: DLIconButton,
  args: { label: 'Open in Plex', icon: IconPlex, variant: 'overlay', size: 'lg' },
};
export default meta;
type Story = StoryObj<typeof DLIconButton>;

export const Rest: Story = {};

export const States: Story = {
  render: (args) => (
    <Group gap="md" p="md" style={{ background: 'var(--dl-surface-0)' }}>
      <DLIconButton {...args} />
      <DLIconButton {...args} data-focused="true" />
      <DLIconButton {...args} disabled />
      <DLIconButton {...args} loading />
    </Group>
  ),
};
