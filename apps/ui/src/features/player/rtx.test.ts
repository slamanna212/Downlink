import { describe, expect, it } from 'vitest';
import type { SessionStats } from '../../api/types';
import { rtxSummary } from './rtx';

const base = { hdr: false, rtxUpscale: null, rtxHdrRequested: false, rtxHdrActive: null } as unknown as SessionStats;

describe('rtxSummary', () => {
  it('is null when RTX Video is not in use', () => {
    expect(rtxSummary(base)).toBeNull();
    expect(rtxSummary(null)).toBeNull();
  });

  it('shows the requested upscale and whether RTX HDR engaged', () => {
    expect(rtxSummary({ ...base, rtxUpscale: 2, rtxHdrRequested: true, rtxHdrActive: true })).toBe(
      'Super Resolution 2× requested · RTX HDR on',
    );
    expect(rtxSummary({ ...base, rtxUpscale: 1.5 })).toBe('Super Resolution 1.5× requested');
    expect(rtxSummary({ ...base, rtxHdrRequested: true, rtxHdrActive: false })).toBe('RTX HDR not engaged');
    expect(rtxSummary({ ...base, rtxHdrRequested: true, hdr: null })).toBe('RTX HDR requested');
    expect(rtxSummary({ ...base, rtxHdrRequested: true, hdr: true })).toBe('RTX HDR skipped (HDR source)');
  });

  it('says when RTX HDR is held off on an SDR monitor', () => {
    expect(rtxSummary({ ...base, rtxUpscale: 2, rtxHdrSdrDisplay: true })).toBe(
      'Super Resolution 2× requested · RTX HDR off (SDR display)',
    );
    expect(rtxSummary({ ...base, rtxHdrSdrDisplay: true, hdr: true })).toBeNull();
  });
});
