import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from '../../app/App';
import { createTestRouter } from '../../app/routes';
import { createQueryClient } from '../../api/queries';
import { invoke, setBackend, type Backend } from '../../api/ipc';
import { createMockBackend } from '../../api/ipc.mock';
import { usePlayerStore } from '../../store/player';

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

/** A saved center with six invented channels (the mock allows four tiles). */
async function seedCenter(): Promise<number> {
  return invoke('command_center_save', {
    center: {
      id: null,
      name: 'Morning briefing',
      layoutStyle: 'grid',
      audible: 0,
      rail: {},
      tiles: [0, 1, 2, 3, 4, 5].map((slot) => ({ slot, channelId: 100 + slot })),
    },
  });
}

describe('saved command center', () => {
  it('starts its tiles clipped to the budget and lists the rest as not on screen', async () => {
    const backend = recording();
    setBackend(backend);
    const id = await seedCenter();
    renderAt(`/multiview/c/${id}`);
    expect(await screen.findByRole('heading', { name: 'Morning briefing' })).toBeInTheDocument();

    await waitFor(() => expect(sent(backend, 'multiview_set_tile')).toHaveLength(3));
    expect(sent(backend, 'play_channel').map((c) => (c.args as { channelId: number }).channelId)).toEqual([100]);
    expect(sent(backend, 'multiview_set_tile').map((c) => c.args)).toEqual([
      { slot: 1, channelId: 101 },
      { slot: 2, channelId: 102 },
      { slot: 3, channelId: 103 },
    ]);
    const onScreen = screen.getByRole('region', { name: 'On screen' });
    await waitFor(() => expect(within(onScreen).getAllByRole('button', { name: /^Focus the tile/ })).toHaveLength(4));
    const off = screen.getByRole('region', { name: 'Not on screen' });
    expect(within(off).getAllByRole('button', { name: /^Swap .* into the focused tile$/ })).toHaveLength(2);

    // Nothing starts twice once the state settles.
    await new Promise((r) => setTimeout(r, 300));
    expect(sent(backend, 'multiview_set_tile')).toHaveLength(3);
    expect(sent(backend, 'play_channel')).toHaveLength(1);
  });

  it('stops every tile when leaving', async () => {
    const backend = recording();
    setBackend(backend);
    const id = await seedCenter();
    const view = renderAt(`/multiview/c/${id}`);
    await waitFor(() => expect(sent(backend, 'multiview_set_tile')).toHaveLength(3));
    view.unmount();
    await waitFor(() => expect(sent(backend, 'multiview_exit').length).toBeGreaterThan(0));
  });

  it('says so and starts nothing when the center is gone', async () => {
    const backend = recording();
    setBackend(backend);
    renderAt('/multiview/c/42');
    expect(await screen.findByText('This command center no longer exists.')).toBeInTheDocument();
    expect(sent(backend, 'play_channel')).toHaveLength(0);
    expect(sent(backend, 'multiview_set_tile')).toHaveLength(0);
  });

  it('saves the multiview wall as a new command center', async () => {
    // Let the previous test's session release (a stop one tick after unmount) land first.
    await new Promise((r) => setTimeout(r, 10));
    const backend = recording();
    setBackend(backend);
    await invoke('play_channel', { channelId: 100 });
    await invoke('multiview_set_tile', { slot: 1, channelId: 104 });
    renderAt('/multiview');

    const save = await screen.findByRole('button', { name: 'Save as command center' });
    // The main player (slot 0) is live before saving.
    await waitFor(() => expect(usePlayerStore.getState().player.channelId).toBe(100));
    await waitFor(() => expect(save).toBeEnabled());
    fireEvent.click(save);
    const name = await screen.findByLabelText('Name');
    fireEvent.change(name, { target: { value: '  Late desk  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(sent(backend, 'command_center_save')).toHaveLength(1));
    const { center } = sent(backend, 'command_center_save')[0]!.args as { center: { id: number | null; name: string; tiles: unknown[] } };
    expect(center.id).toBeNull();
    expect(center.name).toBe('Late desk');
    expect(center.tiles).toEqual([{ slot: 0, channelId: 100 }, { slot: 1, channelId: 104 }]);
    await waitFor(async () => expect((await invoke('command_centers_list', {})).map((c) => [c.name, c.tileCount])).toEqual([['Late desk', 2]]));
    // The wall is now that center: the next save offers "Save as new".
    expect(await screen.findByRole('button', { name: 'Save as new' })).toBeInTheDocument();
  });
});

describe('command center page', () => {
  it('is in the nav and lists My sports first, then saved centers', async () => {
    const backend = recording();
    setBackend(backend);
    await seedCenter();
    renderAt('/command');
    expect(await screen.findByRole('heading', { level: 1, name: 'Command center' })).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: 'Primary' });
    expect(within(nav).getByText('Command center')).toBeInTheDocument();

    const cards = screen.getByTestId('command-cards');
    await waitFor(() => expect(within(cards).getAllByRole('button', { name: /^(Open|Choose sports)/ })).toHaveLength(2));
    const names = within(cards).getAllByRole('button', { name: /^(Open|Choose sports)/ }).map((b) => b.getAttribute('aria-label'));
    expect(names).toEqual(['Choose sports for My sports', 'Open Morning briefing']);
    expect(within(cards).getByText('6 channels')).toBeInTheDocument();
    // Browsing the list starts nothing.
    expect(sent(backend, 'play_channel')).toHaveLength(0);
  });

  it('explains how to save one when there are none', async () => {
    setBackend(recording());
    renderAt('/command');
    expect(await screen.findByText('No saved command centers yet')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Choose sports for My sports' })).toBeInTheDocument();
  });
});
