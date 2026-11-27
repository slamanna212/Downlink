import type { Meta, StoryObj } from '@storybook/react-vite';
import { useState } from 'react';
import { Stack } from '@mantine/core';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { TeamChip } from './TeamChip';
import classes from './Sports.module.css';

const meta: Meta<typeof TeamChip> = {
  title: 'Features/TeamChip',
  component: TeamChip,
  args: { label: 'Bears', abbr: 'CHI', pressed: false, onToggle: () => {} },
  parameters: pseudoStates,
};
export default meta;
type Story = StoryObj<typeof TeamChip>;

export const Playground: Story = {};

/** Rest, hover, pressed, focus, disabled, loading, unselected and selected (DESIGN §11 rule 5). */
export const States: Story = {
  render: (args) => (
    <Stack gap="xl">
      {[false, true].map((pressed) => (
        <StateRow key={String(pressed)} title={pressed ? 'selected' : 'not selected'}>
          {(s) => <TeamChip {...args} pressed={pressed} {...(demoStateProps(s) as object)} />}
        </StateRow>
      ))}
    </Stack>
  ),
};

const TEAMS = [
  { key: 'chi', abbr: 'CHI', short: 'Bears' },
  { key: 'gb', abbr: 'GB', short: 'Packers' },
  { key: 'det', abbr: 'DET', short: 'Lions' },
  { key: 'min', abbr: 'MIN', short: 'Vikings' },
];

function PickerDemo() {
  const [picked, setPicked] = useState<string[]>([]);
  return (
    <div className={classes.chips}>
      <TeamChip label="All games" pressed={picked.length === 0} onToggle={() => setPicked([])} />
      {TEAMS.map((t) => (
        <TeamChip
          key={t.key}
          abbr={t.abbr}
          label={t.short}
          pressed={picked.includes(t.key)}
          onToggle={() => setPicked((p) => (p.includes(t.key) ? p.filter((x) => x !== t.key) : [...p, t.key]))}
        />
      ))}
    </div>
  );
}

/** The picker pattern: "All games" is on while no team is picked. */
export const Picker: Story = { render: () => <PickerDemo /> };
