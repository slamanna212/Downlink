import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
  it('turns the Recordings record dot live while something records, and names it', async () => {
    render(<App router={createTestRouter(['/settings'])} client={createQueryClient()} />);
    const link = await screen.findByRole('link', { name: 'Recordings · Recording now' });
    expect(link).toHaveAttribute('data-recording');
    expect(link.querySelector('[data-rec-dot]')).not.toBeNull();
  });

  it('opens Recordings on the show recording now, then switches to Coming up and filters shows', async () => {
    const user = userEvent.setup();
    render(<App router={createTestRouter(['/recordings'])} client={createQueryClient()} />);
    expect(await screen.findByRole('heading', { level: 2, name: 'Coastline Cup' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Watch live' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Coming up/ }));
    expect(await screen.findByRole('heading', { level: 2, name: 'Coming up' })).toBeInTheDocument();
    await user.type(screen.getByRole('textbox', { name: 'Filter shows' }), 'zzz');
    expect(screen.getByText('No shows match “zzz”.')).toBeInTheDocument();
  });

  it('renders rail entries from caps and a page title', async () => {
    render(<App router={createTestRouter(['/settings'])} client={createQueryClient()} />);
    expect(await screen.findByRole('heading', { name: 'Settings' })).toBeInTheDocument();
    expect(await screen.findByText('Recordings')).toBeInTheDocument();
    expect(screen.getByText('Movies')).toBeInTheDocument();
  });

  it('shows the offline banner when the source is offline', async () => {
    window.history.pushState({}, '', '/?mock=offline');
    setBackend(createMockBackend());
    render(<App router={createTestRouter(['/settings/sources'])} client={createQueryClient()} />);
    expect(await screen.findByText(/Offline — last updated/)).toBeInTheDocument();
    window.history.pushState({}, '', '/');
  });

  it('flags a waiting update on Settings and offers restart and update', async () => {
    window.history.pushState({}, '', '/?mock=update');
    setBackend(createMockBackend());
    render(<App router={createTestRouter(['/'])} client={createQueryClient()} />);
    const settings = await screen.findByRole('link', { name: 'Settings · Update available' });
    expect(settings).toHaveAttribute('href', '/settings/updates');
    expect(screen.getByTestId('rail-notice-dot')).toBeInTheDocument();
    await userEvent.setup().click(settings);
    expect(await screen.findByTestId('update-card')).toBeInTheDocument();
    expect(screen.getByTestId('settings-update-dot')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Restart and update' })).toBeEnabled();
    expect(await screen.findByText(/Faster guide scrolling/)).toBeInTheDocument();
    window.history.pushState({}, '', '/');
  });

  it('has no update dot when up to date', async () => {
    render(<App router={createTestRouter(['/settings/updates'])} client={createQueryClient()} />);
    expect(await screen.findByRole('link', { name: 'Settings' })).toHaveAttribute('href', '/settings');
    expect(screen.queryByTestId('rail-notice-dot')).not.toBeInTheDocument();
    expect(screen.queryByTestId('update-card')).not.toBeInTheDocument();
  });

  it('searches channels and programmes', async () => {
    const user = userEvent.setup();
    render(<App router={createTestRouter(['/search'])} client={createQueryClient()} />);
    await user.type(await screen.findByRole('combobox'), 'harbor');
    expect(await screen.findAllByRole('option')).not.toHaveLength(0);
    expect(screen.getAllByText('Harbor News 24').length).toBeGreaterThan(0);
  });
});
