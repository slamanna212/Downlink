import { useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { SeekBar } from './SeekBar';

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
