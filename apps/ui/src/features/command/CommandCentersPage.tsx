import { useNavigate } from 'react-router';
import { useQueries } from '@tanstack/react-query';
import { IconTrophy } from '@tabler/icons-react';
import { channelQuery, useCommandCenters } from '../../api/queries';
import type { Channel, CommandCenterSummary } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { t } from '../../lib/i18n';
import { onAirNow } from '../sports/follow';
import { useFollowedMatches } from '../sports/useFollowedMatches';
import { CommandCenterCard, CommandCenterCardSkeleton } from './CommandCenterCard';
import { CenterMenu } from './CenterMenu';
import classes from './Command.module.css';

/**
 * Command center page (`/command`, a regular page with the nav rail): My sports first, then one card
 * per saved command center. Opening a card plays it; nothing streams from this page (rule 4).
 */
export function CommandCentersPage() {
  const centers = useCommandCenters();
  const list = centers.data ?? [];
  return (
    <div className={classes.page}>
      <header className={classes.pageHead}>
        <h1 className={classes.h1}>{t('command.page.title')}</h1>
        <p className={classes.pageSub}>{t('command.page.help')}</p>
      </header>
      <div className={classes.cards} data-testid="command-cards">
        <SportsCard />
        {centers.isLoading
          ? <CommandCenterCardSkeleton />
          : list.map((c) => <SavedCard key={c.id} center={c} />)}
      </div>
      {!centers.isLoading && list.length === 0 && <EmptySaved />}
    </div>
  );
}

/** The built-in My sports center: always first, even when nothing is followed. */
function SportsCard() {
  const navigate = useNavigate();
  const { now, following, matches } = useFollowedMatches(1);
  const live = new Set((matches.data ?? []).filter((m) => m.live && onAirNow(m, now)).map((m) => m.channel.id)).size;
  const meta = !following
    ? t('command.sports.card.choose')
    : live === 0
      ? t('command.sports.card.none')
      : live === 1 ? t('command.sports.card.live.one') : t('command.sports.card.live', { n: live });
  return (
    <CommandCenterCard
      name={t('command.sports.title')}
      description={t('command.sports.card.desc')}
      meta={meta}
      icon={IconTrophy}
      label={following ? t('command.card.open', { name: t('command.sports.title') }) : t('command.sports.card.chooseAria')}
      onSelect={() => void navigate(following ? '/multiview/sports' : '/settings/sports')}
    />
  );
}

function SavedCard({ center }: { center: CommandCenterSummary }) {
  const navigate = useNavigate();
  const channels = useQueries({ queries: center.channelIds.map(channelQuery) })
    .flatMap(({ data }) => (data ? [data] : [])) satisfies Channel[];
  return (
    <CommandCenterCard
      name={center.name}
      meta={center.tileCount === 1 ? t('command.card.tiles.one') : t('command.card.tiles', { n: center.tileCount })}
      channels={channels}
      label={t('command.card.open', { name: center.name })}
      onSelect={() => void navigate(`/multiview/c/${center.id}`)}
      corner={<CenterMenu center={center} onDeleted={() => undefined} />}
    />
  );
}

function EmptySaved() {
  const navigate = useNavigate();
  return (
    <>
      <h2 className={classes.label}>{t('command.page.saved')}</h2>
      <div className={classes.pageEmpty}>
        <span className={classes.pageEmptyTitle}>{t('command.page.empty.title')}</span>
        <span>{t('command.page.empty.body')}</span>
        <DLButton size="sm" variant="default" onClick={() => void navigate('/multiview')}>{t('command.page.openMultiview')}</DLButton>
      </div>
    </>
  );
}
