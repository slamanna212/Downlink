import { useQuery } from '@tanstack/react-query';
import { invoke } from '../../api/ipc';

/** Multiview entry points show only where the machine can show more than one tile. */
export function useMultiviewAvailable(): boolean {
  const q = useQuery({ queryKey: ['multiview'], queryFn: () => invoke('get_multiview', {}), staleTime: 60_000 });
  return (q.data?.maxTiles ?? 1) > 1;
}
