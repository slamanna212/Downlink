import { useState, type ReactNode } from 'react';
import type { CommandCenter } from '../../api/types';
import { useNowEpoch } from '../../lib/useNow';
import type { MultiviewApi } from '../multiview/MultiviewPage';
import { FeedManager } from './feeds/FeedManager';
import { HeadlinesSection } from './feeds/Headlines';
import { useCenterRail } from './feeds/useCenterRail';

export interface RailExtrasProps {
  /** The saved center as loaded; `center.rail` is its UI-owned rail document. */
  center: CommandCenter;
  /** The wall (tiles, focus, audio). */
  api: MultiviewApi;
}

/**
 * Extra rail sections on a saved command center, below "On screen" and "Not on screen"
 * (COMMAND_CENTER §7): the "Headlines" section from `center.rail.feeds`, with its feed manager.
 * Watching (polling) the feeds is done by the page so the ticker keeps updating with the rail hidden.
 */
export function RailExtras({ center }: RailExtrasProps): ReactNode {
  const now = useNowEpoch(30_000);
  const [managing, setManaging] = useState(false);
  const { feedIds, setFeeds, saving } = useCenterRail(center);
  return (
    <>
      <HeadlinesSection feedIds={feedIds} now={now} onManage={() => setManaging(true)} />
      <FeedManager opened={managing} onClose={() => setManaging(false)} attached={feedIds} onChange={setFeeds} saving={saving} />
    </>
  );
}
