import { invoke } from '../../api/ipc';

/** Fetch the (already redacted) diagnostics bundle and put it on the clipboard. Returns success. */
export async function copyDiagnostics(): Promise<boolean> {
  try {
    const d = await invoke('get_diagnostics', {});
    await navigator.clipboard.writeText(JSON.stringify(d, null, 2));
    return true;
  } catch {
    return false;
  }
}
