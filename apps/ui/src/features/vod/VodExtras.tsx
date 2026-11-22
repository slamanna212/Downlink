import { useQuery } from '@tanstack/react-query';
import { IconBrandYoutube } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { DLButton } from '../../components/DLButton/DLButton';
import { notifyError } from '../../components/Notify/notify';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { t } from '../../lib/i18n';
import { Attribution, CastStrip, CrewList, Facts } from '../info/InfoView';
import info from '../info/Info.module.css';
import classes from './Vod.module.css';

const open = (url: string) => void invoke('open_external_link', { url }).catch(notifyError);

/**
 * Cast, crew, facts and the trailer for a movie or series: the source's own data first, TMDB for the
 * gaps. Loads after the page itself so the hero never waits on it; renders nothing when empty.
 */
export function VodExtras({ id }: { id: number }) {
  const q = useQuery({
    queryKey: ['details', { kind: 'vod', id }],
    queryFn: () => invoke('media_details', { target: { kind: 'vod', id } }),
    staleTime: 10 * 60_000,
    retry: false,
  });
  const d = q.data;
  if (!d) return null;
  return (
    <section className={classes.extras} aria-label={t('details.facts')}>
      {(d.certification || d.trailerUrl) && (
        <div className={classes.extrasTop}>
          {d.certification && <StatusBadge kind="cert" label={d.certification} />}
          {d.trailerUrl && (
            <DLButton variant="default" size="sm" icon={IconBrandYoutube} onClick={() => open(d.trailerUrl!)}>
              {t('details.trailer')}
            </DLButton>
          )}
        </div>
      )}
      <div className={info.columns}>
        <div className={info.prose}>
          {d.tagline && <p className={info.tagline}>{d.tagline}</p>}
          <CrewList crew={d.crew} />
          <CastStrip cast={d.cast} />
        </div>
        <aside className={info.aside}>
          <Facts d={d} />
        </aside>
      </div>
      <Attribution d={d} onOpenLink={open} />
    </section>
  );
}
