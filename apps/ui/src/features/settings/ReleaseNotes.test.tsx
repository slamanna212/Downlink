import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('../../api/ipc', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { ReleaseNotes } from './ReleaseNotes';

function renderIt(fallback?: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <ReleaseNotes version="1.2.3" fallback={fallback} />
    </QueryClientProvider>,
  );
}

describe('ReleaseNotes', () => {
  it('renders the release body as Markdown with images', async () => {
    invoke.mockResolvedValue({ version: '1.2.3', body: '## Big news\n![shot](https://example.invalid/a.png)', source: 'github' });
    renderIt('old notes');
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Big news' })).toBeTruthy());
    expect(screen.getByAltText('shot')).toBeTruthy();
    expect(screen.queryByText('old notes')).toBeNull();
  });

  it('falls back to the manifest notes when the command fails', async () => {
    invoke.mockRejectedValue(new Error('offline'));
    renderIt('Fixed things');
    await waitFor(() => expect(screen.getByText('Fixed things')).toBeTruthy());
  });

  it('renders nothing without notes', async () => {
    invoke.mockResolvedValue({ version: '1.2.3', body: null, source: 'manifest' });
    const { container } = renderIt();
    await waitFor(() => expect(invoke).toHaveBeenCalled());
    expect(container.querySelector('[data-testid="release-notes"]')).toBeNull();
  });
});
