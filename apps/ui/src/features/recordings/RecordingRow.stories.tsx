import { MemoryRouter } from 'react-router';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import type { Recording } from '../../api/types';
import { RecordingRow } from './RecordingsPage';
import { StateRow, pseudoStates, type DemoState } from '../../components/StateRow';

const NOW = 1_700_000_000;
const base: Recording = {
  id: 1, title: 'Evening Bulletin', subtitle: 'Low Tide', description: null, season: 2, episode: 5, channelId: 100, channelName: 'Harbor News 24', startUtc: NOW + 3600, endUtc: NOW + 7200,
  status: 'scheduled', ruleId: 1, ruleLabel: 'Series', sizeBytes: null, error: null,
};
const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });
const noop = () => undefined;

const meta: Meta<typeof RecordingRow> = {
  title: 'Recordings/RecordingRow',
  component: RecordingRow,
  parameters: pseudoStates,
  decorators: [(Story) => <MemoryRouter><Story /></MemoryRouter>],
};
export default meta;
type Story = StoryObj<typeof RecordingRow>;

export const States: Story = {
  render: () => (
    <Stack gap="xl">
      {(['upcoming', 'inProgress', 'completed'] as const).map((tab) => (
        <StateRow key={tab} title={tab}>
          {(s) => (
            <div style={{ width: 980 }}>
              <RecordingRow
                r={tab === 'inProgress' ? { ...base, status: 'recording', startUtc: NOW - 1200 } : tab === 'completed' ? { ...base, status: 'completed', startUtc: NOW - 86400, endUtc: NOW - 82800 } : base}
                now={NOW}
                tab={tab}
                run={noop}
                busy={s === 'disabled' || s === 'loading'}
                demo={demo(s)}
              />
            </div>
          )}
        </StateRow>
      ))}
      <StateRow title="failed">
        {(s) => (
          <div style={{ width: 980 }}>
            <RecordingRow r={{ ...base, status: 'failed', error: 'The channel stopped sending video, so this recording is incomplete.' }} now={NOW} tab="completed" run={noop} busy={s === 'loading'} demo={demo(s)} />
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
