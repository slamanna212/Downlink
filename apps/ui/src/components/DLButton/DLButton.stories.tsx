import type { Meta, StoryObj } from '@storybook/react-vite';
import { IconDeviceTv, IconPlayerPlay, IconTrash } from '@tabler/icons-react';
import { Stack } from '@mantine/core';
import { DLButton, type DLButtonVariant } from './DLButton';
import { StateRow, demoStateProps, pseudoStates } from '../StateRow';

const meta: Meta<typeof DLButton> = {
  title: 'Primitives/DLButton',
  component: DLButton,
  args: { children: 'Watch live', variant: 'default', size: 'md' },
  argTypes: {
    variant: { control: 'select', options: ['filled', 'default', 'subtle', 'live', 'danger', 'overlay'] },
    size: { control: 'select', options: ['xs', 'sm', 'md', 'lg'] },
  },
  parameters: pseudoStates,
};
export default meta;
type Story = StoryObj<typeof DLButton>;

export const Playground: Story = {};

const VARIANTS: { variant: DLButtonVariant; label: string; icon?: typeof IconPlayerPlay }[] = [
  { variant: 'filled', label: 'Add source', icon: IconPlayerPlay },
  { variant: 'default', label: 'Test connection' },
  { variant: 'subtle', label: 'Cancel' },
  { variant: 'live', label: 'Go live' },
  { variant: 'danger', label: 'Delete recording', icon: IconTrash },
];

export const States: Story = {
  render: () => (
    <Stack gap="xl">
      {VARIANTS.map((v) => (
        <StateRow key={v.variant} title={v.variant}>
          {(s) => (
            <DLButton variant={v.variant} icon={v.icon} {...demoStateProps(s)}>
              {v.label}
            </DLButton>
          )}
        </StateRow>
      ))}
      <div style={{ background: 'linear-gradient(135deg, var(--dl-letterbox), var(--dl-surface-3))', padding: 24, borderRadius: 12 }}>
        <StateRow title="overlay (on video)">
          {(s) => (
            <DLButton variant="overlay" size="lg" icon={IconDeviceTv} {...demoStateProps(s)}>
              Info
            </DLButton>
          )}
        </StateRow>
      </div>
    </Stack>
  ),
};

export const Sizes: Story = {
  render: () => (
    <Stack gap="md" align="flex-start">
      {(['xs', 'sm', 'md', 'lg'] as const).map((size) => (
        <DLButton key={size} variant="filled" size={size} icon={IconPlayerPlay}>
          Watch live ({size})
        </DLButton>
      ))}
    </Stack>
  ),
};

export const WithKeyHint: Story = { args: { variant: 'live', children: 'Go live', keyHint: 'End' } };
