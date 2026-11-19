import { invoke } from '../../api/ipc';
import { toAppError } from '../../api/ipc';
import type { AppError } from '../../api/types';

/** Start playing a channel. Returns an error instead of throwing so callers can render `message`. */
export async function playChannel(channelId: number): Promise<AppError | null> {
  try {
    await invoke('play_channel', { channelId });
    return null;
  } catch (e) {
    return toAppError(e);
  }
}
