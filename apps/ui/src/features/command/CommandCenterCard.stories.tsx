import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { IconDots, IconTrophy } from '@tabler/icons-react';
import { mockChannels } from '../../api/ipc.mock';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { CommandCenterCard, CommandCenterCardSkeleton } from './CommandCenterCard';
import classes from './Command.module.css';

// Synthetic channels from the mock backend (rule 16).
const channels = mockChannels(12).filter((_, i) => i % 3 === 0);
const menu = <DLIconButton label="More actions for Morning briefing" icon={IconDots} size="sm" />;

const meta: Meta<typeof CommandCenterCard> = {
  title: 'Features/CommandCenter/CommandCenterCard',
  component: CommandCenterCard,
  args: { name: 'Morning briefing', meta: '4 channels', channels, label: 'Open Morning briefing', onSelect: () => {}, corner: menu },
  parameters: pseudoStates,
  decorators: [(Story) => <div style={{ maxWidth: 1100 }}><Story /></div>],
};
export default meta;
type Story = StoryObj<typeof CommandCenterCard>;

export const Playground: Story = {};

/** The page grid: My sports first, then saved centers. */
export const Grid: Story = {
  render: (args) => (
    <div className={classes.cards}>
      <CommandCenterCard {...args} name="My sports" description="Live games for the teams and series you follow" meta="3 games live now" icon={IconTrophy} channels={[]} corner={undefined} label="Open My sports" />
      <CommandCenterCard {...args} />
      <CommandCenterCard {...args} name="Late desk" meta="2 channels" channels={channels.slice(0, 2)} />
      <CommandCenterCardSkeleton />
    </div>
  ),
};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5). Loading is the skeleton card. */
export const States: Story = {
  render: (args) => (
    <Stack gap="xl">
      <StateRow title="saved">
        {(s) => (
          <div style={{ width: 240 }}>
            {s === 'loading' ? <CommandCenterCardSkeleton /> : <CommandCenterCard {...args} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />}
          </div>
        )}
      </StateRow>
      <StateRow title="my sports">
        {(s) => (
          <div style={{ width: 240 }}>
            {s === 'loading' ? (
              <CommandCenterCardSkeleton />
            ) : (
              <CommandCenterCard {...args} name="My sports" meta="Choose sports to fill it" icon={IconTrophy} corner={undefined} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />
            )}
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
