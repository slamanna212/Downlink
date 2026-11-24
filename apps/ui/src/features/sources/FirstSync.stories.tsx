import { useEffect, useState } from 'react';
import type { Meta, StoryObj } from '@storybook/react-vite';
import { FirstSync } from './FirstSync';
import { reduceSync, SYNC_START, type SyncView } from './syncProgress';

const meta: Meta<typeof FirstSync> = {
  title: 'Screens/FirstSync',
  component: FirstSync,
  parameters: { layout: 'fullscreen' },
  decorators: [(Story) => <div style={{ position: 'relative', height: '100vh' }}><Story /></div>],
};
export default meta;
type Story = StoryObj<typeof FirstSync>;

/** A whole first sync in the core's event shape, looping. */
const SCRIPT: [number, string, number, number][] = [
  [0, 'connect', 0, 4],
  [1400, 'channels', 1, 4],
  ...Array.from({ length: 10 }, (_, i): [number, string, number, number] => [1800 + i * 420, 'channels', (i + 1) * 124, 1240]),
  [6200, 'guide', 2, 4],
  ...Array.from({ length: 10 }, (_, i): [number, string, number, number] => [6600 + i * 520, 'guide', (i + 1) * 3841, 0]),
  [11800, 'state', 3, 4],
  [13000, 'done', 4, 4],
];
const LOOP_MS = 15500;

function Simulated() {
  const [view, setView] = useState<SyncView>(SYNC_START);
  useEffect(() => {
    let timers: number[] = [];
    const run = () => {
      setView(SYNC_START);
      timers = SCRIPT.map(([at, phase, done, total]) => window.setTimeout(() => setView((v) => reduceSync(v, { sourceId: 1, phase, done, total })), at));
      timers.push(window.setTimeout(run, LOOP_MS));
    };
    run();
    return () => timers.forEach((id) => window.clearTimeout(id));
  }, []);
  return <FirstSync view={view} sourceName="Home server" />;
}

export const FullSync: Story = { render: () => <Simulated /> };
export const Connecting: Story = { args: { view: SYNC_START, sourceName: 'Home server' } };
export const Channels: Story = { args: { view: { ...SYNC_START, phase: 'channels', channels: 640, channelsTotal: 1240, overall: 0.23 }, sourceName: 'Home server' } };
export const Guide: Story = { args: { view: { ...SYNC_START, phase: 'guide', channels: 1240, channelsTotal: 1240, guideDone: 18204, overall: 0.68 }, sourceName: 'Home server' } };
export const Ready: Story = { args: { view: { ...SYNC_START, phase: 'done', channels: 1240, channelsTotal: 1240, overall: 1 }, sourceName: 'Home server' } };
