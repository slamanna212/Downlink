import type { Meta, StoryObj } from '@storybook/react-vite';
import { mockDetails } from '../../api/ipc.mock';
import { DLButton } from '../../components/DLButton/DLButton';
import { InfoView, type InfoBase, type InfoViewProps } from './InfoView';
import classes from './Info.module.css';

const NOW = 1_700_000_000;
const base: InfoBase = {
  title: 'Harbour Lights',
  episodeTitle: null,
  season: 2,
  episode: 5,
  description: 'A storm strands the team at the lighthouse while an old case resurfaces.',
  isNew: false,
  isLive: false,
  isPremiere: false,
  isFinale: false,
};
const noop = () => undefined;
const common: InfoViewProps = {
  base,
  loading: false,
  error: false,
  onRetry: noop,
  onClose: noop,
  onOpenLink: noop,
  now: NOW,
  slot: { startUtc: NOW - 1200, endUtc: NOW + 2400 },
  channel: { id: 7, name: 'Ember Drama', logo: null },
  timeState: 'now',
  actions: (
    <>
      <DLButton variant="filled">Watch live</DLButton>
      <DLButton variant="default">Record</DLButton>
      <DLButton variant="subtle">Channel details</DLButton>
    </>
  ),
};

/** The modal's own frame (surface-1, 880 px, radius 12) so stories read as they ship. */
function Frame(props: InfoViewProps) {
  return (
    <div className={classes.modalContent} style={{ width: 880, borderRadius: 12 }}>
      <InfoView {...props} />
    </div>
  );
}

const meta: Meta<typeof Frame> = { title: 'Details/InfoView', component: Frame, args: common };
export default meta;
type Story = StoryObj<typeof Frame>;

/** Guide row with TMDB filling the episode title, overview, cast and crew. */
export const WithTmdb: Story = { args: { details: mockDetails({ kind: 'programme', ...base }, true) } };

/** No TMDB key: only what the server's guide says. */
export const SourceOnly: Story = { args: { details: mockDetails({ kind: 'programme', ...base, episodeTitle: 'The Lantern Keeper' }, false) } };

/** Waiting on `media_details`: the guide row's own text shows straight away. */
export const Loading: Story = { args: { loading: true } };

export const LoadFailed: Story = { args: { error: true } };

/** A future airing with no episode numbers and no artwork. */
export const PlainProgramme: Story = {
  args: {
    base: { ...base, title: 'Evening Bulletin', season: null, episode: null, isNew: true },
    timeState: 'future',
    slot: { startUtc: NOW + 3600, endUtc: NOW + 5400 },
    details: mockDetails({ kind: 'programme', title: 'Evening Bulletin', isNew: true }, false),
  },
};

export const Recording: Story = {
  args: {
    timeState: null,
    recording: true,
    details: mockDetails({ kind: 'recording', ...base, episodeTitle: 'Low Tide' }, true),
    actions: <DLButton variant="filled">Play recording</DLButton>,
  },
};
