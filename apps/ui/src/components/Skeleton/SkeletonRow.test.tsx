import { act, render, screen } from '@testing-library/react';
import { MantineProvider } from '@mantine/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { theme } from '../../theme';
import { SkeletonList } from './SkeletonRow';

describe('SkeletonList label', () => {
  beforeEach(() => {
    window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener: () => undefined, removeEventListener: () => undefined, addListener: () => undefined, removeListener: () => undefined, dispatchEvent: () => false, onchange: null })) as unknown as typeof window.matchMedia;
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());
  it('says what it waits on only after 300 ms', () => {
    render(
      <MantineProvider theme={theme}>
        <SkeletonList rows={2} label="Fetching guide…" />
      </MantineProvider>,
    );
    expect(screen.queryByText('Fetching guide…')).toBeNull();
    act(() => void vi.advanceTimersByTime(299));
    expect(screen.queryByText('Fetching guide…')).toBeNull();
    act(() => void vi.advanceTimersByTime(2));
    expect(screen.getByRole('status')).toHaveTextContent('Fetching guide…');
  });
});
