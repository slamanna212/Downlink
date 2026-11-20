import { useEffect } from 'react';
import { invoke } from '../api/ipc';
import { createBoundsReporter, toPhysicalBounds } from './videoBounds';

/**
 * Reports the element's rect in physical pixels (ResizeObserver + one rAF debounce) so the libmpv
 * surface sits under it. Used by the full player, the guide's mini player and multiview tiles
 * (`slot` > 0 positions that tile's surface; slot 0 is the main player). `placement` re-reports when the
 * element moves without resizing (a multiview tile reflowing to another grid cell of the same size).
 */
export function useVideoBounds(ref: React.RefObject<HTMLElement | null>, enabled = true, slot = 0, placement = '') {
  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;
    const reporter = createBoundsReporter({
      measure: () => toPhysicalBounds(el.getBoundingClientRect(), window.devicePixelRatio || 1),
      send: (b) =>
        void (slot > 0 ? invoke('multiview_set_bounds', { ...b, slot }) : invoke('set_video_bounds', b)).catch(() => undefined),
    });
    const ro = new ResizeObserver(() => reporter.schedule());
    ro.observe(el);
    // Position can change without a resize (rail collapse transition, window move/zoom).
    const onChange = () => reporter.schedule();
    window.addEventListener('resize', onChange);
    document.addEventListener('transitionend', onChange, true);
    reporter.schedule();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', onChange);
      document.removeEventListener('transitionend', onChange, true);
      reporter.dispose();
    };
  }, [ref, enabled, slot, placement]);
}
