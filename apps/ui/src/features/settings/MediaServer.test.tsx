import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from '../../app/App';
import { createTestRouter } from '../../app/routes';
import { createQueryClient } from '../../api/queries';
import { invoke, setBackend } from '../../api/ipc';
import { createMockBackend } from '../../api/ipc.mock';

beforeEach(() => {
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as never;
  setBackend(createMockBackend());
});

describe('media server settings', () => {
  it('signs in to Plex, then lists servers to pick', async () => {
    const user = userEvent.setup();
    render(<App router={createTestRouter(['/settings/mediaserver'])} client={createQueryClient()} />);
    await user.click(await screen.findByRole('button', { name: 'Sign in with Plex' }));
    expect(await screen.findByText('Approve the sign-in in your browser. Code DEMO.')).toBeInTheDocument();
    expect(await screen.findByText('Signed in as demo-user', {}, { timeout: 5000 })).toBeInTheDocument();
    expect(await screen.findByLabelText('Server', { selector: 'input' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeInTheDocument();
  }, 10_000);

  it('matches only once a server is chosen', async () => {
    expect(await invoke('media_server_match', { target: { kind: 'vod', id: 2 } })).toBeNull();
    await invoke('plex_select_server', { serverId: 'srv-home' });
    const m = await invoke('media_server_match', { target: { kind: 'vod', id: 2 } });
    expect(m?.serverName).toBe('Home server');
  });
});
