import { describe, expect, it } from 'vitest';
import type { AltStream } from '../../api/types';
import { streamDetails } from './PlayerMenus';

const base: AltStream = {
  index: 0, streamId: '1', name: 'Primary', provider: null, resolution: null, fps: null, videoCodec: null,
  audioCodec: null, audioChannels: null, bitrateKbps: null, viewers: null, stale: false,
};

describe('streamDetails', () => {
  it('summarises the probe stats', () => {
    expect(
      streamDetails({ ...base, resolution: '1920x1080', fps: 29.97, videoCodec: 'h264', audioCodec: 'aac', audioChannels: 'stereo', bitrateKbps: 6468.2, viewers: 2 }),
    ).toBe('1080p · 29.97 fps · H.264 · AAC stereo · 6.5 Mbps · 2 watching');
  });
  it('handles low bitrates, unknown codecs and odd resolutions', () => {
    expect(streamDetails({ ...base, resolution: 'unknown', fps: 30, videoCodec: 'mpeg4', bitrateKbps: 812.6, viewers: 0 })).toBe(
      'unknown · 30 fps · MPEG4 · 813 kbps',
    );
  });
  it('flags stale streams and says when nothing is known', () => {
    expect(streamDetails({ ...base, stale: true })).toBe('Stale');
    expect(streamDetails(base)).toBe('No stream info yet');
  });
});
