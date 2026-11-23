import { describe, expect, it } from 'vitest';
import { GUIDE_STALE_HOURS, freshGuideView, useViewStateStore, type GuideView } from './viewState';

const view = (savedAt: number): GuideView => ({ leftTime: 1000, topRows: 12, span: 3, groupId: null, focus: { row: 14, t: 1200 }, savedAt });

describe('freshGuideView', () => {
  it('returns null when nothing was saved', () => {
    expect(freshGuideView(null, 5000)).toBeNull();
  });
  it('keeps a recent view', () => {
    const v = view(5000);
    expect(freshGuideView(v, 5000 + 3600)).toBe(v);
  });
  it('drops a view older than the stale window', () => {
    expect(freshGuideView(view(5000), 5000 + GUIDE_STALE_HOURS * 3600 + 1)).toBeNull();
  });
  it('drops a view saved in the future (clock change)', () => {
    expect(freshGuideView(view(5000), 4000)).toBeNull();
  });
});

describe('view state store', () => {
  it('stores guide and channel views', () => {
    useViewStateStore.getState().saveChannels({ selection: { kind: 'group', id: 3 }, query: 'x', active: 4, scrollTop: 200 });
    expect(useViewStateStore.getState().channels?.active).toBe(4);
  });
});
