import { useEffect } from 'react';
import { invoke } from '../api/ipc';
import { useUiStore } from '../store/ui';
import { createBoundsReporter, nextBoundsSeq, toPhysicalBounds } from './videoBounds';

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
    const reporter: ReturnType<typeof createBoundsReporter> = createBoundsReporter({
      measure: () => toPhysicalBounds(el.getBoundingClientRect(), window.devicePixelRatio || 1),
      send: (b) => {
        const args = { ...b, seq: nextBoundsSeq() };
        void (slot > 0 ? invoke('multiview_set_bounds', { ...args, slot }) : invoke('set_video_bounds', args)).catch(() => {
          // Not applied natively: forget it so the next measure resends instead of deduping against it.
          reporter.reset();
          reporter.schedule();
        });
      },
    });
    const ro = new ResizeObserver(() => reporter.schedule());
    ro.observe(el);
    // Position can change without a resize (rail collapse transition, window move/zoom).
    const onChange = () => reporter.schedule();
    // A window transition (fullscreen) resizes the window over several frames and the DPR can change
    // with it: re-measure as it settles. Native sizes are only ever corrected by a later report.
    const onWindowChange = () => reporter.settle();
    window.addEventListener('resize', onWindowChange);
    const unsubFullscreen = useUiStore.subscribe((s, prev) => {
      if (s.fullscreen !== prev.fullscreen) reporter.settle();
    });
    document.addEventListener('transitionend', onChange, true);
    reporter.schedule();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', onWindowChange);
      unsubFullscreen();
      document.removeEventListener('transitionend', onChange, true);
      reporter.dispose();
    };
  }, [ref, enabled, slot, placement]);
}
