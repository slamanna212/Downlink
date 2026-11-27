import { useState } from 'react';
import { Checkbox, Switch, TextInput } from '@mantine/core';
import { IconSearch } from '@tabler/icons-react';
import { useSetSportsFollow, useSportsCatalog, useSportsFollow } from '../../api/queries';
import type { SportsFollow, SportsLeague, SportsSession } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { Icon } from '../../components/Icon/Icon';
import { SettingRow } from '../../components/SettingRow/SettingRow';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { t, type MessageKey } from '../../lib/i18n';
import {
  allGames,
  filterTeams,
  GROUP_ORDER,
  motorsportGroups,
  setIncludeReplays,
  setLeagueFollowed,
  setSeriesFollowed,
  SESSION_ORDER,
  sortMatches,
  toggleSession,
  toggleTeam,
} from '../sports/follow';
import { SportsMatchCard } from '../sports/SportsMatchCard';
import { TeamChip } from '../sports/TeamChip';
import { useFollowedMatches } from '../sports/useFollowedMatches';
import { useSelectChannel, useSelectMatch } from '../sports/useSelectMatch';
import sports from '../sports/Sports.module.css';
import { Section } from './Section';

type Update = (next: SportsFollow) => void;

const sessionLabel = (s: SportsSession) => t(`sports.session.${s}`);

export const groupLabel = (group: string): string =>
  (GROUP_ORDER as readonly string[]).includes(group) ? t(`sports.group.${group}` as MessageKey) : t('sports.group.other');

function teamSummary(teams: string[], league: SportsLeague): string {
  const known = teams.filter((k) => league.teams.some((x) => x.key === k)).length;
  if (known === 0) return t('sports.league.allGames');
  return known === 1 ? t('sports.league.oneTeam') : t('sports.league.teams', { n: known });
}

/** Searchable chip grid of a followed league's teams, led by "All games" (on while no team is picked). */
function TeamPicker({ league, teams, follow, update }: { league: SportsLeague; teams: string[]; follow: SportsFollow; update: Update }) {
  const [query, setQuery] = useState('');
  const shown = filterTeams(league.teams, query);
  return (
    <div className={sports.picker} role="group" aria-label={t('sports.teams.label', { league: league.name })}>
      {league.teams.length > 8 && (
        <TextInput
          className={sports.search}
          aria-label={t('sports.teams.search')}
          placeholder={t('sports.teams.search')}
          leftSection={<Icon icon={IconSearch} size={16} />}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
        />
      )}
      <div className={sports.chips}>
        <TeamChip label={t('sports.league.allGames')} pressed={teams.length === 0} onToggle={() => update(allGames(follow, league.key))} />
        {shown.map((team) => (
          <TeamChip
            key={team.key}
            abbr={team.abbr}
            label={team.short}
            pressed={teams.includes(team.key)}
            onToggle={() => update(toggleTeam(follow, league.key, team.key))}
          />
        ))}
      </div>
      {shown.length === 0 && <p className={sports.muted}>{t('sports.teams.none', { query: query.trim() })}</p>}
    </div>
  );
}

function LeagueRow({ league, follow, update }: { league: SportsLeague; follow: SportsFollow; update: Update }) {
  const picked = follow.leagues[league.key];
  return (
    <>
      <SettingRow label={league.name} help={picked ? teamSummary(picked.teams, league) : t('sports.league.off')} wide>
        <Switch
          aria-label={t('sports.league.toggle', { league: league.name })}
          checked={!!picked}
          onChange={(e) => update(setLeagueFollowed(follow, league.key, e.currentTarget.checked))}
        />
      </SettingRow>
      {/* Unfollowed leagues render no chips, so 40 leagues and ~175 teams stay cheap. */}
      {picked && <TeamPicker league={league} teams={picked.teams} follow={follow} update={update} />}
    </>
  );
}

function SeriesRow({ series, sessions, follow, update }: { series: SportsLeague; sessions: SportsSession[]; follow: SportsFollow; update: Update }) {
  const picked = follow.series[series.key];
  const help = !picked
    ? t('sports.league.off')
    : picked.sessions.length
      ? SESSION_ORDER.filter((s) => picked.sessions.includes(s)).map(sessionLabel).join(', ')
      : t('sports.series.noSessions');
  return (
    <>
      <SettingRow label={series.name} help={help} wide>
        <Switch
          aria-label={t('sports.series.toggle', { series: series.name })}
          checked={!!picked}
          onChange={(e) => update(setSeriesFollowed(follow, series.key, e.currentTarget.checked))}
        />
      </SettingRow>
      {picked && (
        <div className={sports.sessions} role="group" aria-label={t('sports.series.sessions', { series: series.name })}>
          {sessions.map((s) => (
            <Checkbox
              key={s}
              label={sessionLabel(s)}
              checked={picked.sessions.includes(s)}
              onChange={() => update(toggleSession(follow, series.key, s))}
            />
          ))}
        </div>
      )}
    </>
  );
}

const PREVIEW_HOURS = 24;
const PREVIEW_FIRST = 24;

/** "In your guide now": the next 24 h of matches, for tuning the picks above (SPORTS §3). */
function Preview() {
  const { now, catalog, following, matches } = useFollowedMatches(PREVIEW_HOURS);
  const select = useSelectMatch();
  const selectChannel = useSelectChannel();
  const [all, setAll] = useState(false);
  const list = sortMatches((matches.data ?? []).filter((m) => !m.programme || m.programme.endUtc > now), now);
  const shown = all ? list : list.slice(0, PREVIEW_FIRST);
  return (
    <>
      <h3 className={sports.subhead}>{t('sports.preview')}</h3>
      <p className={sports.previewHelp}>{t('sports.preview.help')}</p>
      {!following ? (
        <p className={sports.muted}>{t('sports.preview.nothingFollowed')}</p>
      ) : matches.isError && !matches.data ? (
        <ErrorState onRetry={() => void matches.refetch()} />
      ) : !matches.data ? (
        <SkeletonList rows={2} label={t('loading.generic')} />
      ) : list.length === 0 ? (
        <p className={sports.muted}>{t('sports.preview.empty')}</p>
      ) : (
        <>
          <div className={sports.grid} data-testid="sports-preview">
            {shown.map((m) => (
              <SportsMatchCard key={`${m.channel.id}:${m.league}:${m.programme?.id ?? ''}`} match={m} catalog={catalog.data} now={now} onSelect={select} onSelectChannel={selectChannel} />
            ))}
          </div>
          {shown.length < list.length && (
            <DLButton variant="subtle" onClick={() => setAll(true)}>{t('sports.preview.showAll', { n: list.length })}</DLButton>
          )}
        </>
      )}
    </>
  );
}

export function SportsSection() {
  const catalog = useSportsCatalog();
  const follow = useSportsFollow();
  const set = useSetSportsFollow();
  if ((catalog.isError && !catalog.data) || (follow.isError && !follow.data)) {
    return <ErrorState onRetry={() => void (catalog.isError ? catalog.refetch() : follow.refetch())} />;
  }
  if (!catalog.data || !follow.data) return <SkeletonList rows={4} label={t('loading.generic')} />;
  const f = follow.data;
  const update: Update = (next) => set.mutate(next);
  const teamLeagues = catalog.data.leagues.filter((l) => l.kind === 'team');
  const sessions = SESSION_ORDER.filter((s) => catalog.data.sessions.includes(s));
  return (
    <Section title={t('settings.sports')} help={t('settings.sports.help')}>
      <SettingRow label={t('sports.replays')} help={t('sports.replays.help')} wide>
        <Switch
          aria-label={t('sports.replays')}
          checked={f.includeReplays}
          onChange={(e) => update(setIncludeReplays(f, e.currentTarget.checked))}
        />
      </SettingRow>

      <h3 className={sports.subhead}>{t('sports.leagues')}</h3>
      {teamLeagues.map((l) => <LeagueRow key={l.key} league={l} follow={f} update={update} />)}

      <h3 className={sports.subhead}>{t('sports.motorsport')}</h3>
      {motorsportGroups(catalog.data.leagues).map(({ group, series }) => (
        <div key={group} role="group" aria-label={groupLabel(group)}>
          <h4 className={sports.groupLabel}>{groupLabel(group)}</h4>
          {series.map((s) => <SeriesRow key={s.key} series={s} sessions={sessions} follow={f} update={update} />)}
        </div>
      ))}

      <Preview />
    </Section>
  );
}
