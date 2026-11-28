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

/** Opens the Add teams dialog and returns it (hidden: jsdom leaves Mantine's transition at display:none). */
async function openPicker(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: 'Add teams' }));
  return screen.findByRole('dialog', { name: 'Add teams and series', hidden: true });
}

const opts = { hidden: true };

describe('Settings → Sports', () => {
  it('lists nothing until something is followed', async () => {
    renderSports();
    expect(await screen.findByText('Nothing followed yet. Add teams to see their games on Home.')).toBeInTheDocument();
  });

  it('follows a series with race, qualifying and practice on and shows off', async () => {
    const user = userEvent.setup();
    renderSports();
    const dialog = await openPicker(user);
    const f1 = within(dialog).getByRole('group', { name: 'Formula 1', ...opts });
    await user.click(within(f1).getByRole('button', { name: 'Follow', ...opts }));
    await waitFor(() => expect(within(f1).getByRole('button', { name: 'Race', ...opts })).toHaveAttribute('aria-pressed', 'true'));
    expect(within(f1).getByRole('button', { name: 'Qualifying', ...opts })).toHaveAttribute('aria-pressed', 'true');
    expect(within(f1).getByRole('button', { name: 'Practice', ...opts })).toHaveAttribute('aria-pressed', 'true');
    expect(within(f1).getByRole('button', { name: 'Shows', ...opts })).toHaveAttribute('aria-pressed', 'false');
    await user.click(within(f1).getByRole('button', { name: 'Shows', ...opts }));
    await waitFor(async () => {
      const f = await invoke('sports_follow_get', {});
      expect(f.series.f1?.sessions).toEqual(['race', 'qualifying', 'practice', 'shows']);
    });
  });

  it('follows a league with all games, then narrows it to a team', async () => {
    const user = userEvent.setup();
    renderSports();
    const dialog = await openPicker(user);
    const nfl = within(dialog).getByRole('group', { name: 'NFL', ...opts });
    const all = within(nfl).getByRole('button', { name: 'All games', ...opts });
    expect(all).toHaveAttribute('aria-pressed', 'false');
    await user.click(all);
    await waitFor(() => expect(within(nfl).getByRole('button', { name: 'All games', ...opts })).toHaveAttribute('aria-pressed', 'true'));
    // Every team shows as picked while All games is on; picking one narrows to it.
    expect(within(nfl).getByRole('button', { name: /Chicago Bears/, ...opts })).toHaveAttribute('aria-pressed', 'true');
    await user.click(within(nfl).getByRole('button', { name: /Chicago Bears/, ...opts }));
    await waitFor(async () => expect((await invoke('sports_follow_get', {})).leagues).toEqual({ nfl: { teams: ['chi'] } }));
    expect(within(nfl).getByRole('button', { name: 'All games', ...opts })).toHaveAttribute('aria-pressed', 'false');
    expect(within(nfl).getByRole('button', { name: /Green Bay Packers/, ...opts })).toHaveAttribute('aria-pressed', 'false');
  });

  it('searches teams across leagues', async () => {
    const user = userEvent.setup();
    renderSports();
    const dialog = await openPicker(user);
    await user.type(within(dialog).getByRole('textbox', { name: 'Search teams, leagues or series', ...opts }), 'kraken');
    expect(within(dialog).getByRole('group', { name: 'NHL', ...opts })).toBeInTheDocument();
    expect(within(dialog).queryByRole('group', { name: 'NFL', ...opts })).not.toBeInTheDocument();
  });

  it('lists what is followed and removes a team or the whole league', async () => {
    const user = userEvent.setup();
    await invoke('sports_follow_set', { follow: { version: 1, includeReplays: false, leagues: { nfl: { teams: ['chi', 'gb'] } }, series: {} } });
    renderSports();
    await user.click(await screen.findByRole('button', { name: 'Remove Bears' }));
    await waitFor(async () => expect((await invoke('sports_follow_get', {})).leagues).toEqual({ nfl: { teams: ['gb'] } }));
    // Removing the last team unfollows the league rather than switching it to all games.
    await user.click(await screen.findByRole('button', { name: 'Remove Packers' }));
    await waitFor(async () => expect((await invoke('sports_follow_get', {})).leagues).toEqual({}));
    expect(await screen.findByText('Nothing followed yet. Add teams to see their games on Home.')).toBeInTheDocument();
  });

  it('says when nothing is followed in the preview', async () => {
    renderSports();
    expect(await screen.findByText('Follow a league or series to see matches here.')).toBeInTheDocument();
  });

  it('previews matches for what is followed, grouped by when, and replays only when included', async () => {
    const user = userEvent.setup();
    await invoke('sports_follow_set', { follow: { version: 1, includeReplays: false, leagues: {}, series: { f1: { sessions: ['race', 'qualifying', 'practice'] } } } });
    renderSports();
    const preview = await screen.findByTestId('sports-preview');
    expect(within(preview).getByText(/· Qualifying ·/)).toBeInTheDocument();
    expect(within(preview).getAllByRole('group').length).toBeGreaterThan(0);
    expect(within(preview).queryByText('Replay')).not.toBeInTheDocument();
    await user.click(screen.getByRole('switch', { name: 'Include replays' }));
    expect(await within(screen.getByTestId('sports-preview')).findByText('Replay')).toBeInTheDocument();
  });
});
