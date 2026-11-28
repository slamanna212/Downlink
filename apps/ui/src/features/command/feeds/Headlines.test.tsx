import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { App } from '../../../app/App';
import { createTestRouter } from '../../../app/routes';
import { createQueryClient } from '../../../api/queries';
import { invoke, setBackend, type Backend } from '../../../api/ipc';
import { createMockBackend } from '../../../api/ipc.mock';
import type { CommandCenterRail, CommandCenterSave } from '../../../api/types';

beforeEach(() => {
  window.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} unobserve() {} disconnect() {} } as never;
});

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
const saves = (b: ReturnType<typeof recording>) => sent(b, 'command_center_save').map((c) => (c.args as { center: CommandCenterSave }).center);

async function seedCenter(rail: CommandCenterRail): Promise<number> {
  return invoke('command_center_save', {
    center: { id: null, name: 'Election desk', layoutStyle: 'grid', audible: 0, rail, tiles: [0, 1].map((slot) => ({ slot, channelId: 100 + slot })) },
  });
}

describe('command center headlines', () => {
  it('shows headlines and the ticker from the center feeds, and watches them while open', async () => {
    const backend = recording();
    setBackend(backend);
    const feeds = await invoke('feeds_list', {});
    expect(feeds.length).toBeGreaterThan(0);
    const id = await seedCenter({ feeds: feeds.map((f) => ({ id: f.id })) });
    const view = renderAt(`/multiview/c/${id}`);

    const section = await screen.findByRole('region', { name: 'Headlines' });
    await waitFor(() => expect(within(section).getAllByRole('button', { name: /^Open .* in your browser$/ }).length).toBeGreaterThan(0));
    expect(screen.getByRole('region', { name: 'Headline ticker' })).toBeInTheDocument();
    await waitFor(() => expect(sent(backend, 'feeds_watch').some((c) => (c.args as { feedIds: number[] }).feedIds.length === feeds.length)).toBe(true));

    // Opening a headline goes to the system browser through the backend.
    fireEvent.click(within(section).getAllByRole('button', { name: /^Open / })[0]!);
    await waitFor(() => expect(sent(backend, 'open_external_link')).toHaveLength(1));

    // The ticker switch is saved on the center.
    fireEvent.click(screen.getByRole('button', { name: 'Hide headline ticker' }));
    await waitFor(() => expect(saves(backend).at(-1)?.rail.ticker).toBe(false));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Headline ticker' })).not.toBeInTheDocument());
    // Rail-only saves keep the stored tiles.
    expect(saves(backend).at(-1)?.tiles).toEqual([{ slot: 0, channelId: 100 }, { slot: 1, channelId: 101 }]);

    view.unmount();
    await waitFor(() => expect(sent(backend, 'feeds_unwatch').length).toBeGreaterThan(0));
  });

  it('adds a feed by address and attaches it to the center', async () => {
    const backend = recording();
    setBackend(backend);
    const id = await seedCenter({});
    renderAt(`/multiview/c/${id}`);

    const section = await screen.findByRole('region', { name: 'Headlines' });
    expect(within(section).getByText('Add a news feed to see live headlines here.')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Headline ticker' })).not.toBeInTheDocument();

    fireEvent.click(within(section).getAllByRole('button', { name: 'Manage feeds' })[0]!);
    const dialog = await screen.findByRole('dialog', { name: 'News feeds' });
    fireEvent.change(within(dialog).getByLabelText('Feed address'), { target: { value: 'https://desk.example/feed.xml' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Add feed' }));

    await waitFor(() => expect(sent(backend, 'feed_add')).toHaveLength(1));
    await waitFor(() => expect(saves(backend).at(-1)?.rail.feeds?.length).toBe(1));
    await waitFor(() => expect(screen.getByRole('region', { name: 'Headline ticker' })).toBeInTheDocument());
  });
});
