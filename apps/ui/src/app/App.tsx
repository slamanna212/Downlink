import { useEffect, useMemo } from 'react';
import { MantineProvider } from '@mantine/core';
import { QueryClientProvider, type QueryClient } from '@tanstack/react-query';
import { RouterProvider, type DataRouter } from 'react-router';
import { cssVariablesResolver, theme } from '../theme';
import { createQueryClient, startCatalogBridge } from '../api/queries';
import { startPlayerBridge } from '../store/player';
import { applyDensity, useUiStore } from '../store/ui';
import { createAppRouter } from './routes';

export function App({ router, client }: { router?: DataRouter; client?: QueryClient }) {
  const qc = useMemo(() => client ?? createQueryClient(), [client]);
  const r = useMemo(() => router ?? createAppRouter(), [router]);

  useEffect(() => {
    applyDensity(useUiStore.getState().density);
    const stops: Array<() => void> = [];
    let cancelled = false;
    void startCatalogBridge(qc).then((s) => (cancelled ? s() : stops.push(s)));
    void startPlayerBridge().then((s) => (cancelled ? s() : stops.push(s)));
    return () => {
      cancelled = true;
      stops.forEach((s) => s());
    };
  }, [qc]);

  return (
    <MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark">
      <QueryClientProvider client={qc}>
        <RouterProvider router={r} />
      </QueryClientProvider>
    </MantineProvider>
  );
}
