import type { Meta, StoryObj } from '@storybook/react-vite';
import { Stack } from '@mantine/core';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { FollowChip, PickerTile } from './FollowChip';

const meta: Meta<typeof FollowChip> = {
  title: 'Features/FollowChip',
  component: FollowChip,
  args: { label: 'Bears', abbr: 'CHI', onRemove: () => {} },
  parameters: pseudoStates,
};
export default meta;
type Story = StoryObj<typeof FollowChip>;

export const Playground: Story = {};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5). The state applies to the × button. */
export const States: Story = {
  render: (args) => (
    <StateRow title="followed team">
      {(s) => <FollowChip {...args} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />}
    </StateRow>
  ),
};

/** The Add teams dialog's checkbox tile, unselected and selected. */
export const Tiles: Story = {
  render: () => (
    <Stack gap="xl">
      {[false, true].map((selected) => (
        <StateRow key={String(selected)} title={selected ? 'tile: selected' : 'tile: not selected'}>
          {(s) => (
            <div style={{ width: 220 }}>
              <PickerTile label="Bears" abbr="CHI" selected={selected} onToggle={() => {}} {...(demoStateProps(s) as object)} disabled={s === 'disabled'} />
            </div>
          )}
        </StateRow>
      ))}
    </Stack>
  ),
};
