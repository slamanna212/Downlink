import { render, screen } from '@testing-library/react';
import { describe, expect, it, beforeEach } from 'vitest';
import { App } from './App';
import { createTestRouter } from './routes';
import { createQueryClient } from '../api/queries';
import { setBackend } from '../api/ipc';
import { createMockBackend } from '../api/ipc.mock';

beforeEach(() => {
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as never;
  setBackend(createMockBackend());
});

describe('app shell', () => {
  it('renders rail entries from caps and a page title', async () => {
    render(<App router={createTestRouter(['/guide'])} client={createQueryClient()} />);
    expect(await screen.findByRole('heading', { name: 'Guide' })).toBeInTheDocument();
    expect(await screen.findByText('Recordings')).toBeInTheDocument();
    expect(screen.getByText('Movies')).toBeInTheDocument();
  });
});
