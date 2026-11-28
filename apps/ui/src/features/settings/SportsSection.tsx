import { useState } from 'react';
import { Modal, SegmentedControl, Switch, TextInput } from '@mantine/core';
import { IconArrowDown, IconArrowUp, IconPlus, IconSearch, IconTrash } from '@tabler/icons-react';
import { useSetSportsFollow, useSportsCatalog, useSportsFollow } from '../../api/queries';
import type { SportsCatalog, SportsFollow, SportsLeague, SportsSession } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { Icon } from '../../components/Icon/Icon';
import { SettingRow } from '../../components/SettingRow/SettingRow';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { t, type MessageKey } from '../../lib/i18n';
import {
  allGames,
  catalogSports,
  GROUP_ORDER,
  groupByWhen,
  type MatchWhen,
  moveRank,
  pickerBlocks,
  rankEntries,
  type RankEntry,
  removeSession,
  removeTeam,
  setIncludeReplays,
  setLeagueFollowed,
  setSeriesFollowed,
  SESSION_ORDER,
  SPORT_ORDER,
  sortMatches,
  toggleSession,
  toggleTeam,
} from '../sports/follow';
import { FollowChip, PickerTile } from '../sports/FollowChip';
import { SportsMatchCard } from '../sports/SportsMatchCard';
import { TeamChip } from '../sports/TeamChip';
import { useFollowedMatches } from '../sports/useFollowedMatches';
import { useSelectChannel, useSelectMatch } from '../sports/useSelectMatch';
import { dayLabel } from '../../lib/time';
import sports from '../sports/Sports.module.css';
import { Section } from './Section';

type Update = (next: SportsFollow) => void;

const sessionLabel = (s: SportsSession) => t(`sports.session.${s}`);

export const groupLabel = (group: string): string =>
  (GROUP_ORDER as readonly string[]).includes(group) ? t(`sports.group.${group}` as MessageKey) : t('sports.group.other');

export const sportLabel = (sport: string): string => t(`sports.sport.${SPORT_ORDER.includes(sport) ? sport : 'other'}` as MessageKey);

function teamSummary(teams: string[], league: SportsLeague): string {
  const known = teams.filter((k) => league.teams.some((x) => x.key === k)).length;
  if (known === 0) return t('sports.league.allGames');
  return known === 1 ? t('sports.league.oneTeam') : t('sports.league.teams', { n: known });
}

/** Leagues and series the follow names that the catalog still knows, in catalog order. */
const followedLeagues = (catalog: SportsCatalog, f: SportsFollow): SportsLeague[] =>
  catalog.leagues.filter((l) => (l.kind === 'motorsport' ? !!f.series[l.key] : !!f.leagues[l.key]));

/** One followed league or series: its name, removable chips (teams or sessions), Edit and Unfollow. */
function FollowRow({ league, follow, update, onEdit }: { league: SportsLeague; follow: SportsFollow; update: Update; onEdit: () => void }) {
  const series = league.kind === 'motorsport';
  const teamKeys = follow.leagues[league.key]?.teams ?? [];
  const teams = league.teams.filter((x) => teamKeys.includes(x.key));
  const sessions = SESSION_ORDER.filter((s) => follow.series[league.key]?.sessions.includes(s));
  const meta = series ? sportLabel('motorsport') : t('sports.following.meta', { sport: sportLabel(league.sport), detail: teamSummary(teamKeys, league) });
  const unfollow = () => update(series ? setSeriesFollowed(follow, league.key, false) : setLeagueFollowed(follow, league.key, false));
  return (
    <div className={sports.followRow}>
      <div className={sports.followName}>
        <div className={sports.followTitle}>{league.name}</div>
        <div className={sports.followMeta}>{meta}</div>
      </div>
      <div className={sports.followChips}>
        {series ? (
          sessions.length ? (
            sessions.map((s) => <FollowChip key={s} label={sessionLabel(s)} onRemove={() => update(removeSession(follow, league.key, s))} />)
          ) : (
            <span className={sports.followMeta}>{t('sports.series.noSessions')}</span>
          )
        ) : teams.length ? (
          teams.map((x) => <FollowChip key={x.key} abbr={x.abbr} label={x.short} onRemove={() => update(removeTeam(follow, league.key, x.key))} />)
        ) : (
          <FollowChip label={t('sports.league.allGames')} onRemove={unfollow} />
        )}
      </div>
      <div className={sports.followActions}>
        <DLButton size="xs" variant="subtle" aria-label={t('sports.following.editAria', { name: league.name })} onClick={onEdit}>
          {t('sports.following.edit')}
        </DLButton>
        <DLIconButton size="sm" variant="subtle" icon={IconTrash} label={t('sports.following.unfollow', { name: league.name })} onClick={unfollow} />
      </div>
    </div>
  );
}

/**
 * "Add teams and series": every league and series by sport, searchable. Teams and sessions are checkbox
 * tiles; a league's "All games" follows every game, and picking a team while it is on narrows to that team.
 */
function AddTeamsModal({ opened, initialQuery, catalog, follow, update, onClose }: {
  opened: boolean;
  initialQuery: string;
  catalog: SportsCatalog;
  follow: SportsFollow;
  update: Update;
  onClose: () => void;
}) {
  return (
    <Modal
      opened={opened}
      onClose={onClose}
      title={t('sports.picker.title')}
      size={760}
      withinPortal
      classNames={{ content: sports.pickerContent, body: sports.pickerBody }}
    >
      {/* Keyed so each opening starts from its own search. */}
      {opened && <Picker key={initialQuery} initialQuery={initialQuery} catalog={catalog} follow={follow} update={update} onClose={onClose} />}
    </Modal>
  );
}

function Picker({ initialQuery, catalog, follow, update, onClose }: { initialQuery: string; catalog: SportsCatalog; follow: SportsFollow; update: Update; onClose: () => void }) {
  const [query, setQuery] = useState(initialQuery);
  const [tab, setTab] = useState('all');
  const sessions = SESSION_ORDER.filter((s) => catalog.sessions.includes(s));
  const blocks = pickerBlocks(catalog.leagues, tab === 'all' ? null : tab, query);
  const n = followedLeagues(catalog, follow).length;
  return (
    <>
      <div className={sports.pickerTop}>
        <TextInput
          data-autofocus
          aria-label={t('sports.picker.search')}
          placeholder={t('sports.picker.search')}
          leftSection={<Icon icon={IconSearch} size={16} />}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
        />
        <SegmentedControl
          className={sports.pickerTabs}
          aria-label={t('sports.picker.tabs')}
          value={tab}
          onChange={setTab}
          data={[{ value: 'all', label: t('sports.picker.all') }, ...catalogSports(catalog.leagues).map((s) => ({ value: s, label: sportLabel(s) }))]}
        />
      </div>
      <div className={sports.pickerScroll}>
        {blocks.map(({ league, teams }) => {
          if (league.kind === 'motorsport') {
            const picked = follow.series[league.key];
            return (
              <div key={league.key} role="group" aria-label={league.name}>
                <div className={sports.blockHead}>
                  <span className={sports.blockName}>{league.name}</span>
                  <span className={sports.blockSport}>{groupLabel(league.group ?? 'other')}</span>
                  <TeamChip
                    label={picked ? t('sports.picker.following') : t('sports.picker.follow')}
                    pressed={!!picked}
                    onToggle={() => update(setSeriesFollowed(follow, league.key, !picked))}
                  />
                </div>
                <div className={sports.tiles}>
                  {sessions.map((s) => (
                    <PickerTile key={s} label={sessionLabel(s)} selected={!!picked?.sessions.includes(s)} onToggle={() => update(picked?.sessions.includes(s) ? removeSession(follow, league.key, s) : toggleSession(follow, league.key, s))} />
                  ))}
                </div>
              </div>
            );
          }
          const picked = follow.leagues[league.key];
          const all = !!picked && picked.teams.length === 0;
          return (
            <div key={league.key} role="group" aria-label={league.name}>
              <div className={sports.blockHead}>
                <span className={sports.blockName}>{league.name}</span>
                <span className={sports.blockSport}>{sportLabel(league.sport)}</span>
                <TeamChip
                  label={t('sports.picker.allGames')}
                  pressed={all}
                  onToggle={() => update(all ? setLeagueFollowed(follow, league.key, false) : allGames(follow, league.key))}
                />
              </div>
              <div className={sports.tiles}>
                {teams.map((x) => (
                  <PickerTile
                    key={x.key}
                    abbr={x.abbr}
                    label={x.name}
                    selected={all || !!picked?.teams.includes(x.key)}
                    onToggle={() => update(!all && picked?.teams.includes(x.key) ? removeTeam(follow, league.key, x.key) : toggleTeam(follow, league.key, x.key))}
                  />
                ))}
              </div>
            </div>
          );
        })}
        {blocks.length === 0 && <p className={sports.muted}>{t('sports.picker.none')}</p>}
      </div>
      <div className={sports.pickerFoot}>
        <span className={sports.pickerSummary}>{n === 1 ? t('sports.picker.summaryOne') : t('sports.picker.summary', { n })}</span>
        <DLButton variant="filled" onClick={onClose}>{t('sports.picker.done')}</DLButton>
      </div>
    </>
  );
}

const rankLabel = (e: RankEntry): string =>
  e.team ? `${e.team.short} · ${e.league.name}` : e.league.kind === 'team' ? t('sports.rank.allGames', { league: e.league.name }) : e.league.name;

/**
 * Command center order (COMMAND_CENTER §3): when more games are live than there are tiles, the
 * higher entry gets the tile. Shown once two or more things are followed.
 */
function RankList({ catalog, follow, update }: { catalog: SportsCatalog; follow: SportsFollow; update: Update }) {
  const entries = rankEntries(follow, catalog);
  if (entries.length < 2) return null;
  return (
    <>
      <h3 className={sports.subhead}>{t('sports.rank')}</h3>
      <p className={sports.previewHelp}>{t('sports.rank.help')}</p>
      <ol className={sports.rankList} aria-label={t('sports.rank')}>
        {entries.map((e, i) => {
          const name = rankLabel(e);
          return (
            <li key={e.key} className={sports.rankItem}>
              <span className={sports.rankNumber}>{i + 1}</span>
              <span className={sports.rankName}>{name}</span>
              <DLIconButton label={t('sports.rank.up', { name })} icon={IconArrowUp} size="sm" disabled={i === 0} onClick={() => update(moveRank(follow, catalog, e.key, -1))} />
              <DLIconButton label={t('sports.rank.down', { name })} icon={IconArrowDown} size="sm" disabled={i === entries.length - 1} onClick={() => update(moveRank(follow, catalog, e.key, 1))} />
            </li>
          );
        })}
      </ol>
    </>
  );
}

const PREVIEW_HOURS = 24;

/** A preview group's heading: On now, Later today, Tomorrow, or the day for anything later. */
const whenLabel = (when: MatchWhen, start: number | null, now: number): string =>
  when === 'later' ? dayLabel(start ?? now, now) : t(`sports.when.${when}`);
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
          <div data-testid="sports-preview">
            {groupByWhen(shown, now).map(({ when, matches: group }) => (
              <div key={when} className={sports.whenGroup} role="group" aria-label={whenLabel(when, group[0]?.programme?.startUtc ?? null, now)}>
                <div className={sports.whenLabel}>
                  <span className={sports.whenName}>{whenLabel(when, group[0]?.programme?.startUtc ?? null, now)}</span>
                  <span className={sports.whenCount}>{group.length === 1 ? t('sports.when.countOne') : t('sports.when.count', { n: group.length })}</span>
                </div>
                <div className={sports.grid}>
                  {group.map((m) => (
                    <SportsMatchCard key={`${m.channel.id}:${m.league}:${m.programme?.id ?? ''}`} match={m} catalog={catalog.data} now={now} onSelect={select} onSelectChannel={selectChannel} />
                  ))}
                </div>
              </div>
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

/** What is followed, with the Add teams dialog. Only followed things are listed; the rest live in the dialog. */
function Following({ catalog, follow, update }: { catalog: SportsCatalog; follow: SportsFollow; update: Update }) {
  // null = closed; otherwise the search the dialog opens with ("" from Add teams, the league name from Edit).
  const [picker, setPicker] = useState<string | null>(null);
  const followed = followedLeagues(catalog, follow);
  return (
    <>
      <div className={sports.followHead}>
        <h3 className={sports.subhead}>{t('sports.following')}</h3>
        <span className={sports.followCount}>{followed.length}</span>
        <span className={sports.followSpacer} />
        <DLButton variant="filled" icon={IconPlus} onClick={() => setPicker('')}>{t('sports.following.add')}</DLButton>
      </div>
      <div className={sports.followList}>
        {followed.map((l) => <FollowRow key={l.key} league={l} follow={follow} update={update} onEdit={() => setPicker(l.name)} />)}
        {followed.length === 0 && <p className={sports.followEmpty}>{t('sports.following.empty')}</p>}
      </div>
      <AddTeamsModal opened={picker !== null} initialQuery={picker ?? ''} catalog={catalog} follow={follow} update={update} onClose={() => setPicker(null)} />
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
  return (
    <Section title={t('settings.sports')} help={t('settings.sports.help')}>
      <SettingRow label={t('sports.replays')} help={t('sports.replays.help')} wide>
        <Switch
          aria-label={t('sports.replays')}
          checked={f.includeReplays}
          onChange={(e) => update(setIncludeReplays(f, e.currentTarget.checked))}
        />
      </SettingRow>

      <Following catalog={catalog.data} follow={f} update={update} />

      <RankList catalog={catalog.data} follow={f} update={update} />

      <Preview />
    </Section>
  );
}
