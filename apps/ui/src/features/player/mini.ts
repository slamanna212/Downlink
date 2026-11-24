import { invoke } from '../../api/ipc';
import { useUiStore } from '../../store/ui';

/** Enter or leave the mini player (compact always-on-top window). Only the player route renders it. */
export function setMiniPlayer(enabled: boolean): Promise<void> {
  return invoke('set_mini_player', { enabled })
    .then(() => useUiStore.getState().setMini(enabled))
    .catch(() => undefined);
}
