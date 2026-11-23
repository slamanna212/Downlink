import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Timeline as TimelineState } from '../../api/types';
import { Timeline } from './Timeline';

const NOW = 1_763_640_000;
const tl: TimelineState = {
  playheadUtc: NOW - 600,
  liveEdgeUtc: NOW,
  bufferStartUtc: NOW - 2400,
  archiveStartUtc: null,
  behindLiveSecs: 600,
  source: 'buffer',
};

function setup(onSeekTo = vi.fn()) {
  const r = render(<Timeline label="Timeline" timeline={tl} programmes={[{ startUtc: NOW - 1800, endUtc: NOW + 1800, title: 'Show' }, { startUtc: NOW - 5400, endUtc: NOW - 1800, title: 'Earlier' }]} onSeekTo={onSeekTo} />);
  const slider = screen.getByRole('slider');
  slider.getBoundingClientRect = () => ({ left: 0, width: 1000, top: 0, height: 24, right: 1000, bottom: 24, x: 0, y: 0, toJSON: () => ({}) });
  slider.setPointerCapture = () => undefined;
  return { ...r, slider, onSeekTo };
}

describe('Timeline', () => {
  it('splits the bar at programme boundaries', () => {
    const { container } = setup();
    // Earlier | Show (the window is NOW-1800..NOW+600, so only one boundary is inside it).
    expect(container.querySelectorAll('[data-first], [data-last]').length).toBe(2);
  });

  it('a click seeks to the clicked moment', () => {
    const { slider, onSeekTo } = setup();
    fireEvent.pointerDown(slider, { clientX: 250, pointerId: 1 });
    fireEvent.pointerUp(slider, { clientX: 250, pointerId: 1 });
    // 25% of a 1 h window centred on the playhead = playhead - 15 min.
    expect(onSeekTo).toHaveBeenCalledWith(NOW - 600 - 900);
  });

  it('dragging the track right moves back in time', () => {
    const { slider, onSeekTo } = setup();
    fireEvent.pointerDown(slider, { clientX: 500, pointerId: 1 });
    fireEvent.pointerMove(slider, { clientX: 600, pointerId: 1 });
    fireEvent.pointerUp(slider, { clientX: 600, pointerId: 1 });
    expect(onSeekTo).toHaveBeenCalledWith(NOW - 600 - 360);
  });
});
