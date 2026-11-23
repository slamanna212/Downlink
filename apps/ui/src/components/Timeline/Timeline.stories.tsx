import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import type { Timeline as TimelineState } from '../../api/types';
import { Timeline, type TimelineProgramme } from './Timeline';
import { StateRow, pseudoStates, type DemoState } from '../StateRow';

const meta: Meta<typeof Timeline> = { title: 'Primitives/Timeline', component: Timeline };
export default meta;
type Story = StoryObj<typeof Timeline>;

const NOW = 1_763_640_000;
const progs: TimelineProgramme[] = [
  { startUtc: NOW - 5400, endUtc: NOW - 1800, title: 'Evening news' },
  { startUtc: NOW - 1800, endUtc: NOW - 60, title: 'Quiz hour' },
  { startUtc: NOW - 60, endUtc: NOW + 3600, title: 'Late film' },
];
const tl = (over: Partial<TimelineState> = {}): TimelineState => ({
  playheadUtc: NOW,
  liveEdgeUtc: NOW,
  bufferStartUtc: NOW - 2400,
  archiveStartUtc: null,
  behindLiveSecs: 0,
  source: 'live',
  ...over,
});
const frame = { background: 'var(--dl-letterbox)', padding: 48, width: 720 };
const noop = () => undefined;

function Demo({ programmes = progs, archive = false }: { programmes?: TimelineProgramme[]; archive?: boolean }) {
  const [playhead, setPlayhead] = useState(NOW - 600);
  const state = tl({
    playheadUtc: playhead,
    behindLiveSecs: NOW - playhead,
    source: archive && playhead < NOW - 2400 ? 'archive' : NOW - playhead > 5 ? 'buffer' : 'live',
    archiveStartUtc: archive ? NOW - 7 * 86400 : null,
  });
  return (
    <div style={frame}>
      <Timeline label="Channel timeline" timeline={state} programmes={programmes} onSeekTo={(u) => setPlayhead(Math.min(NOW, Math.max(archive ? u : NOW - 2400, u)))} />
    </div>
  );
}

export const BehindLive: Story = { render: () => <Demo /> };
export const WithArchive: Story = { render: () => <Demo archive /> };
export const NoGuide: Story = { render: () => <Demo programmes={[]} /> };
/** A show that started a minute ago: there is still room to rewind into the previous one. */
export const JustStarted: Story = {
  render: () => (
    <div style={frame}>
      <Timeline label="Channel timeline" timeline={tl()} programmes={progs} onSeekTo={noop} />
    </div>
  ),
};

const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });

/** The six states (DESIGN §11 rule 5). Pressed = dragging; loading = nothing buffered yet. */
export const States: Story = {
  parameters: pseudoStates,
  render: () => (
    <div style={frame}>
      <StateRow title="timeline">
        {(s) => (
          <div style={{ width: 300 }} {...demo(s)}>
            <Timeline
              label="Channel timeline"
              timeline={s === 'loading' ? tl({ bufferStartUtc: null }) : tl({ playheadUtc: NOW - 900, behindLiveSecs: 900, source: 'buffer' })}
              programmes={progs}
              disabled={s === 'disabled' || s === 'loading'}
              onSeekTo={noop}
            />
          </div>
        )}
      </StateRow>
    </div>
  ),
};
