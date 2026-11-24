import { t, type MessageKey } from '../../lib/i18n';

/** Where the back pill goes: named after the route the player was opened from. */
const ORIGINS: [string, MessageKey][] = [
  ['/guide', 'nav.guide'],
  ['/channels', 'nav.channels'],
  ['/search', 'nav.search'],
  ['/movies', 'nav.movies'],
  ['/series', 'nav.series'],
  ['/recordings', 'nav.recordings'],
  ['/settings', 'nav.settings'],
];
export function originLabel(path: string | null, hasHistory: boolean): string {
  if (!hasHistory) return t('nav.channels'); // goBack falls back to the channel list
  if (path === '/') return t('nav.home');
  const hit = ORIGINS.find(([p]) => path === p || path?.startsWith(`${p}/`));
  return hit ? t(hit[1]) : t('player.back');
}
