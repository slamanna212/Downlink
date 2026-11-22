import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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

/** The mock backend, but reporting no RTX card. */
function withoutRtx(): Backend {
  const base = createMockBackend();
  return {
    listen: base.listen.bind(base),
    invoke: ((cmd, args) =>
      cmd === 'get_video_caps' ? Promise.resolve({ rtxVideo: false, rtxAdapter: null }) : base.invoke(cmd, args)) as Backend['invoke'],
  };
}

const renderPlayback = () =>
  render(<App router={createTestRouter(['/settings/playback'])} client={createQueryClient()} />);

describe('RTX Video settings', () => {
  it('turns on Super Resolution and RTX HDR with an RTX card, and explains the NVIDIA app step', async () => {
    setBackend(createMockBackend());
    const user = userEvent.setup();
    renderPlayback();
    const vsr = await screen.findByRole('switch', { name: 'RTX Super Resolution' });
    await waitFor(() => expect(vsr).toBeEnabled());
    expect(screen.getByText(/under RTX Video in the NVIDIA app \(Demo RTX adapter\)/)).toBeInTheDocument();
    await user.click(vsr);
    await user.click(screen.getByRole('switch', { name: 'RTX HDR' }));
    await waitFor(async () => {
      const s = await invoke('get_settings', {});
      expect(s.rtxSuperResolution).toBe(true);
      expect(s.rtxHdr).toBe(true);
    });
  });

  it('greys both out without an RTX card', async () => {
    setBackend(withoutRtx());
    renderPlayback();
    expect(await screen.findByRole('switch', { name: 'RTX Super Resolution' })).toBeDisabled();
    expect(screen.getByRole('switch', { name: 'RTX HDR' })).toBeDisabled();
    expect(screen.getAllByText('Needs an NVIDIA RTX graphics card on Windows.')).toHaveLength(2);
    expect(screen.queryByText(/NVIDIA app/)).not.toBeInTheDocument();
  });

  it('greys out RTX HDR while HDR output is off', async () => {
    setBackend(createMockBackend());
    await invoke('set_settings', { patch: { hdrOutput: 'off' } });
    renderPlayback();
    const hdr = await screen.findByRole('switch', { name: 'RTX HDR' });
    await waitFor(() => expect(hdr).toBeDisabled());
    expect(screen.getByText('Needs HDR output set to Auto.')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'RTX Super Resolution' })).toBeEnabled();
  });
});
