import { useDeferredValue, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { Spotlight, spotlight, type SpotlightActionData, type SpotlightActionGroupData } from '@mantine/spotlight';
import {
  IconArrowsMaximize,
  IconDeviceTv,
  IconHome,
  IconCalendarTime,
  IconListDetails,
  IconMovie,
  IconCircle,
  IconPalette,
  IconPlayerPlay,
  IconPlugConnected,
  IconRefresh,
  IconSearch,
  IconStethoscope,
  IconLayoutSidebar,
} from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps, useSources } from '../../api/queries';
import { dl } from '../../theme';
import { Icon } from '../../components/Icon/Icon';
import { useUiStore } from '../../store/ui';
import { dayLabel, formatClock } from '../../lib/time';
import { onOpenPalette } from '../../lib/palette';
import { t } from '../../lib/i18n';
import { activateProgramme } from '../guide/activate';
import { playChannel } from '../player/actions';


const ic = (g: Parameters<typeof Icon>[0]['icon']) => <Icon icon={g} size={16} />;

/** Ctrl/Cmd+K palette: channels, programmes, screens, actions and settings sections (PLAN §8.4). */
export function CommandPalette() {
  const navigate = useNavigate();
  const { caps } = useCaps();
  const sources = useSources();
  const density = useUiStore((s) => s.density);
  const setDensity = useUiStore((s) => s.setDensity);
  const railPref = useUiStore((s) => s.railPref);
  const setRailPref = useUiStore((s) => s.setRailPref);
  const [query, setQuery] = useState('');
  const q = useDeferredValue(query.trim());
  useEffect(() => onOpenPalette(() => spotlight.open()), []);

  const results = useQuery({
    queryKey: ['catalog', 'palette', q],
    queryFn: () => invoke('search', { query: q, limit: 6 }),
    enabled: q.length > 0,
    placeholderData: (prev) => prev,
    staleTime: 30_000,
  });

  const actions = useMemo<(SpotlightActionData | SpotlightActionGroupData)[]>(() => {
    const go = (to: string) => () => void navigate(to);
    const match = (label: string) => !q || label.toLowerCase().includes(q.toLowerCase());
    const mk = (id: string, label: string, onClick: () => void, icon: Parameters<typeof Icon>[0]['icon'], description?: string): SpotlightActionData => ({ id, label, description, onClick, leftSection: ic(icon) });

    const channelActions = (q ? results.data?.channels ?? [] : []).map((c) =>
      mk(`c${c.id}`, `${c.number ?? ''} ${c.name}`.trim(), () => void playChannel(c.id).then((err) => (err ? undefined : navigate('/player'))), IconPlayerPlay),
    );
    const progActions = (q ? results.data?.programmes ?? [] : []).map(({ programme: p, channel }) =>
      mk(`p${p.id}`, p.title ?? '', () => activateProgramme(p, channel, navigate), IconCalendarTime, `${channel.name} · ${dayLabel(p.startUtc)} ${formatClock(p.startUtc)}`),
    );

    const goActions = [
      mk('go-home', t('nav.home'), go('/'), IconHome),
      mk('go-channels', t('nav.channels'), go('/channels'), IconListDetails),
      mk('go-guide', t('nav.guide'), go('/guide'), IconCalendarTime),
      ...(caps.vod ? [mk('go-movies', t('nav.movies'), go('/movies'), IconMovie), mk('go-series', t('nav.series'), go('/series'), IconDeviceTv)] : []),
      ...(caps.dvr ? [mk('go-rec', t('nav.recordings'), go('/recordings'), IconCircle)] : []),
    ].filter((a) => match(a.label ?? ''));

    const actionItems = [
      mk('a-sync', t('palette.sync'), () => sources.data?.[0] && void invoke('sync_source', { sourceId: sources.data[0].id }), IconRefresh),
      mk('a-tv', t('palette.toggleTv'), () => setDensity(density === 'tv' ? 'comfortable' : 'tv'), IconArrowsMaximize),
      mk('a-rail', t('palette.toggleRail'), () => setRailPref(railPref === 'collapsed' ? 'expanded' : 'collapsed'), IconLayoutSidebar),
    ].filter((a) => match(a.label ?? ''));

    const settingsItems = [
      mk('s-sources', t('settings.sources'), go('/settings/sources'), IconPlugConnected),
      mk('s-playback', t('settings.playback'), go('/settings/playback'), IconPlayerPlay),
      mk('s-appearance', t('settings.appearance'), go('/settings/appearance'), IconPalette),
      mk('s-updates', t('settings.updates'), go('/settings/updates'), IconRefresh),
      mk('s-about', t('settings.about'), go('/settings/about'), IconStethoscope),
    ].filter((a) => match(a.label ?? ''));

    const seeAll = q ? [mk('see-all', t('palette.seeAll', { query: q }), () => void navigate('/search'), IconSearch)] : [];
    const groups: SpotlightActionGroupData[] = [
      { group: t('palette.group.channels'), actions: channelActions },
      { group: t('palette.group.programmes'), actions: progActions },
      { group: t('palette.group.go'), actions: goActions },
      { group: t('palette.group.actions'), actions: [...seeAll, ...actionItems] },
      { group: t('palette.group.settings'), actions: settingsItems },
    ];
    return groups.filter((g) => g.actions.length > 0);
  }, [q, results.data, caps.vod, caps.dvr, navigate, sources.data, density, setDensity, railPref, setRailPref]);

  return (
    <Spotlight
      actions={actions}
      query={query}
      onQueryChange={setQuery}
      filter={(_, a) => a}
      limit={30}
      highlightQuery
      shortcut={null}
      scrollable
      maxHeight={dl.density.comfortable.channelRow * 8}
      nothingFound={t('palette.nothing')}
      searchProps={{ placeholder: t('palette.placeholder'), 'aria-label': t('palette.label'), leftSection: ic(IconSearch) }}
      onSpotlightClose={() => setQuery('')}
    />
  );
}
