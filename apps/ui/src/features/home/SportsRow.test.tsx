import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from '../../app/App';
import { createTestRouter } from '../../app/routes';
import { createQueryClient } from '../../api/queries';
import { invoke, setBackend, type Backend } from '../../api/ipc';
import { createMockBackend } from '../../api/ipc.mock';

beforeEach(() => {
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as never;
});

/** The mock backend, but the matcher finds nothing. `asked` counts match requests. */
function withNoMatches(): Backend & { asked: () => number } {
  const base = createMockBackend();
  let asked = 0;
  return {
    asked: () => asked,
    listen: base.listen.bind(base),
    invoke: ((cmd, args) => {
      if (cmd !== 'sports_matches') return base.invoke(cmd, args);
      asked += 1;
      return Promise.resolve([]);
    }) as Backend['invoke'],
  };
}

const FOLLOW = { version: 1, includeReplays: false, leagues: { nfl: { teams: ['chi'] } }, series: { f1: { sessions: ['race' as const, 'qualifying' as const] } } };

const renderHome = () => render(<App router={createTestRouter(['/'])} client={createQueryClient()} />);

describe('home: Your sports', () => {
  it('is hidden while nothing is followed', async () => {
    setBackend(createMockBackend());
    renderHome();
    expect(await screen.findByRole('heading', { name: 'On now in your favorites' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Your sports' })).not.toBeInTheDocument();
  });

  it('is hidden when things are followed but nothing matches', async () => {
    const backend = withNoMatches();
    setBackend(backend);
    await invoke('sports_follow_set', { follow: FOLLOW });
    renderHome();
    expect(await screen.findByRole('heading', { name: 'On now in your favorites' })).toBeInTheDocument();
    // Wait until the matches were asked for and answered (empty), then check the row never appeared.
    await waitFor(() => expect(backend.asked()).toBeGreaterThan(0));
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole('heading', { name: 'Your sports' })).not.toBeInTheDocument();
  });

  it('shows followed matches, on air first, labeled by team or series', async () => {
    setBackend(createMockBackend());
    await invoke('sports_follow_set', { follow: FOLLOW });
    renderHome();
    expect(await screen.findByRole('heading', { name: 'Your sports' })).toBeInTheDocument();
    const row = screen.getByTestId('home-sports');
    const cards = within(row).getAllByRole('button', { name: /^(Watch|Details for) / });
    // The Bears event channel and the live Bears game are on now; both come before the F1 qualifying.
    expect(cards[0]).toHaveAccessibleName(/^Watch /);
    expect(cards.at(-1)).toHaveAccessibleName(/^Details for /);
    expect(within(row).getAllByText(/^Bears · NFL/).length).toBeGreaterThan(0);
    expect(within(row).getByText(/· Qualifying ·/)).toBeInTheDocument();
    expect(within(row).getAllByText('Live').length).toBeGreaterThan(0);
    // Replays stay out unless asked for.
    expect(within(row).queryByText('Replay')).not.toBeInTheDocument();
  });

  it('lists the other channels showing the same game', async () => {
    setBackend(createMockBackend());
    await invoke('sports_follow_set', { follow: FOLLOW });
    renderHome();
    const row = await screen.findByTestId('home-sports');
    const more = await within(row).findByRole('button', { name: '2 more channels showing this' });
    expect(more).toHaveTextContent('+2');
    fireEvent.click(more);
    // jsdom leaves Mantine's open transition at display:none, so query hidden nodes too.
    const dialog = await screen.findByRole('dialog', { name: /Choose a stream/, hidden: true });
    // The guide's pick first, then the two others.
    const rows = within(dialog).getAllByRole('listitem', { hidden: true });
    expect(rows).toHaveLength(3);
    expect(within(rows[0]!).getByText('Listed in guide')).toBeInTheDocument();
    expect(within(rows[1]!).getByText('Same event')).toBeInTheDocument();
  });
});
