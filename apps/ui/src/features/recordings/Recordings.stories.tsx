import { MemoryRouter } from 'react-router';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import type { Recording } from '../../api/types';
import { StateRow, pseudoStates, type DemoState } from '../../components/StateRow';
import { EpisodeRow } from './EpisodeRow';
import { ComingUpItem, ShowItem } from './RecordingsPage';
import { groupShows } from './shows';

const NOW = 1_700_000_000;
const base: Recording = {
  id: 1, title: 'Evening Bulletin', subtitle: 'Low Tide', description: null, season: 2, episode: 5, channelId: 100, channelName: 'Harbor News 24',
  startUtc: NOW - 86400, endUtc: NOW - 82800, status: 'completed', ruleId: null, ruleLabel: null, sizeBytes: null, error: null,
  channelLogo: null, programmeId: null, positionSecs: null, durationSecs: 3600, watched: false, playable: true,
};
const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });
const noop = () => undefined;

const ROWS: [string, Recording][] = [
  ['new', base],
  ['part-watched', { ...base, positionSecs: 1500 }],
  ['watched', { ...base, watched: true }],
  ['recording now', { ...base, status: 'recording', startUtc: NOW - 1200, endUtc: NOW + 2400 }],
  ['interrupted', { ...base, status: 'failed', error: 'The stream dropped after 1 h 12 min, so this recording is incomplete.' }],
];

const [show] = groupShows(
  { upcoming: [{ ...base, id: 3, status: 'scheduled', startUtc: NOW + 3600, endUtc: NOW + 7200, playable: false }], inProgress: [{ ...base, id: 2, status: 'recording' }], completed: [base] },
  [],
);

const meta: Meta = {
  title: 'Recordings/Rows',
  parameters: pseudoStates,
  decorators: [(Story) => <MemoryRouter><Story /></MemoryRouter>],
};
export default meta;

export const Episodes: StoryObj = {
  render: () => (
    <Stack gap="xl">
      {ROWS.map(([title, r]) => (
        <StateRow key={title} title={title}>
          {(s) => (
            <div style={{ width: 760 }}>
              <EpisodeRow r={r} now={NOW} run={noop} busy={s === 'disabled' || s === 'loading'} onPlay={noop} demo={demo(s)} />
            </div>
          )}
        </StateRow>
      ))}
    </Stack>
  ),
};

export const ListItems: StoryObj = {
  render: () => (
    <Stack gap="xl">
      <StateRow title="show">
        {(s) => (
          <div style={{ width: 288 }}>
            <ShowItem show={show!} focusKey={`STORY_SHOW_${s}`} active={false} onSelect={noop} demo={demo(s)} />
          </div>
        )}
      </StateRow>
      <StateRow title="show (selected)">
        {(s) => (
          <div style={{ width: 288 }}>
            <ShowItem show={{ ...show!, recordingNow: false, newCount: 2 }} focusKey={`STORY_SEL_${s}`} active onSelect={noop} demo={demo(s)} />
          </div>
        )}
      </StateRow>
      <StateRow title="coming up">
        {(s) => (
          <div style={{ width: 288 }}>
            <ComingUpItem count={4} focusKey={`STORY_UP_${s}`} active={s === 'pressed'} onSelect={noop} demo={demo(s)} />
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
