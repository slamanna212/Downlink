// Window chrome actions for the custom title bar. The window is frameless on Windows and Linux
// (tauri.conf.json `decorations: false`); macOS keeps its native traffic lights over an overlay title
// bar (tauri.macos.conf.json), so these buttons are never shown there.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { inTauri } from '../api/ipc';

const win = () => (inTauri() ? getCurrentWindow() : null);

export const minimizeWindow = (): void => void win()?.minimize().catch(() => undefined);
export const toggleMaximizeWindow = (): void => void win()?.toggleMaximize().catch(() => undefined);
export const closeWindow = (): void => void win()?.close().catch(() => undefined);

/** Calls `cb` with the maximized state now and after every resize. Returns an unsubscribe. */
export function watchMaximized(cb: (maximized: boolean) => void): () => void {
  const w = win();
  if (!w) return () => undefined;
  let stop: (() => void) | undefined;
  let cancelled = false;
  const read = () => void w.isMaximized().then(cb).catch(() => undefined);
  read();
  void w
    .onResized(read)
    .then((u) => (cancelled ? u() : (stop = u)))
    .catch(() => undefined);
  return () => {
    cancelled = true;
    stop?.();
  };
}
