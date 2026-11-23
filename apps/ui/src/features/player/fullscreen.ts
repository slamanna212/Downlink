// Window fullscreen belongs to the full-size video surfaces (the player and multiview). Each calls
// `holdFullscreen()` on mount and `releaseFullscreen()` on unmount; like ./session, holds are counted and a
// release checks one tick later, so player <-> multiview keeps fullscreen, StrictMode's simulated remount
// does not drop it, and navigating anywhere else (guide, lists) leaves fullscreen.
import { invoke } from '../../api/ipc';
import { useUiStore } from '../../store/ui';

let holders = 0;
let pendingExit: ReturnType<typeof setTimeout> | null = null;

/** Enter or leave window fullscreen. */
export function setFullscreen(enabled: boolean): void {
  useUiStore.getState().setFullscreen(enabled);
  void invoke('set_fullscreen', { enabled }).catch(() => undefined);
}

export function toggleFullscreen(): void {
  setFullscreen(!useUiStore.getState().fullscreen);
}

/** F11 everywhere, plus Ctrl+Cmd+F (macOS convention; plain F is taken on both video routes). */
export function isFullscreenKey(e: KeyboardEvent): boolean {
  return e.key === 'F11' || (e.ctrlKey && e.metaKey && (e.key === 'f' || e.key === 'F'));
}

export function holdFullscreen(): void {
  holders += 1;
}

/** `onExit` runs alongside the fullscreen exit, only when no surface holds it any more. */
export function releaseFullscreen(onExit?: () => void): void {
  holders = Math.max(0, holders - 1);
  if (pendingExit) clearTimeout(pendingExit);
  pendingExit = setTimeout(() => {
    pendingExit = null;
    if (holders > 0) return;
    onExit?.();
    if (useUiStore.getState().fullscreen) setFullscreen(false);
  }, 0);
}
