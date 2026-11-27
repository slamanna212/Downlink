import { render, screen, waitFor, within } from '@testing-library/react';
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

const renderSports = () => render(<App router={createTestRouter(['/settings/sports'])} client={createQueryClient()} />);

describe('Settings → Sports', () => {
  it('follows a series with race, qualifying and practice on and shows off', async () => {
    const user = userEvent.setup();
    renderSports();
    expect(screen.queryByRole('group', { name: 'Sessions for Formula 1' })).not.toBeInTheDocument();
    await user.click(await screen.findByRole('switch', { name: 'Follow Formula 1' }));
    const sessions = await screen.findByRole('group', { name: 'Sessions for Formula 1' });
    expect(within(sessions).getByRole('checkbox', { name: 'Race' })).toBeChecked();
    expect(within(sessions).getByRole('checkbox', { name: 'Qualifying' })).toBeChecked();
    expect(within(sessions).getByRole('checkbox', { name: 'Practice' })).toBeChecked();
    expect(within(sessions).getByRole('checkbox', { name: 'Shows' })).not.toBeChecked();
    await user.click(within(sessions).getByRole('checkbox', { name: 'Shows' }));
    await waitFor(async () => {
      const f = await invoke('sports_follow_get', {});
      expect(f.series.f1?.sessions).toEqual(['race', 'qualifying', 'practice', 'shows']);
    });
  });

  it('follows a league with all games, then narrows it to a team', async () => {
    const user = userEvent.setup();
    renderSports();
    await user.click(await screen.findByRole('switch', { name: 'Follow NFL' }));
    const teams = await screen.findByRole('group', { name: 'Teams in NFL' });
    const all = within(teams).getByRole('button', { name: 'All games' });
    expect(all).toHaveAttribute('aria-pressed', 'true');
    await user.click(within(teams).getByRole('button', { name: /Bears/ }));
    expect(all).toHaveAttribute('aria-pressed', 'false');
    expect(within(teams).getByRole('button', { name: /Bears/ })).toHaveAttribute('aria-pressed', 'true');
    await waitFor(async () => expect((await invoke('sports_follow_get', {})).leagues).toEqual({ nfl: { teams: ['chi'] } }));
  });

  it('previews matches for what is followed, and replays only when included', async () => {
    const user = userEvent.setup();
    renderSports();
    expect(await screen.findByText('Follow a league or series to see matches here.')).toBeInTheDocument();
    await user.click(await screen.findByRole('switch', { name: 'Follow Formula 1' }));
    const preview = await screen.findByTestId('sports-preview');
    expect(within(preview).getByText('Qualifying')).toBeInTheDocument();
    expect(within(preview).queryByText('Replay')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Include replays' }));
    expect(await within(screen.getByTestId('sports-preview')).findByText('Replay')).toBeInTheDocument();
  });
});
