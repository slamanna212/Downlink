import { describe, expect, it } from 'vitest';
import type { Track } from '../../api/types';
import { trackLabel } from './PlayerMenus';

const base: Track = { id: 2, kind: 'audio', lang: null, title: null, codec: null, channels: null, default: false, selected: false };

describe('trackLabel', () => {
  it('combines language name, title, codec and channels', () => {
    expect(trackLabel({ ...base, lang: 'eng', title: 'Main', codec: 'ac3', channels: '5.1' })).toBe('English · Main · AC-3 5.1');
    expect(trackLabel({ ...base, lang: 'deu', codec: 'mp2', channels: 'stereo' })).toBe('German · MP2 stereo');
  });
  it('keeps unknown codecs and languages readable', () => {
    expect(trackLabel({ ...base, lang: 'zzz-bad!', codec: 'dvb_teletext' })).toBe('ZZZ-BAD! · DVB TELETEXT');
  });
  it('does not show channels on subtitle tracks', () => {
    expect(trackLabel({ ...base, kind: 'sub', lang: 'fra', channels: 'stereo' })).toBe('French');
  });
  it('falls back to the track number', () => {
    expect(trackLabel(base)).toBe('Track 2');
  });
});
