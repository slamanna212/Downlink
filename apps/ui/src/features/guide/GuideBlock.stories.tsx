import type { Meta, StoryObj } from '@storybook/react-vite';
import type { Programme } from '../../api/types';
import { GuideBlock } from './GuideBlock';
import type { BlockState } from './guideModel';
import classes from './Guide.module.css';

const P: Programme = {
  id: 's:1', channelId: 1, startUtc: 1_700_000_000, endUtc: 1_700_003_600, title: 'Evening Bulletin', subtitle: null,
  description: null, season: null, episode: null, isNew: true, isLive: false, isPremiere: false, isFinale: false, isScheduled: false,
};

const meta: Meta<typeof GuideBlock> = { title: 'Guide/GuideBlock', component: GuideBlock };
export default meta;
type Story = StoryObj<typeof GuideBlock>;

const STATES: { state: BlockState; label: string; extra?: Partial<Programme> }[] = [
  { state: 'past-archive', label: 'Past, archive' },
  { state: 'past-none', label: 'Past, no archive (dashed)' },
  { state: 'now', label: 'Now (fill and bar)' },
  { state: 'future', label: 'Future' },
  { state: 'future', label: 'Scheduled to record', extra: { isScheduled: true } },
];

export const States: Story = {
  render: () => (
    <div style={{ display: 'grid', gap: 12, width: 420 }}>
      {STATES.map((s) => (
        <div key={s.label} style={{ position: 'relative', height: 64 }} data-demo-state>
          <GuideBlock p={{ ...P, ...s.extra, title: s.label }} row={0} state={s.state} x={0} y={0} w={420} h={64} pct={0.4} focused={false} tabbable={false} />
        </div>
      ))}
      <div style={{ position: 'relative', height: 64 }}>
        <GuideBlock p={{ ...P, title: 'Focused (ring)' }} row={0} state="future" x={0} y={0} w={420} h={64} pct={0} focused tabbable />
      </div>
      <span className={classes.blockTime}>Hover any block to see the hover fill.</span>
    </div>
  ),
};
