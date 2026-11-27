import { useSportsCatalog, useSportsFollow, useSportsMatches } from '../../api/queries';
import { bucketEpoch, hoursToSeconds } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { followsAnything } from './follow';

/**
 * Followed matches from now through `hours` ahead. The window start is bucketed to 5 minutes so the
 * query key holds still; nothing is asked for while nothing is followed.
 */
export function useFollowedMatches(hours: number) {
  const now = useNowEpoch(30_000);
  const catalog = useSportsCatalog();
  const follow = useSportsFollow();
  const following = followsAnything(follow.data, catalog.data);
  const start = bucketEpoch(now);
  const matches = useSportsMatches(start, start + hoursToSeconds(hours), following);
  return { now, catalog, follow, following, matches };
}
