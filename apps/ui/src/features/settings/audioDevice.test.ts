import { describe, expect, it } from 'vitest';
import { audioDeviceOptions } from './SettingsPage';

describe('audioDeviceOptions', () => {
  it('leads with auto even when the list is empty', () => {
    expect(audioDeviceOptions([], null)).toEqual([{ value: 'auto', label: 'Auto (system default)' }]);
  });
  it('dedupes mpv auto and keeps real devices', () => {
    const o = audioDeviceOptions([{ id: 'auto', name: 'Autoselect device' }, { id: 'hdmi', name: 'HDMI' }], null);
    expect(o.map((x) => x.value)).toEqual(['auto', 'hdmi']);
  });
  it('keeps a missing stored device selectable', () => {
    const o = audioDeviceOptions([], 'gone');
    expect(o.some((x) => x.value === 'gone')).toBe(true);
  });
});
