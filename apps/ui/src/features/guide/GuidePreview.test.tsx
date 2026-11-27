import { MantineProvider } from '@mantine/core';
import { QueryClientProvider } from '@tanstack/react-query';
import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router';
import { beforeEach, expect, it, vi } from 'vitest';
import { invoke, setBackend } from '../../api/ipc';
import { createMockBackend } from '../../api/ipc.mock';
import { createQueryClient } from '../../api/queries';
import { usePlayerStore } from '../../store/player';
import { GuidePreview } from './GuidePreview';

const idle = usePlayerStore.getState().player;

beforeEach(() => {
  window.matchMedia ??= ((media: string) => ({ matches: false, media, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false, onchange: null })) as never;
  window.ResizeObserver ??= class { observe() {} disconnect() {} } as never;
  setBackend(createMockBackend());
  usePlayerStore.getState().setPlayer(idle);
});

it('shows the playing channel and programme until the mini player stops', async () => {
  const focused = await invoke('get_channel', { channelId: 100 });
  const playing = await invoke('get_channel', { channelId: 101 });
  const [entry] = await invoke('get_now_next', { channelIds: [playing.id] });
  const current = entry?.now;
  expect(current).toBeTruthy();
  if (!current) throw new Error('Mock channel has no current programme');
  const programme = { ...current, channelId: focused.id, title: 'Focused programme' };
  const onInfo = vi.fn();
  const onFavourite = vi.fn();
  const client = createQueryClient();
  const preview = (title: string) => (
    <MantineProvider>
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <GuidePreview programme={{ ...programme, title }} channel={focused} state="now" onInfo={onInfo} onFavourite={onFavourite} onDetails={vi.fn()} onPrimary={vi.fn()} />
        </MemoryRouter>
      </QueryClientProvider>
    </MantineProvider>
  );
  const view = render(preview('Focused programme'));
  expect(screen.getByRole('heading', { name: 'Focused programme' })).toBeInTheDocument();

  act(() => usePlayerStore.getState().setPlayer({ ...idle, state: 'Playing', mode: 'live', channelId: playing.id }));
  expect(await screen.findByText(new RegExp(playing.name))).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: current.title ?? undefined })).toBeInTheDocument();
  view.rerender(preview('New selection'));
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'More info' }));
  expect(onInfo).toHaveBeenCalledWith(current, playing);
  await user.click(screen.getByRole('button', { name: /add to fav/i }));
  expect(onFavourite).toHaveBeenCalledWith(playing);
  expect(screen.getByRole('button', { name: 'Full screen' })).toBeInTheDocument();

  act(() => usePlayerStore.getState().setPlayer({ ...idle, state: 'Stopped', channelId: playing.id }));
  expect(await screen.findByRole('heading', { name: 'New selection' })).toBeInTheDocument();
});
