import type { Meta, StoryObj } from '@storybook/react-vite';
import { Poster } from './Poster';

const meta: Meta<typeof Poster> = { title: 'Primitives/Poster', component: Poster };
export default meta;
type Story = StoryObj<typeof Poster>;

export const Fallbacks: Story = {
  render: () => (
    <div style={{ display: 'flex', gap: 16, width: 720 }}>
      {['Silent Harbour', 'Golden Compass', 'Northern Echo', 'Hidden Station'].map((t) => (
        <div key={t} style={{ width: 160 }}>
          <Poster src={null} title={t} />
        </div>
      ))}
    </div>
  ),
};
export const BrokenImage: Story = { render: () => <div style={{ width: 160 }}><Poster src="dlimg://missing" title="Broken artwork" /></div> };
export const Wide: Story = { render: () => <div style={{ width: 320 }}><Poster src={null} title="S1 E3" shape="wide" /></div> };
