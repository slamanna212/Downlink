import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { UnstyledButton } from '@mantine/core';
import { IconChevronLeft, IconChevronRight, IconPlayerPlay, IconSearch } from '@tabler/icons-react';
import { useCaps, useHome } from '../../api/queries';
import type { Channel, ContinueItem, OnNowItem } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { EmptySource } from '../../pages/EmptySource';
import { ProgrammeArt } from '../../components/ProgrammeArt/ProgrammeArt';
import { formatClock, minutesLeft, progress } from '../../lib/time';
import { useDLFocusable } from '../../lib/spatial';
import { useNowEpoch } from '../../lib/useNow';
import { modKeyLabel } from '../../lib/platform';
import { useInteractive } from '../../lib/perf';
import { t } from '../../lib/i18n';
import { openPalette } from '../../lib/palette';
import { playCatchup, playChannel, playRecording, playVod, restartProgramme } from '../player/actions';
import classes from './Home.module.css';
import { SportsRow } from './SportsRow';

function Focusable({ children, className, onPlay, label }: { children: React.ReactNode; className: string | undefined; onPlay: () => void; label?: string }) {
  // Native focus follows norigin (lib/spatial), which also scrolls a paged-out row card into view.
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: onPlay });
  return (
    <UnstyledButton ref={ref} className={className} onClick={onPlay} aria-label={label} {...focusProps}>
      {children}
    </UnstyledButton>
  );
}

function useActions() {
  const navigate = useNavigate();
  const go = (p: Promise<unknown>) => void p.then((err) => (err ? undefined : navigate('/player')));
  return {
    live: (ch: Channel) => go(playChannel(ch.id)),
    open: (it: ContinueItem) => {
      if (it.kind === 'live' && it.channel) go(playChannel(it.channel.id));
      else if (it.kind === 'catchup') {
        // id is `catchup:<channelId>:<startUtc>`; the programme id is `<channelId>:<startUtc>`. Resume at the saved position.
        const pid = it.programme?.id ?? it.id.split(':').slice(1).join(':');
        if (pid.includes(':')) go(playCatchup(pid, true));
        else if (it.channel) go(restartProgramme(it.channel.id));
      }
      else if (it.kind === 'movie' || it.kind === 'episode') go(playVod(it.kind, Number(it.id.split(':')[1]), true));
      else if (it.kind === 'recording') go(playRecording(Number(it.id.split(':')[1])));
    },
  };
}

/** Art for a continue item: the programme's art for live/catch-up (their `art` is the channel logo). */
function itemArt(item: ContinueItem): { programmeId: string | null; src: string | null } {
  if (item.kind === 'live' || item.kind === 'catchup') return { programmeId: item.programme?.id ?? null, src: null };
  return { programmeId: null, src: item.art };
}

function SearchButton() {
  return (
    <DLButton variant="default" justify="space-between" className={classes.searchBtn} icon={IconSearch} onClick={openPalette}>
      <span className={classes.searchLabel}>{t('home.search')}</span> <span className={classes.kbd}>{modKeyLabel()}</span>
    </DLButton>
  );
}

function ContinueCard({ item }: { item: ContinueItem }) {
  const a = useActions();
  const tag = item.kind === 'movie' ? t('home.badge.movie') : item.kind === 'episode' ? t('home.badge.episode') : item.kind === 'recording' ? t('home.badge.recording') : null;
  return (
    <Focusable className={classes.card} onPlay={() => a.open(item)} label={t('home.continue.play', { title: item.title })}>
      <ProgrammeArt {...itemArt(item)} channelId={item.channel?.id ?? item.id} className={classes.art}>
        <span className={classes.artTag}>
          {item.kind === 'live' ? <StatusBadge kind="live" /> : item.kind === 'catchup' ? <StatusBadge kind="catchup" /> : tag && <span className={classes.tag}>{tag}</span>}
        </span>
        {item.channel && (item.kind === 'live' || item.kind === 'catchup') && (
          <span className={classes.artLogo}>
            <ChannelLogo channelId={item.channel.id} name={item.channel.name} src={item.channel.logo} density="compact" />
          </span>
        )}
        <span className={classes.playOverlay} aria-hidden>
          <span className={classes.playDisc}><Icon icon={IconPlayerPlay} size={24} filled /></span>
        </span>
        {item.progress != null && (
          <span className={classes.track}>
            <span className={classes.fill} style={{ width: `${item.progress * 100}%` }} />
          </span>
        )}
      </ProgrammeArt>
      <div className={classes.cardTitle}>{item.title}</div>
      {item.subtitle && <div className={classes.cardSub}>{item.subtitle}</div>}
    </Focusable>
  );
}

/** How many continue items the row holds (the old hero plus its eight-card grid). */
const CONTINUE_MAX = 9;

/** "Continue watching": an even row, four across, paged with the arrows (or by scrolling). */
function ContinueRow({ items }: { items: ContinueItem[] }) {
  const row = useRef<HTMLDivElement>(null);
  const [edges, setEdges] = useState({ start: true, end: true });
  const measure = useCallback(() => {
    const el = row.current;
    if (!el) return;
    setEdges({ start: el.scrollLeft <= 1, end: el.scrollLeft + el.clientWidth >= el.scrollWidth - 1 });
  }, []);
  useEffect(() => {
    measure();
    const el = row.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure, items.length]);
  const page = (dir: -1 | 1) => row.current?.scrollBy({ left: dir * row.current.clientWidth, behavior: 'smooth' });
  return (
    <section className={classes.section} aria-labelledby="home-continue">
      <div className={classes.sectionHead}>
        <h3 className={classes.h3} id="home-continue">{t('home.continue')}</h3>
        <span className={classes.count}>{items.length}</span>
        <span className={classes.spacer} />
        {!(edges.start && edges.end) && (
          <span className={classes.pager}>
            <DLIconButton label={t('home.continue.prev')} icon={IconChevronLeft} size="sm" disabled={edges.start} onClick={() => page(-1)} tabIndex={-1} />
            <DLIconButton label={t('home.continue.next')} icon={IconChevronRight} size="sm" disabled={edges.end} onClick={() => page(1)} tabIndex={-1} />
          </span>
        )}
      </div>
      <div ref={row} className={classes.row} onScroll={measure}>
        {items.map((it) => <ContinueCard key={it.id} item={it} />)}
      </div>
    </section>
  );
}

function OnNowCard({ item, now }: { item: OnNowItem; now: number }) {
  const a = useActions();
  const p = item.now;
  const next = item.next;
  return (
    <Focusable className={classes.nowCard} onPlay={() => a.live(item.channel)}>
      <ProgrammeArt programmeId={p?.id} channelId={item.channel.id} className={classes.nowArt}>
        <span className={classes.nowLogo}>
          <ChannelLogo channelId={item.channel.id} name={item.channel.name} src={item.channel.logo} density="compact" />
        </span>
        {p && (
          <span className={classes.nowBadge}>
            <StatusBadge kind="live" />
          </span>
        )}
      </ProgrammeArt>
      <span className={classes.nowBody}>
        <span className={classes.nowChannel}>{item.channel.number} · {item.channel.name}</span>
        <span className={classes.nowTitle}>{p?.title ?? t('channels.noGuide')}</span>
        {p && (
          <span className={classes.nowFoot}>
            <span className={classes.nowTrack}>
              <span className={classes.fill} style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
            </span>
            <span>{t('home.minLeft', { min: minutesLeft(p.endUtc, now) })}</span>
          </span>
        )}
        {next && <span className={classes.nowNext}>{t('home.next', { time: formatClock(next.startUtc), title: next.title ?? '' })}</span>}
      </span>
    </Focusable>
  );
}

export function HomePage() {
  const navigate = useNavigate();
  const { loaded, hasSource } = useCaps();
  const home = useHome();
  useInteractive(!!home.data);
  const now = useNowEpoch(30_000);
  const a = useActions();
  if (loaded && !hasSource) return <EmptySource />;
  const d = home.data;
  const continueItems = (d?.continueWatching ?? []).slice(0, CONTINUE_MAX);
  return (
    <div className={classes.page}>
      <div className={classes.top}>
        <h1 className={classes.title}>{t('home.title')}</h1>
        <SearchButton />
      </div>
      <div className={classes.body}>
        {home.isError && !d ? (
          <ErrorState onRetry={() => void home.refetch()} />
        ) : !d ? (
          <SkeletonList rows={4} label={t('loading.generic')} />
        ) : (
          <>
            {continueItems.length > 0 ? <ContinueRow items={continueItems} /> : <p className={classes.muted}>{t('home.empty')}</p>}

            <SportsRow />

            <section className={classes.section}>
              <div className={classes.sectionHead}>
                <h3 className={classes.h3}>{t('home.onNow')}</h3>
                <span className={classes.spacer} />
                <button type="button" className={classes.link} onClick={() => void navigate('/channels')}>{t('home.editFavourites')}</button>
              </div>
              {d.onNow.length ? (
                <div className={classes.grid}>{d.onNow.map((it) => <OnNowCard key={it.channel.id} item={it} now={now} />)}</div>
              ) : (
                <p className={classes.muted}>{t('home.noFavourites')}</p>
              )}
            </section>

            {d.recents.length > 0 && (
              <section className={classes.section}>
                <div className={classes.sectionHead}><h3 className={classes.h3}>{t('home.recents')}</h3></div>
                <div className={classes.recents}>
                  {d.recents.map((c) => (
                    <Focusable key={c.id} className={classes.recent} onPlay={() => a.live(c)}>
                      <span className={classes.recentPlate}>
                        <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="tv" />
                      </span>
                      <span className={classes.recentName}><span className={classes.dim}>{c.number}</span> {c.name}</span>
                    </Focusable>
                  ))}
                </div>
              </section>
            )}
          </>
        )}
      </div>
    </div>
  );
}
