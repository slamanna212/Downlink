import type { ReactNode } from 'react';
import { IconBrandYoutube, IconExternalLink, IconX } from '@tabler/icons-react';
import type { Crew, MediaDetails, Person } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { useAfterDelay } from '../../components/Skeleton/SkeletonRow';
import { resolveImg } from '../../lib/dlimg';
import { episodeCode } from '../../lib/episode';
import { t, type MessageKey } from '../../lib/i18n';
import { initialsOf } from '../../lib/logo';
import { dayLabel, formatCalendarDate, formatRange, minutesLeft, progress } from '../../lib/time';
import { languageName } from '../player/PlayerMenus';
import { runtimeLabel } from '../vod/vod';
import classes from './Info.module.css';

/** What is known before `media_details` answers (the guide row or recording). */
export interface InfoBase {
  title: string;
  episodeTitle: string | null;
  season: number | null;
  episode: number | null;
  description: string | null;
  isNew: boolean;
  isLive: boolean;
  isPremiere: boolean;
  isFinale: boolean;
}

export type InfoTimeState = 'now' | 'past-archive' | 'past-none' | 'future';

export interface InfoViewProps {
  base: InfoBase;
  details?: MediaDetails | null;
  loading: boolean;
  error: boolean;
  onRetry: () => void;
  onClose: () => void;
  /** The airing slot, for guide rows and recordings. */
  slot?: { startUtc: number; endUtc: number } | null;
  channel?: { id: number; name: string; logo: string | null } | null;
  timeState?: InfoTimeState | null;
  /** Scheduled to record / recording now. */
  recording?: boolean;
  now: number;
  actions?: ReactNode;
  /** Opens an external page (trailer, TMDB) in the browser. */
  onOpenLink: (url: string) => void;
}

let regionNames: Intl.DisplayNames | null | undefined;
function regionName(code: string): string {
  if (!/^[A-Za-z]{2}$/.test(code)) return code;
  if (regionNames === undefined) {
    try {
      regionNames = new Intl.DisplayNames(['en'], { type: 'region' });
    } catch {
      regionNames = null;
    }
  }
  try {
    return regionNames?.of(code.toUpperCase()) ?? code;
  } catch {
    return code;
  }
}

const CREW_LABEL: Record<Crew['job'], MessageKey> = {
  creator: 'details.crew.creator',
  director: 'details.crew.director',
  writer: 'details.crew.writer',
  producer: 'details.crew.producer',
  presenter: 'details.crew.presenter',
};

function quality(q: string | null): 'hd' | '4k' | null {
  if (!q) return null;
  const v = q.toUpperCase();
  if (v.includes('UHD') || v.includes('4K')) return '4k';
  return v.includes('HD') ? 'hd' : null;
}

function Portrait({ p }: { p: Person }) {
  const src = resolveImg(p.photo);
  return (
    <li className={classes.person}>
      <div className={classes.portrait}>
        {src ? <img src={src} alt="" loading="lazy" draggable={false} /> : <span className={classes.initials} aria-hidden>{initialsOf(p.name)}</span>}
      </div>
      <span className={classes.personName}>{p.name}</span>
      {p.role && <span className={classes.personRole}>{p.role}</span>}
      {p.guest && <span className={classes.personGuest}>{t('details.guest')}</span>}
    </li>
  );
}

export function Facts({ d, base }: { d: MediaDetails | null | undefined; base?: Pick<InfoBase, 'season'> }) {
  if (!d) return null;
  const facts: [MessageKey, string][] = [];
  const date = formatCalendarDate(d.airDate);
  if (date) facts.push([d.kind === 'movie' ? 'details.fact.released' : 'details.fact.firstAired', date]);
  else if (d.year) facts.push(['details.fact.year', String(d.year)]);
  const rt = runtimeLabel(d.runtimeMins ? d.runtimeMins * 60 : null);
  if (rt) facts.push(['details.fact.runtime', rt]);
  if (d.genres.length) facts.push(['details.fact.genres', d.genres.join(', ')]);
  if (d.score) facts.push(['details.fact.score', t('details.score', { value: d.score.toFixed(1) })]);
  if (d.network) facts.push(['details.fact.network', d.network]);
  if (d.status && (base?.season != null || d.kind === 'series' || d.kind === 'episode')) facts.push(['details.fact.status', d.status]);
  if (d.seasonCount && d.seasonCount > 1) facts.push(['details.fact.seasons', String(d.seasonCount)]);
  if (d.country) facts.push(['details.fact.country', regionName(d.country)]);
  if (d.language) facts.push(['details.fact.language', languageName(d.language)]);
  const picture = [d.videoQuality, d.aspectRatio].filter(Boolean).join(' ');
  if (picture) facts.push(['details.fact.picture', picture]);
  if (d.audio) facts.push(['details.fact.sound', d.audio.charAt(0).toUpperCase() + d.audio.slice(1)]);
  if (d.advisories.length) facts.push(['details.fact.advisory', d.advisories.join(', ')]);
  if (!facts.length) return null;
  return (
    <dl className={classes.facts} aria-label={t('details.facts')}>
      {facts.map(([k, v]) => (
        <div key={k} className={classes.fact}>
          <dt>{t(k)}</dt>
          <dd>{v}</dd>
        </div>
      ))}
    </dl>
  );
}

export function CastStrip({ cast }: { cast: Person[] }) {
  if (!cast.length) return null;
  return (
    <section className={classes.castSection}>
      <h3 className={classes.h3}>{t('details.cast')}</h3>
      <ul className={classes.cast}>
        {cast.map((p) => (
          <Portrait key={`${p.name}|${p.role ?? ''}`} p={p} />
        ))}
      </ul>
    </section>
  );
}

export function CrewList({ crew }: { crew: Crew[] }) {
  if (!crew.length) return null;
  return (
    <dl className={classes.crew}>
      {crew.map((c) => (
        <div key={c.job} className={classes.crewRow}>
          <dt>{t(CREW_LABEL[c.job])}</dt>
          <dd>{c.names.join(', ')}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Attribution({ d, onOpenLink }: { d: MediaDetails; onOpenLink: (url: string) => void }) {
  if (!d.fromTmdb) return null;
  return (
    <footer className={classes.attribution}>
      <span>{t('details.tmdbAttribution')}</span>
      {d.tmdbUrl && (
        <DLButton variant="subtle" size="xs" icon={IconExternalLink} onClick={() => onOpenLink(d.tmdbUrl!)}>
          {t('details.tmdbLink')}
        </DLButton>
      )}
    </footer>
  );
}

function LoadingLines() {
  const late = useAfterDelay(300);
  return (
    <div className={classes.loading} role="status" aria-live="polite">
      <span className={classes.line} style={{ width: '92%' }} />
      <span className={classes.line} style={{ width: '84%' }} />
      <span className={classes.line} style={{ width: '58%' }} />
      {late && <span className={classes.loadingLabel}>{t('details.loading')}</span>}
    </div>
  );
}

/**
 * The details view: an artwork hero with the episode slate, then badges, actions, the prose and a
 * facts column. Pure: everything comes in as props (the modal wires data and actions).
 */
export function InfoView({ base, details, loading, error, onRetry, onClose, slot, channel, timeState, recording, now, actions, onOpenLink }: InfoViewProps) {
  const d = details ?? null;
  const title = d?.title || base.title;
  const season = d?.season ?? base.season;
  const episode = d?.episode ?? base.episode;
  const episodeTitle = d?.episodeTitle ?? base.episodeTitle;
  const description = d?.description ?? base.description;
  const art = resolveImg(d?.still ?? d?.backdrop ?? null);
  const logo = resolveImg(d?.logo ?? null);
  const code = episodeCode({ season, episode });
  const pct = slot && timeState === 'now' ? progress(slot.startUtc, slot.endUtc, now) : null;
  const q = quality(d?.videoQuality ?? null);
  const isNew = d?.isNew ?? base.isNew;
  const isLive = d?.isLive ?? base.isLive;
  const isPremiere = d?.isPremiere ?? base.isPremiere;
  const isFinale = d?.isFinale ?? base.isFinale;

  const badges: ReactNode[] = [];
  if (timeState === 'now' || isLive) badges.push(<StatusBadge key="live" kind="live" />);
  if (timeState === 'past-archive') badges.push(<StatusBadge key="cu" kind="catchup" />);
  if (recording) badges.push(<StatusBadge key="rec" kind="rec" />);
  if (isPremiere) badges.push(<StatusBadge key="pr" kind="premiere" />);
  if (isFinale) badges.push(<StatusBadge key="fi" kind="finale" />);
  if (isNew) badges.push(<StatusBadge key="new" kind="new" />);
  else if (d?.isRepeat) badges.push(<StatusBadge key="rep" kind="repeat" />);
  if (d?.certification) badges.push(<StatusBadge key="cert" kind="cert" label={d.certification} />);
  if (q) badges.push(<StatusBadge key="q" kind={q} />);

  return (
    <article className={classes.view} data-art={art ? 'image' : 'none'}>
      <header className={classes.hero}>
        {art && <img className={classes.heroArt} src={art} alt="" draggable={false} />}
        {art && <div className={classes.heroScrim} aria-hidden />}
        <div className={classes.close}>
          <DLIconButton label={t('common.close')} icon={IconX} variant={art ? 'overlay' : 'subtle'} onClick={onClose} />
        </div>
        <div className={classes.heroBody}>
          {!art && channel && (
            <div className={classes.heroLogo}>
              <ChannelLogo channelId={channel.id} name={channel.name} src={channel.logo} density="comfortable" />
            </div>
          )}
          <div className={classes.heroText}>
            {logo ? (
              <h2 className={classes.logoTitle}>
                <img src={logo} alt={title} draggable={false} />
              </h2>
            ) : (
              <h2 className={classes.title}>{title}</h2>
            )}
            {(code || episodeTitle) && (
              <div className={classes.slate}>
                {code && (
                  <span className={classes.code} aria-label={`${t('details.slate.season')} ${season ?? ''} ${t('details.slate.episode')} ${episode ?? ''}`}>
                    {code}
                  </span>
                )}
                {episodeTitle && <span className={classes.epTitle}>{episodeTitle}</span>}
              </div>
            )}
            {(slot || channel) && (
              <div className={classes.slot}>
                {channel && <span className={classes.slotChannel}>{channel.name}</span>}
                {slot && <span>{`${dayLabel(slot.startUtc, now)} ${formatRange(slot.startUtc, slot.endUtc)}`}</span>}
                {slot && timeState === 'now' && <span className={classes.slotLeft}>{t('details.minLeft', { min: minutesLeft(slot.endUtc, now) })}</span>}
              </div>
            )}
          </div>
        </div>
        {pct != null && (
          <div className={classes.ribbon} role="progressbar" aria-label={title} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct * 100)}>
            <span style={{ width: `${pct * 100}%` }} />
          </div>
        )}
      </header>

      <div className={classes.body}>
        {badges.length > 0 && <div className={classes.badges}>{badges}</div>}
        {(actions || d?.trailerUrl) && (
          <div className={classes.actions}>
            {actions}
            {d?.trailerUrl && (
              <DLButton variant="default" icon={IconBrandYoutube} onClick={() => onOpenLink(d.trailerUrl!)}>
                {t('details.trailer')}
              </DLButton>
            )}
          </div>
        )}

        <div className={classes.columns}>
          <div className={classes.prose}>
            {d?.tagline && <p className={classes.tagline}>{d.tagline}</p>}
            {description ? <p className={classes.description}>{description}</p> : !loading && <p className={classes.muted}>{t('details.noDescription')}</p>}
            {loading && <LoadingLines />}
            {error && !loading && (
              <div className={classes.error} role="alert">
                <span>{t('details.error')}</span>
                <DLButton variant="subtle" size="sm" onClick={onRetry}>{t('error.retry')}</DLButton>
              </div>
            )}
            {d?.overview && (
              <section className={classes.about}>
                <h3 className={classes.h3}>{t('details.aboutShow')}</h3>
                <p>{d.overview}</p>
              </section>
            )}
            {d && <CrewList crew={d.crew} />}
            {d && <CastStrip cast={d.cast} />}
          </div>
          <aside className={classes.aside}>
            <Facts d={d} base={base} />
          </aside>
        </div>

        {d && <Attribution d={d} onOpenLink={onOpenLink} />}
      </div>
    </article>
  );
}
