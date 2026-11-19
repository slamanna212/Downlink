import type { Meta, StoryObj } from '@storybook/react-vite';
import { SegmentedControl, Select, Slider, Switch } from '@mantine/core';
import { SettingRow } from './SettingRow';

const meta: Meta<typeof SettingRow> = { title: 'Patterns/SettingRow', component: SettingRow };
export default meta;
type Story = StoryObj<typeof SettingRow>;

export const Pattern: Story = {
  render: () => (
    <div style={{ maxWidth: 820 }}>
      <SettingRow label="Hardware decoding" help="Auto picks a safe decoder for your GPU.">
        <Select data={['Auto (safe)', 'Auto', 'Off (software)']} defaultValue="Auto (safe)" w="100%" aria-label="Hardware decoding" />
      </SettingRow>
      <SettingRow label="Live buffer" help="Larger buffers survive network hiccups but zap slower.">
        <Slider min={1} max={10} defaultValue={2} w="100%" aria-label="Live buffer" />
      </SettingRow>
      <SettingRow label="Density" help="Compact fits more rows." wide>
        <SegmentedControl data={['Compact', 'Comfortable', '10-foot']} defaultValue="Comfortable" />
      </SettingRow>
      <SettingRow label="Debug logging" help="Credentials are always removed." wide>
        <Switch aria-label="Debug logging" />
      </SettingRow>
    </div>
  ),
};
