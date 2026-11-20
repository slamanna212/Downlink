import { describe, expect, it } from 'vitest';
import { gridColumns, progressOf, resumeUseful, runtimeLabel } from './vod';

describe('vod helpers', () => {
  it('offers resume only between 2 s from start and 2 s from end', () => {
    expect(resumeUseful(null, 100)).toBe(false);
    expect(resumeUseful(2, 100)).toBe(false);
    expect(resumeUseful(3, 100)).toBe(true);
    expect(resumeUseful(99, 100)).toBe(false);
    expect(resumeUseful(50, null)).toBe(true);
  });
  it('progress', () => {
    expect(progressOf(50, 100)).toBe(0.5);
    expect(progressOf(1, 100)).toBeNull();
  });
  it('runtime and columns', () => {
    expect(runtimeLabel(5400)).toBe('1 h 30 min');
    expect(runtimeLabel(2700)).toBe('45 min');
    expect(gridColumns(1000, 160, 16)).toBe(5);
    expect(gridColumns(100, 160, 16)).toBe(1);
  });
});
