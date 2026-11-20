import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { SeekBar } from './SeekBar';
import { StateRow, pseudoStates, type DemoState } from '../StateRow';

const meta: Meta<typeof SeekBar> = { title: 'Primitives/SeekBar', component: SeekBar };
export default meta;
type Story = StoryObj<typeof SeekBar>;

function Demo({ disabled }: { disabled?: boolean }) {
  const [pos, setPos] = useState(1668);
  return (
    <div style={{ background: 'var(--dl-letterbox)', padding: 32, width: 720 }}>
      <SeekBar label="Seek" position={pos} duration={3600} buffered={pos + 600} disabled={disabled} onSeek={setPos} />
    </div>
  );
}
export const Interactive: Story = { render: () => <Demo /> };
export const Disabled: Story = { render: () => <Demo disabled /> };

const noop = () => undefined;
const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });

/** The six states (DESIGN §11 rule 5). Pressed = dragging; loading = nothing buffered yet. */
export const States: Story = {
  parameters: pseudoStates,
  render: () => (
    <div style={{ background: 'var(--dl-letterbox)', padding: 32, width: 720 }}>
      <StateRow title="seek bar">
        {(s) => (
          <div style={{ width: 300 }} {...demo(s)}>
            <SeekBar label="Seek" position={s === 'loading' ? 0 : 1200} duration={3600} buffered={s === 'loading' ? 0 : 1800} disabled={s === 'disabled' || s === 'loading'} onSeek={noop} />
          </div>
        )}
      </StateRow>
    </div>
  ),
};
