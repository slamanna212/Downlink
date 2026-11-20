import { describe, expect, it } from 'vitest';
import { resolveImg } from './dlimg';

describe('resolveImg', () => {
  it('rewrites dlimg on Windows only', () => {
    expect(resolveImg('dlimg://abc/64', true)).toBe('http://dlimg.localhost/abc/64');
    expect(resolveImg('dlimg://abc/64', false)).toBe('dlimg://abc/64');
    expect(resolveImg('data:image/svg+xml;utf8,x', true)).toBe('data:image/svg+xml;utf8,x');
    expect(resolveImg(null, true)).toBeNull();
  });
});
