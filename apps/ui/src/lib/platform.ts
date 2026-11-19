export const isMac = (): boolean => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
/** Label for the command-palette shortcut. */
export const modKeyLabel = (): string => (isMac() ? '⌘K' : 'Ctrl K');
