import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from '../../app/App';
import { createTestRouter } from '../../app/routes';
import { createQueryClient } from '../../api/queries';
import { invoke, setBackend, type Backend } from '../../api/ipc';
import { createMockBackend, idleState } from '../../api/ipc.mock';
import { usePlayerStore } from '../../store/player';
import { mockSportsFollow } from '../../api/ipc.mock.sports';

beforeEach(() => {
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as never;
});

/** The mock backend, recording every command the page sends. */
function recording(): Backend & { calls: { cmd: string; args: unknown }[] } {
  const base = createMockBackend();
  const calls: { cmd: string; args: unknown }[] = [];
  return {
    calls,
    listen: base.listen.bind(base),
    invoke: ((cmd, args) => {
      calls.push({ cmd, args });
      return base.invoke(cmd, args);
    }) as Backend['invoke'],
  };
}

const renderAt = (path: string) => render(<App router={createTestRouter([path])} client={createQueryClient()} />);
const sent = (b: ReturnType<typeof recording>, cmd: string) => b.calls.filter((c) => c.cmd === cmd);
const STREAMS = ['play_channel', 'multiview_set_tile'];

describe('sports command center', () => {
  it('fills the wall with the best live games and lists the rest as also live', async () => {
    const backend = recording();
    setBackend(backend);
    await invoke('sports_follow_set', { follow: mockSportsFollow(true) });
    renderAt('/multiview/sports');
    expect(await screen.findByRole('heading', { name: 'My sports' })).toBeInTheDocument();

    // Five followed games are live and the mock allows 4 tiles: main player + 3 tiles.
    await waitFor(() => expect(sent(backend, 'multiview_set_tile')).toHaveLength(3));
    expect(sent(backend, 'play_channel')).toHaveLength(1);
    expect(sent(backend, 'multiview_set_tile').map((c) => (c.args as { slot: number }).slot)).toEqual([1, 2, 3]);

    const onScreen = screen.getByRole('region', { name: 'On screen' });
    await waitFor(() => expect(within(onScreen).getAllByRole('button', { name: /^Focus the tile/ })).toHaveLength(4));
    const alsoLive = screen.getByRole('region', { name: 'Also live' });
    expect(within(alsoLive).getByRole('button', { name: /^Swap NASCAR Cup Series/ })).toBeInTheDocument();
    const upNext = screen.getByRole('region', { name: 'Up next' });
    expect(within(upNext).getAllByRole('button', { name: /^Details for / }).length).toBeGreaterThan(0);
  });

  it('does not start a removed game again, and swapping pins the new game', async () => {
    const backend = recording();
    setBackend(backend);
    await invoke('sports_follow_set', { follow: mockSportsFollow(true) });
    renderAt('/multiview/sports');
    const onScreen = await screen.findByRole('region', { name: 'On screen' });
    await waitFor(() => expect(within(onScreen).getAllByRole('button', { name: /^Focus the tile/ })).toHaveLength(4));

    const before = STREAMS.flatMap((c) => sent(backend, c)).length;
    fireEvent.click(within(onScreen).getAllByRole('button', { name: 'Remove tile' })[0]!);
    await waitFor(() => expect(within(onScreen).getAllByRole('button', { name: /^Focus the tile/ })).toHaveLength(3));
    // The freed slot is not refilled: removing lowers how many tiles auto-fill keeps.
    expect(STREAMS.flatMap((c) => sent(backend, c))).toHaveLength(before);

    fireEvent.click(within(screen.getByRole('region', { name: 'Also live' })).getAllByRole('button', { name: 'Swap into the focused tile' })[0]!);
    await waitFor(() => expect(STREAMS.flatMap((c) => sent(backend, c)).length).toBe(before + 1));
    await waitFor(() => expect(within(onScreen).getAllByRole('button', { name: 'Let auto-fill replace this tile' })).toHaveLength(1));
  });

  it('asks to choose sports when nothing is followed, and starts nothing', async () => {
    const backend = recording();
    setBackend(backend);
    renderAt('/multiview/sports');
    expect(await screen.findByRole('button', { name: 'Choose sports' })).toBeInTheDocument();
    expect(STREAMS.flatMap((c) => sent(backend, c))).toHaveLength(0);
  });

  it('shows no tile for a main player that was stopped on the way in', async () => {
    // Opening the wall from Home or Command centers stops the player; the backend keeps its last
    // live channel, which must not linger as a dead, black slot 0.
    setBackend(recording());
    renderAt('/multiview/sports');
    expect(await screen.findByRole('button', { name: 'Choose sports' })).toBeInTheDocument();
    act(() => usePlayerStore.getState().setPlayer({ ...idleState(), state: 'Stopped', mode: 'live', channelId: 1 }));
    const slot = await screen.findByRole('group', { name: 'Slot 1: empty' });
    // An empty slot offers the picker; a lingering stopped tile would not.
    await waitFor(() => expect(slot).toHaveAttribute('data-empty'));
    expect(within(slot).getByRole('button', { name: /Add channel/ })).toBeInTheDocument();
  });

  it('stops every tile when leaving', async () => {
    const backend = recording();
    setBackend(backend);
    await invoke('sports_follow_set', { follow: mockSportsFollow(true) });
    const view = renderAt('/multiview/sports');
    await waitFor(() => expect(sent(backend, 'multiview_set_tile')).toHaveLength(3));
    view.unmount();
    await waitFor(() => expect(sent(backend, 'multiview_exit').length).toBeGreaterThan(0));
  });
});
