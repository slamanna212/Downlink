import { create } from 'zustand';
import type { GroupSelection } from '../features/channels/GroupSidebar';
import { hoursToSeconds, type Epoch } from '../lib/time';

/** A saved guide older than this reopens at "now" instead (the schedule it showed has moved on). */
export const GUIDE_STALE_HOURS = 2;

export interface GuideView {
  /** Time at the left edge of the grid (just right of the channel column). */
  leftTime: Epoch;
  /** Vertical scroll in rows, so a density change keeps the same channels in view. */
  topRows: number;
  span: number;
  groupId: number | null;
  focus: { row: number; t: Epoch };
  savedAt: Epoch;
}

export interface ChannelsView {
  selection: GroupSelection;
  query: string;
  active: number;
  scrollTop: number;
}

/** Pure: a saved guide view is only restored while it is recent enough to still make sense. */
export function freshGuideView(saved: GuideView | null, now: Epoch): GuideView | null {
  if (!saved) return null;
  const age = now - saved.savedAt;
  return age >= 0 && age <= hoursToSeconds(GUIDE_STALE_HOURS) ? saved : null;
}

interface ViewState {
  guide: GuideView | null;
  channels: ChannelsView | null;
  /** Recordings page: the selected show key, or `upcoming` for Coming up. */
  recordings: string | null;
  saveGuide: (v: GuideView) => void;
  saveChannels: (v: ChannelsView) => void;
  saveRecordings: (selected: string) => void;
}

/** Per-page position memory for the session (in memory only): scroll, focus and filters. */
export const useViewStateStore = create<ViewState>((set) => ({
  guide: null,
  channels: null,
  recordings: null,
  saveGuide: (guide) => set({ guide }),
  saveChannels: (channels) => set({ channels }),
  saveRecordings: (recordings) => set({ recordings }),
}));
