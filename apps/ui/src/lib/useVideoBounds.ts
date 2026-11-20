import { useEffect } from 'react';
import { invoke } from '../api/ipc';
import { createBoundsReporter, toPhysicalBounds } from './videoBounds';

/**
 * Reports the element's rect in physical pixels (ResizeObserver + one rAF debounce) so the libmpv
 * surface sits under it. Used by the full player and by the guide's mini player.
 */
export function useVideoBounds(ref: React.RefObject<HTMLElement | null>, enabled = true) {
  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;
    const reporter = createBoundsReporter({
      measure: () => toPhysicalBounds(el.getBoundingClientRect(), window.devicePixelRatio || 1),
      send: (b) => void invoke('set_video_bounds', b).catch(() => undefined),
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
  }, [ref, enabled]);
}
