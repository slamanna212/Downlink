import type { Meta, StoryObj } from '@storybook/react-vite';
import { QueryClientProvider } from '@tanstack/react-query';
import { IconArrowsShuffle, IconBadgeCc, IconLanguage } from '@tabler/icons-react';
import { createQueryClient } from '../../api/queries';
import { setBackend } from '../../api/ipc';
import { createMockBackend, mockTracks } from '../../api/ipc.mock';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StateRow, demoStateProps, pseudoStates } from '../../components/StateRow';
import { AudioMenu, StreamMenu, SubtitleMenu } from './PlayerMenus';

// Player control-bar menus: audio tracks, subtitles (teletext included) and the stream picker.
function Open({ which }: { which: 'audio' | 'subs' | 'stream' }) {
  setBackend(createMockBackend());
  const tracks = mockTracks();
  const noop = () => undefined;
  return (
    <QueryClientProvider client={createQueryClient()}>
      <div style={{ background: 'var(--dl-letterbox)', padding: 32, height: 360, display: 'flex', alignItems: 'flex-end', justifyContent: 'flex-end' }}>
        {which === 'audio' && <AudioMenu tracks={tracks} opened onChange={noop} />}
        {which === 'subs' && <SubtitleMenu tracks={tracks} opened onChange={noop} />}
        {which === 'stream' && <StreamMenu channelId={1} opened onChange={noop} onDenied={noop} />}
      </div>
    </QueryClientProvider>
  );
}

const meta: Meta<typeof Open> = { title: 'Player/Menus', component: Open };
export default meta;
type Story = StoryObj<typeof Open>;

export const Audio: Story = { args: { which: 'audio' } };
export const Subtitles: Story = { args: { which: 'subs' } };
export const Stream: Story = { args: { which: 'stream' } };

/** Menu triggers in the six states (DESIGN §11 rule 5). */
export const TriggerStates: Story = {
  parameters: pseudoStates,
  render: () => (
    <div style={{ background: 'var(--dl-letterbox)', padding: 32, display: 'grid', gap: 24 }}>
      {[
        ['audio', IconLanguage],
        ['subtitles', IconBadgeCc],
        ['stream', IconArrowsShuffle],
      ].map(([name, icon]) => (
        <StateRow key={name as string} title={`${name as string} menu`}>
          {(s) => <DLIconButton label={name as string} icon={icon as typeof IconLanguage} variant="overlay" size="lg" {...demoStateProps(s)} />}
        </StateRow>
      ))}
    </div>
  ),
};
