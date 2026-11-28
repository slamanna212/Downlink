import type { Meta, StoryObj } from '@storybook/react-vite';
import { useState } from 'react';
import { Stack } from '@mantine/core';
import { QueryClientProvider } from '@tanstack/react-query';
import { setBackend } from '../../../api/ipc';
import { createMockBackend } from '../../../api/ipc.mock';
import { createQueryClient } from '../../../api/queries';
import type { FeedItem } from '../../../api/types';
import { DLButton } from '../../../components/DLButton/DLButton';
import { StateRow, demoStateProps, pseudoStates } from '../../../components/StateRow';
import command from '../Command.module.css';
import { FeedManager } from './FeedManager';
import { HeadlineRow, HeadlineSkeleton, HeadlinesSection, HeadlineTicker } from './Headlines';

// Synthetic headlines on invented hosts (rule 16). The live stories read the mock backend's feeds 1 and 2.
const NOW = Math.floor(Date.now() / 1000);
const item = (n: number, minutes: number, link = true): FeedItem => ({
  feedId: 1,
  guid: `story-${n}`,
  title: ['Council approves the harbor bridge budget after a late session', 'Storm watch issued for the coast through Sunday', 'Rail line reopens a day early'][n % 3]!,
  link: link ? `https://news.example/story/${n}` : null,
  summary: 'Synthetic summary for the mock backend.',
  publishedUtc: NOW - minutes * 60,
});

setBackend(createMockBackend());
const client = createQueryClient();

const meta: Meta<typeof HeadlineRow> = {
  title: 'Features/CommandCenter/Headlines',
  component: HeadlineRow,
  args: { item: item(0, 4), source: 'Example Daily', now: NOW, onOpen: () => {} },
  parameters: pseudoStates,
  decorators: [
    (Story) => (
      <QueryClientProvider client={client}>
        <div className={command.railBody} style={{ width: 320 }}><Story /></div>
      </QueryClientProvider>
    ),
  ],
};
export default meta;
type Story = StoryObj<typeof HeadlineRow>;

export const Playground: Story = {};

/** Rest, hover, pressed, focus, disabled, loading (DESIGN §11 rule 5). Disabled = a headline with no link. */
export const States: Story = {
  render: (args) => (
    <Stack gap="xl">
      <StateRow title="headline">
        {(s) => (
          <div style={{ width: 320 }}>
            {s === 'loading' ? <HeadlineSkeleton /> : <HeadlineRow {...args} item={item(1, 95, s !== 'disabled')} {...(demoStateProps(s) as object)} />}
          </div>
        )}
      </StateRow>
    </Stack>
  ),
};

/** The rail section against the mock backend's feeds. */
export const Section: Story = { render: () => <HeadlinesSection feedIds={[1, 2]} now={NOW} onManage={() => {}} /> };

/** No feeds yet: the empty state with "Manage feeds". */
export const SectionEmpty: Story = { render: () => <HeadlinesSection feedIds={[]} now={NOW} onManage={() => {}} /> };

/** The ticker strip (it sits under the wall in a command center). Hover pauses the crawl. */
export const Ticker: Story = {
  render: () => (
    <div style={{ position: 'relative', width: 900, height: 36, ['--mv-side' as string]: '0px' }}>
      <HeadlineTicker feedIds={[1, 2]} height={36} />
    </div>
  ),
};

function ManagerDemo() {
  const [opened, setOpened] = useState(true);
  const [attached, setAttached] = useState([1]);
  return (
    <>
      <DLButton onClick={() => setOpened(true)}>Manage feeds</DLButton>
      <FeedManager opened={opened} onClose={() => setOpened(false)} attached={attached} onChange={setAttached} />
    </>
  );
}

/** The feed manager: this center's feeds, the library, and add by address. */
export const Manager: Story = { render: () => <ManagerDemo /> };
