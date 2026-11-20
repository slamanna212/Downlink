import { describe, expect, it } from 'vitest';
import { parseReleaseNotes } from './releaseNotes';

describe('parseReleaseNotes', () => {
  it('turns headings, bullets and paragraphs into blocks', () => {
    const md = [
      "## What's new",
      'Faster guide scrolling and a',
      'new stream switcher.',
      '',
      '### Fixes',
      '- **guide:** Fix the time header (a1b2c3d)',
      '* See [the docs](https://example.invalid/docs) for `hwdec`',
      '',
      '---',
    ].join('\r\n');
    expect(parseReleaseNotes(md)).toEqual([
      { kind: 'heading', text: "What's new" },
      { kind: 'para', text: 'Faster guide scrolling and a new stream switcher.' },
      { kind: 'heading', text: 'Fixes' },
      { kind: 'list', items: ['guide: Fix the time header (a1b2c3d)', 'See the docs for hwdec'] },
    ]);
  });

  it('returns nothing for empty notes', () => {
    expect(parseReleaseNotes('  \n\n')).toEqual([]);
  });
});
