import { useNavigate } from 'react-router';
import { t } from '../../lib/i18n';
import { hoursToSeconds } from '../../lib/time';
import { homeSportsItems } from '../sports/follow';
import { SportsMatchCard } from '../sports/SportsMatchCard';
import { useFollowedMatches } from '../sports/useFollowedMatches';
import { useSelectChannel, useSelectMatch } from '../sports/useSelectMatch';
import sports from '../sports/Sports.module.css';
import classes from './Home.module.css';

const HORIZON_HOURS = 48;

/**
 * "Your sports" (SPORTS §6 phase 2): followed matches on now, then the next 48 hours, each card labeled
 * with the team or series it is for. Hidden entirely when nothing is followed or nothing matches.
 */
export function SportsRow() {
  const navigate = useNavigate();
  const { now, catalog, following, matches } = useFollowedMatches(HORIZON_HOURS);
  const select = useSelectMatch();
  const selectChannel = useSelectChannel();
  if (!following || !matches.data) return null;
  const items = homeSportsItems(matches.data, now, hoursToSeconds(HORIZON_HOURS));
  if (items.length === 0) return null;
  return (
    <section className={classes.section} aria-labelledby="home-sports">
      <div className={classes.sectionHead}>
        <h3 className={classes.h3} id="home-sports">{t('home.sports')}</h3>
        <span className={classes.count}>{items.length}</span>
        <span className={classes.spacer} />
        <button type="button" className={classes.link} onClick={() => void navigate('/multiview/sports')}>{t('command.sports.open')}</button>
        <button type="button" className={classes.link} onClick={() => void navigate('/settings/sports')}>{t('home.editSports')}</button>
      </div>
      <div className={`${sports.grid} ${sports.homeGrid}`} data-testid="home-sports">
        {items.map((m) => (
          <SportsMatchCard key={`${m.channel.id}:${m.programme?.id ?? m.league}`} match={m} catalog={catalog.data} now={now} onSelect={select} onSelectChannel={selectChannel} />
        ))}
      </div>
    </section>
  );
}
