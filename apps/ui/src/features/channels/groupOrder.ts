import type { Group } from '../../api/types';

export type GroupOrder = 'alphabetical' | 'provider';

/** The catalog already returns A–Z. Provider positions are scoped to each source. */
export function orderGroups(groups: Group[], order: GroupOrder): Group[] {
  return order === 'provider'
    ? [...groups].sort((a, b) => a.sourceId - b.sourceId || a.position - b.position || a.id - b.id)
    : groups;
}
