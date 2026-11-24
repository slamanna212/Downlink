import {
  IconHome,
  IconListDetails,
  IconCalendarTime,
  IconMovie,
  IconSearch,
  IconSettings,
  IconDeviceTv,
} from '@tabler/icons-react';
import { IconRecordings } from '../components/Icon/IconRecordings';
import type { Caps } from '../api/types';
import type { TablerIcon } from '../components/Icon/Icon';
import type { MessageKey } from '../lib/i18n';

export interface NavEntry {
  id: string;
  to: string;
  label: MessageKey;
  icon: TablerIcon;
  /** Key hint shown in the expanded rail. */
  hint?: string;
  /** Only shown when this predicate on Caps is true. Never keyed on source kind. */
  requires?: (caps: Caps) => boolean;
}

export const PRIMARY_NAV: NavEntry[] = [
  { id: 'search', to: '/search', label: 'nav.search', icon: IconSearch, hint: '#mod' },
  { id: 'home', to: '/', label: 'nav.home', icon: IconHome },
  { id: 'guide', to: '/guide', label: 'nav.guide', icon: IconCalendarTime, hint: 'G' },
  { id: 'channels', to: '/channels', label: 'nav.channels', icon: IconListDetails, hint: 'L' },
  { id: 'movies', to: '/movies', label: 'nav.movies', icon: IconMovie, requires: (c) => c.vod },
  { id: 'series', to: '/series', label: 'nav.series', icon: IconDeviceTv, requires: (c) => c.vod },
  { id: 'recordings', to: '/recordings', label: 'nav.recordings', icon: IconRecordings, requires: (c) => c.dvr },
];

export const SETTINGS_NAV: NavEntry = { id: 'settings', to: '/settings', label: 'nav.settings', icon: IconSettings };

export function visibleNav(caps: Caps): NavEntry[] {
  return PRIMARY_NAV.filter((e) => !e.requires || e.requires(caps));
}
