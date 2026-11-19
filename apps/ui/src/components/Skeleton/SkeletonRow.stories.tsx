import type { Meta, StoryObj } from '@storybook/react-vite';
import { SkeletonList, SkeletonRow } from './SkeletonRow';

const meta: Meta<typeof SkeletonRow> = { title: 'Primitives/SkeletonRow', component: SkeletonRow };
export default meta;
type Story = StoryObj<typeof SkeletonRow>;

export const Row: Story = {};
export const ListWithDelayedLabel: Story = {
  render: () => <SkeletonList rows={8} label="Fetching channels…" />,
};
