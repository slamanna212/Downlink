import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { MOCK_SPORTS_CATALOG, sportsMatchesFor, mockSportsFollow } from '../../api/ipc.mock.sports';
import { mockChannels } from '../../api/ipc.mock';
import type { SportsMatch } from '../../api/types';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { SportsMatchCard, SportsMatchCardSkeleton } from './SportsMatchCard';
import classes from './Sports.module.css';

// Synthetic matches from the mock backend: live, likely live, an event channel, upcoming and a replay.
const NOW = Math.floor(Date.now() / 1000);
const BASE = Math.floor(NOW / 300) * 300;
const follow = { ...mockSportsFollow(true), includeReplays: true, leagues: { nfl: { teams: ['chi'] }, epl: { teams: [] } } };
follow.series = { ...follow.series, f1: { sessions: ['race', 'qualifying', 'practice', 'shows'] } };
const { matches } = sportsMatchesFor(follow, mockChannels(), NOW - 3600, NOW + 48 * 3600, NOW, BASE);
const pick = (pred: (m: SportsMatch) => boolean): SportsMatch => matches.find(pred) ?? matches[0]!;

const live = pick((m) => m.live && !m.liveInferred && !!m.programme && m.programme.startUtc <= NOW);
const likely = pick((m) => m.liveInferred);
const eventChannel = pick((m) => m.source === 'channel');
const qualifying = pick((m) => m.session === 'qualifying');
const replay = pick((m) => !m.live);

const meta: Meta<typeof SportsMatchCard> = {
  title: 'Features/SportsMatchCard',
  component: SportsMatchCard,
  args: { match: live, catalog: MOCK_SPORTS_CATALOG, now: NOW, onSelect: () => {}, onSelectChannel: () => {} },
  parameters: pseudoStates,
  decorators: [(Story) => <div style={{ maxWidth: 1100 }}><Story /></div>],
};
export default meta;
type Story = StoryObj<typeof SportsMatchCard>;

export const Playground: Story = {};

export const Kinds: Story = {
  render: (args) => (
    <div className={classes.grid}>
      {[live, likely, eventChannel, qualifying, replay].map((m, i) => (
        <SportsMatchCard key={i} {...args} match={m} />
      ))}
    </div>
  ),
};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5). Loading is the skeleton card. */
export const States: Story = {
  render: (args) => (
    <Stack gap="xl">
      <StateRow title="on air">
        {(s) => (
          <div style={{ width: 240 }}>
            {s === 'loading' ? <SportsMatchCardSkeleton /> : <SportsMatchCard {...args} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />}
          </div>
        )}
      </StateRow>
      <StateRow title="upcoming">
        {(s) => (
          <div style={{ width: 240 }}>
            {s === 'loading' ? <SportsMatchCardSkeleton /> : <SportsMatchCard {...args} match={qualifying} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />}
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
