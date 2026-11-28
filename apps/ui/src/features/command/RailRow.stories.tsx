import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { IconPin, IconPlus, IconReplace, IconVolume, IconX } from '@tabler/icons-react';
import { MOCK_SPORTS_CATALOG, mockSportsFollow, sportsMatchesFor } from '../../api/ipc.mock.sports';
import { mockChannels } from '../../api/ipc.mock';
import type { SportsMatch } from '../../api/types';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { RailRow } from './RailRow';
import classes from './Command.module.css';

// Synthetic matches from the mock backend (rule 16).
const NOW = Math.floor(Date.now() / 1000);
const BASE = Math.floor(NOW / 300) * 300;
const { matches } = sportsMatchesFor(mockSportsFollow(true), mockChannels(), NOW - 3600, NOW + 48 * 3600, NOW, BASE);
const pick = (pred: (m: SportsMatch) => boolean): SportsMatch => matches.find(pred) ?? matches[0]!;
const live = pick((m) => m.live && !m.liveInferred && !!m.programme && m.programme.startUtc <= NOW);
const likely = pick((m) => m.liveInferred);
const upcoming = pick((m) => !!m.programme && m.programme.startUtc > NOW);

const onScreenActions = (
  <>
    <DLIconButton label="Listen" icon={IconVolume} size="sm" />
    <DLIconButton label="Keep on screen" icon={IconPin} size="sm" />
    <DLIconButton label="Remove tile" icon={IconX} variant="danger" size="sm" />
  </>
);
const overflowActions = (
  <>
    <DLIconButton label="Swap into the focused tile" icon={IconReplace} size="sm" />
    <DLIconButton label="Add a tile" icon={IconPlus} size="sm" />
  </>
);

const meta: Meta<typeof RailRow> = {
  title: 'Features/CommandCenter/RailRow',
  component: RailRow,
  args: { match: live, channel: live.channel, catalog: MOCK_SPORTS_CATALOG, now: NOW, label: 'Focus the tile', onSelect: () => {} },
  parameters: pseudoStates,
  decorators: [(Story) => <div className={classes.railBody} style={{ width: 320 }}><Story /></div>],
};
export default meta;
type Story = StoryObj<typeof RailRow>;

export const Playground: Story = {};

/** The three rail sections: on screen (focused tile marked), also live, up next. */
export const Sections: Story = {
  render: (args) => (
    <>
      <RailRow {...args} active pinned actions={onScreenActions} />
      <RailRow {...args} match={likely} channel={likely.channel} actions={overflowActions} />
      <RailRow {...args} match={upcoming} channel={upcoming.channel} />
      <RailRow {...args} match={null} channel={live.alternates[0] ?? live.channel} actions={onScreenActions} />
    </>
  ),
};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5). The row has no loading state of its own. */
export const States: Story = {
  render: (args) => (
    <Stack gap="xl">
      <StateRow title="on screen">
        {(s) => (
          <div style={{ width: 320 }}>
            <RailRow {...args} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} actions={onScreenActions} />
          </div>
        )}
      </StateRow>
      <StateRow title="up next">
        {(s) => (
          <div style={{ width: 320 }}>
            <RailRow {...args} match={upcoming} channel={upcoming.channel} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
