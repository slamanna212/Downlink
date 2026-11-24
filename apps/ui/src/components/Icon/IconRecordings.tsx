import { createReactComponent } from '@tabler/icons-react';

/**
 * Rail glyph for Recordings: a screen on a stand with a filled record dot. Drawn to Tabler rules (24 grid,
 * stroke 1.75, round caps) so it sits with the other rail icons; the stand tells it apart from TV shows
 * (`IconDeviceTv`, antenna on top). The dot follows the stroke colour unless the rail marks it live
 * (`[data-rec-dot]`, Rail.module.css).
 */
export const IconRecordings = createReactComponent('outline', 'recordings', 'Recordings', [
  ['path', { d: 'M3 5m0 2a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v9a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z', key: 'svg-0' }],
  ['path', { d: 'M9 21h6', key: 'svg-1' }],
  ['path', { d: 'M12 18v3', key: 'svg-2' }],
  ['circle', { cx: '12', cy: '11.5', r: '2.75', fill: 'currentColor', stroke: 'none', 'data-rec-dot': '', key: 'svg-3' }],
]);
