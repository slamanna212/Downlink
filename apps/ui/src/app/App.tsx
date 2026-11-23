import { useEffect, useMemo } from 'react';
import { MantineProvider } from '@mantine/core';
import { ChannelContextMenu } from '../components/ContextMenu/ChannelContextMenu';
import { Notifications } from '@mantine/notifications';
import { QueryClientProvider, type QueryClient } from '@tanstack/react-query';
import { RouterProvider, type DataRouter } from 'react-router';
import { cssVariablesResolver, theme } from '../theme';
import { createQueryClient, startCatalogBridge, startUpdateBridge } from '../api/queries';
import { startPlayerBridge } from '../store/player';
import { startTaskBridge } from '../store/tasks';
import { applyDensity, useUiStore } from '../store/ui';
import { BlockingPrompt } from '../features/blocking/BlockingPrompt';
import { createAppRouter } from './routes';

export function App({ router, client }: { router?: DataRouter; client?: QueryClient }) {
  const qc = useMemo(() => client ?? createQueryClient(), [client]);
  const r = useMemo(() => router ?? createAppRouter(), [router]);

  useEffect(() => {
    applyDensity(useUiStore.getState().density);
    const stops: Array<() => void> = [];
    let cancelled = false;
    void startCatalogBridge(qc).then((s) => (cancelled ? s() : stops.push(s)));
    void startUpdateBridge(qc).then((s) => (cancelled ? s() : stops.push(s)));
    void startTaskBridge().then((s) => (cancelled ? s() : stops.push(s)));
    void startPlayerBridge().then((s) => (cancelled ? s() : stops.push(s)));
    return () => {
      cancelled = true;
      stops.forEach((s) => s());
    };
  }, [qc]);

  return (
    <MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark">
      <Notifications position="bottom-right" limit={3} autoClose={5000}  />
      <QueryClientProvider client={qc}>
        <RouterProvider router={r} />
      <ChannelContextMenu />
        <BlockingPrompt />
      </QueryClientProvider>
    </MantineProvider>
  );
}
