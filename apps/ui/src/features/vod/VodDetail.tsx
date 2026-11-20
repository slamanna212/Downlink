import { useState } from 'react';
import { useNavigate, useParams } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { Select, UnstyledButton } from '@mantine/core';
import { IconArrowLeft } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import type { Episode, Movie, SeriesDetail } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { Poster } from '../../components/Poster/Poster';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { resolveImg } from '../../lib/dlimg';
import { t } from '../../lib/i18n';
import { formatDuration } from '../../lib/time';
import { playVod } from '../player/actions';
import { progressOf, resumeUseful, runtimeLabel } from './vod';
import classes from './Vod.module.css';

function useStart() {
  const navigate = useNavigate();
  const [error, setError] = useState<string | null>(null);
  const start = async (kind: 'movie' | 'episode', id: number, resume: boolean) => {
    setError(null);
    const err = await playVod(kind, id, resume);
    if (err) setError(err.message || t('vod.playFailed'));
    else void navigate('/player');
  };
  return { start, error };
}

function Problem({ message, back }: { message: string; back: () => void }) {
  return (
    <div className={classes.msgBox} role="alert">
      <span>{message}</span>
      <DLButton variant="default" icon={IconArrowLeft} onClick={back}>{t('vod.back')}</DLButton>
    </div>
  );
}

function Hero({ title, poster, backdrop, meta, plot, onBack, children }: { title: string; poster: string | null; backdrop: string | null; meta: (string | null | undefined | false)[]; plot: string | null; onBack: () => void; children: React.ReactNode }) {
  const parts = meta.filter(Boolean) as string[];
  return (
    <div className={classes.hero}>
      {backdrop && <div className={classes.heroBackdrop} style={{ backgroundImage: `url(${resolveImg(backdrop) ?? ''})` }} aria-hidden />}
      <div className={classes.heroPoster}>
        <Poster src={poster} title={title} />
      </div>
      <div className={classes.heroText}>
        <DLButton className={classes.backLink} variant="subtle" size="sm" icon={IconArrowLeft} onClick={onBack}>{t('vod.back')}</DLButton>
        <h1 className={classes.dTitle}>{title}</h1>
        <div className={classes.meta}>
          {parts.map((p, i) => (
            <span key={p} className={i ? classes.dot : undefined}>{p}</span>
          ))}
        </div>
        {plot && <p className={classes.plot}>{plot}</p>}
        {children}
      </div>
    </div>
  );
}

export function MovieDetail() {
  const { id } = useParams();
  const navigate = useNavigate();
  const q = useQuery({ queryKey: ['catalog', 'movie', id], queryFn: () => invoke('get_movie', { id: Number(id) }) });
  const { start, error } = useStart();
  const back = () => void navigate('/movies');
  if (q.isLoading) return <SkeletonList rows={5} label={t('loading.generic')} />;
  if (q.isError || !q.data) return <Problem message={t('vod.notFound')} back={back} />;
  const m: Movie = q.data;
  const resume = resumeUseful(m.resumeSecs, m.durationSecs);
  return (
    <div className={classes.detail}>
      <Hero title={m.title} poster={m.poster} backdrop={m.backdrop} onBack={back} plot={m.plot}
        meta={[m.year ? String(m.year) : null, runtimeLabel(m.durationSecs), m.rating != null ? `${m.rating.toFixed(1)} / 10` : null, m.genres.join(', ')]}>
        <div className={classes.actions}>
          <DLButton variant="filled" size="lg" onClick={() => void start('movie', m.id, resume)}>
            {resume ? t('vod.resume', { time: formatDuration(m.resumeSecs ?? 0) }) : t('vod.play')}
          </DLButton>
          {resume && <DLButton variant="default" size="lg" onClick={() => void start('movie', m.id, false)}>{t('vod.startOver')}</DLButton>}
        </div>
        {error && <p className={classes.msg} role="alert">{error}</p>}
      </Hero>
    </div>
  );
}

function EpisodeRow({ ep, season, next, onPlay }: { ep: Episode; season: number; next: boolean; onPlay: () => void }) {
  const prog = progressOf(ep.resumeSecs, ep.durationSecs);
  return (
    <UnstyledButton className={classes.epRow} data-next={next || undefined} onClick={onPlay}>
      <span className={classes.epNum}>{ep.number}</span>
      <span className={classes.epStill}>
        <Poster src={ep.still} title={`S${season} E${ep.number}`} shape="wide" />
      </span>
      <span className={classes.epMain}>
        <span className={classes.epTitle}>{ep.title}</span>
        <span className={classes.epSub}>{runtimeLabel(ep.durationSecs)}</span>
      </span>
      <span className={classes.epState}>
        {next ? t('vod.next') : ep.watched ? t('vod.watched') : prog != null ? formatDuration(ep.resumeSecs ?? 0) : ''}
      </span>
    </UnstyledButton>
  );
}

export function SeriesDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const q = useQuery({ queryKey: ['catalog', 'seriesDetail', id], queryFn: () => invoke('get_series', { id: Number(id) }) });
  const { start, error } = useStart();
  const data: SeriesDetail | undefined = q.data;
  let nextInfo: { season: number; ep: Episode } | null = null;
  for (const sn of data?.seasons ?? []) for (const e of sn.episodes) if (e.id === data?.nextEpisodeId) nextInfo = { season: sn.number, ep: e };
  const [seasonNo, setSeasonNo] = useState<number | null>(null);
  const back = () => void navigate('/series');
  if (q.isLoading) return <SkeletonList rows={5} label={t('loading.generic')} />;
  if (q.isError || !data) return <Problem message={t('vod.notFound')} back={back} />;
  const s = data.series;
  const season = data.seasons.find((x) => x.number === (seasonNo ?? nextInfo?.season)) ?? data.seasons[0];
  const resume = nextInfo && resumeUseful(nextInfo.ep.resumeSecs, nextInfo.ep.durationSecs);
  const label = (n: { season: number; ep: Episode }) =>
    resume ? t('vod.resumeEpisode', { s: n.season, e: n.ep.number, time: formatDuration(n.ep.resumeSecs ?? 0) }) : t('vod.playEpisode', { s: n.season, e: n.ep.number });
  return (
    <div className={classes.detail}>
      <Hero title={s.title} poster={s.poster} backdrop={s.backdrop} onBack={back} plot={s.plot}
        meta={[s.year ? String(s.year) : null, s.seasonCount === 1 ? t('vod.season1') : t('vod.seasons', { count: s.seasonCount }), s.rating != null ? `${s.rating.toFixed(1)} / 10` : null, s.genres.join(', ')]}>
        <div className={classes.actions}>
          {nextInfo && (
            <DLButton variant="filled" size="lg" onClick={() => void start('episode', nextInfo.ep.id, !!resume)}>{label(nextInfo)}</DLButton>
          )}
          {nextInfo && resume && <DLButton variant="default" size="lg" onClick={() => void start('episode', nextInfo.ep.id, false)}>{t('vod.startOver')}</DLButton>}
        </div>
        {error && <p className={classes.msg} role="alert">{error}</p>}
      </Hero>
      <div className={classes.body}>
        <div className={classes.seasonTabs}>
          <Select
            size="sm"
            w={180}
            aria-label={t('vod.season', { n: season?.number ?? 1 })}
            allowDeselect={false}
            value={String(season?.number ?? '')}
            data={data.seasons.map((x) => ({ value: String(x.number), label: t('vod.season', { n: x.number }) }))}
            onChange={(v) => v && setSeasonNo(Number(v))}
          />
        </div>
        <h2 className={classes.h2}>{t('vod.episodes')}</h2>
        {season?.episodes.map((ep) => (
          <EpisodeRow key={ep.id} ep={ep} season={season.number} next={ep.id === data.nextEpisodeId} onPlay={() => void start('episode', ep.id, resumeUseful(ep.resumeSecs, ep.durationSecs))} />
        ))}
      </div>
    </div>
  );
}
