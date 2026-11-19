// Spatial (d-pad / remote) navigation via norigin. The focus ring itself is drawn by global.css from
// `data-focused="true"`, which useDLFocusable sets on any norigin-managed element (DESIGN §5).
import { init, useFocusable, type UseFocusableConfig } from '@noriginmedia/norigin-spatial-navigation';

let initialised = false;

export function initSpatialNavigation(): void {
  if (initialised) return;
  initialised = true;
  init({
    // Native DOM focus follows norigin focus so :focus-visible, screen readers and Tab all agree.
    shouldFocusDOMNode: true,
    useGetBoundingClientRect: true,
    throttle: 0,
    throttleKeypresses: false,
  });
}

export function useDLFocusable<P extends object = object>(config?: UseFocusableConfig<P>) {
  const result = useFocusable<P, HTMLElement>(config);
  const focusProps = {
    'data-focusable': true as const,
    'data-focused': result.focused ? ('true' as const) : undefined,
  };
  return { ...result, focusProps };
}
