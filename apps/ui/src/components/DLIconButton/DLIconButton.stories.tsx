import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { IconInfoCircle, IconPlayerPause, IconStar, IconVolume } from '@tabler/icons-react';
import { Group, Stack } from '@mantine/core';
import { DLIconButton } from './DLIconButton';
import { StateRow, demoStateProps, pseudoStates } from '../StateRow';

const meta: Meta<typeof DLIconButton> = {
  title: 'Primitives/DLIconButton',
  component: DLIconButton,
  args: { label: 'Favourite', icon: IconStar, variant: 'subtle', size: 'md' },
  argTypes: {
    variant: { control: 'select', options: ['subtle', 'default', 'filled', 'live', 'danger', 'overlay'] },
    size: { control: 'select', options: ['sm', 'md', 'lg'] },
  },
  parameters: pseudoStates,
};
export default meta;
type Story = StoryObj<typeof DLIconButton>;

export const Playground: Story = {};

export const States: Story = {
  render: () => (
    <Stack gap="xl">
      <StateRow title="subtle">{(s) => <DLIconButton label="Info" icon={IconInfoCircle} {...demoStateProps(s)} />}</StateRow>
      <StateRow title="default">{(s) => <DLIconButton label="Volume" icon={IconVolume} variant="default" {...demoStateProps(s)} />}</StateRow>
      <StateRow title="toggled (aria-pressed)">
        {(s) => <DLIconButton label="Favourite" icon={IconStar} filledGlyph pressed {...demoStateProps(s)} />}
      </StateRow>
      <div style={{ background: 'linear-gradient(135deg, var(--dl-letterbox), var(--dl-surface-3))', padding: 24, borderRadius: 12 }}>
        <StateRow title="overlay on video, 44 px">
          {(s) => <DLIconButton label="Pause" icon={IconPlayerPause} variant="overlay" size="lg" filledGlyph {...demoStateProps(s)} />}
        </StateRow>
      </div>
    </Stack>
  ),
};

function ToggleDemo() {
  const [on, setOn] = useState(false);
  return (
    <Group>
      <DLIconButton label="Favourite" icon={IconStar} pressed={on} filledGlyph={on} onClick={() => setOn((v) => !v)} />
    </Group>
  );
}

export const Toggle: Story = { render: () => <ToggleDemo /> };
