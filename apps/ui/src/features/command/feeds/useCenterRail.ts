import { useCallback } from 'react';
import { useSaveCommandCenter } from '../../../api/queries';
import type { CommandCenter, CommandCenterRail } from '../../../api/types';
import { notifyError } from '../../../components/Notify/notify';
import { railFeedIds, railSave, tickerOn, withFeeds, withTicker } from './rail';

/**
 * A saved center's feeds and ticker setting, saved straight away when changed (rail-only save:
 * the stored tiles go back unchanged, hand edits on the wall are not saved here).
 */
export function useCenterRail(center: CommandCenter | undefined) {
  const save = useSaveCommandCenter();
  const rail: CommandCenterRail = center?.rail ?? {};
  const put = useCallback(
    (next: CommandCenterRail) => {
      if (center) save.mutate(railSave(center, next), { onError: notifyError });
    },
    [center, save],
  );
  return {
    feedIds: railFeedIds(rail),
    ticker: tickerOn(rail),
    setFeeds: (ids: number[]) => put(withFeeds(rail, ids)),
    setTicker: (on: boolean) => put(withTicker(rail, on)),
    saving: save.isPending,
  };
}
