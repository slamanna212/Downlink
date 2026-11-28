import type { Meta, StoryObj } from '@storybook/react-vite';
import { Group, Stack } from '@mantine/core';
import { IconCopyPlus, IconDeviceFloppy } from '@tabler/icons-react';
import type { MultiviewTile } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { CenterNameModal } from './CenterNameModal';
import { SaveCenterControls } from './SaveCenterControls';

const tile = (slot: number, channelId: number): [number, MultiviewTile] => [slot, { slot, channelId, state: 'Playing', failure: null }];
const api = { tiles: new Map([tile(0, 100), tile(1, 103)]), style: 'grid' as const, audible: 0 };

const meta: Meta<typeof SaveCenterControls> = {
  title: 'Features/CommandCenter/SaveCenterControls',
  component: SaveCenterControls,
  args: { api, center: null, edited: false, onSaved: () => {} },
  parameters: pseudoStates,
};
export default meta;
type Story = StoryObj<typeof SaveCenterControls>;

/** A wall that was never saved: one button opens the name dialog. */
export const NewWall: Story = {};

/** A saved center with hand edits: Save changes plus Save as new. */
export const SavedAndEdited: Story = { args: { center: { id: 1, name: 'Morning briefing' }, edited: true } };

/** The name dialog on its own. */
export const NameDialog: Story = {
  render: () => <CenterNameModal opened title="Save as command center" initial="My command center" onClose={() => {}} onSubmit={() => {}} />,
};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5), as the toolbar renders them. */
export const States: Story = {
  render: () => (
    <Stack gap="xl">
      <StateRow title="save">
        {(s) => (
          <DLButton variant="default" size="sm" icon={IconDeviceFloppy} {...(demoStateProps(s) as object)}>
            Save changes
          </DLButton>
        )}
      </StateRow>
      <StateRow title="save as new">
        {(s) => (
          <Group>
            <DLIconButton label="Save as new" icon={IconCopyPlus} size="sm" {...(demoStateProps(s) as object)} />
          </Group>
        )}
      </StateRow>
    </Stack>
  ),
};
