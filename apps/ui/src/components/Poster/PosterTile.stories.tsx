import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { PosterTile, type Tile } from './PosterTile';
import { StateRow, pseudoStates, type DemoState } from '../StateRow';

const TILE: Tile = { id: 1, title: 'Silent Harbour', poster: null, sub: '1990 · 2 seasons', progress: 0.4, watched: false };
const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });

const meta: Meta<typeof PosterTile> = { title: 'Vod/PosterTile', component: PosterTile, parameters: pseudoStates };
export default meta;
type Story = StoryObj<typeof PosterTile>;

// A tile has no disabled or loading look of its own: the grid shows a skeleton while loading, and an
// unavailable title is simply not listed. Those two states render the rest look, labelled for the audit.
export const States: Story = {
  render: () => (
    <Stack gap="xl" style={{ width: 760 }}>
      <StateRow title="poster tile">
        {(s) => (
          <div style={{ width: 160 }}>
            <PosterTile tile={TILE} active={s === 'focus' || s === 'pressed'} demo={demo(s)} />
          </div>
        )}
      </StateRow>
      <StateRow title="watched, no progress">
        {(s) => (
          <div style={{ width: 160 }}>
            <PosterTile tile={{ ...TILE, progress: null, watched: true }} active={s === 'focus'} demo={demo(s)} />
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};
