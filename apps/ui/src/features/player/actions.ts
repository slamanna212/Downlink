import { invoke } from '../../api/ipc';
import { toAppError } from '../../api/ipc';
import { notifyError } from '../../components/Notify/notify';
import type { AppError } from '../../api/types';

/** Start playing a channel. Returns an error instead of throwing so callers can render `message`. */
export async function playChannel(channelId: number): Promise<AppError | null> {
  try {
    await invoke('play_channel', { channelId });
    return null;
  } catch (e) {
    const err = toAppError(e);
    notifyError(err);
    return err;
  }
}

export async function playCatchup(programmeId: string, resume = false): Promise<AppError | null> {
  try {
    await invoke('play_catchup', { programmeId, resume });
    return null;
  } catch (e) {
    const err = toAppError(e);
    notifyError(err);
    return err;
  }
}

export async function restartProgramme(channelId: number): Promise<AppError | null> {
  try {
    await invoke('restart_programme', { channelId });
    return null;
  } catch (e) {
    const err = toAppError(e);
    notifyError(err);
    return err;
  }
}

export async function playVod(kind: 'movie' | 'episode', id: number, resume = false): Promise<AppError | null> {
  try {
    await invoke('play_vod', { kind, id, resume });
    return null;
  } catch (e) {
    return toAppError(e);
  }
}

export async function playRecording(id: number): Promise<AppError | null> {
  try {
    await invoke('play_recording', { id });
    return null;
  } catch (e) {
    const err = toAppError(e);
    notifyError(err);
    return err;
  }
}
