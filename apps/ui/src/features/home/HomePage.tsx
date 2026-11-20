import { useNavigate } from 'react-router';
import { UnstyledButton } from '@mantine/core';
import { IconSearch } from '@tabler/icons-react';
import { useCaps, useHome } from '../../api/queries';
import type { Channel, ContinueItem, OnNowItem } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { ErrorState } from '../../components/ErrorState/ErrorState';
import { DLButton } from '../../components/DLButton/DLButton';
import { SkeletonList } from '../../components/Skeleton/SkeletonRow';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { EmptySource } from '../../pages/EmptySource';
import { formatClock, minutesLeft, progress } from '../../lib/time';
import { useDLFocusable } from '../../lib/spatial';
import { useNowEpoch } from '../../lib/useNow';
import { modKeyLabel } from '../../lib/platform';
import { t } from '../../lib/i18n';
import { openPalette } from '../palette/CommandPalette';
import { playCatchup, playChannel, playRecording, playVod, restartProgramme } from '../player/actions';
import classes from './Home.module.css';

function Focusable({ children, className, onPlay }: { children: React.ReactNode; className: string | undefined; onPlay: () => void }) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: onPlay });
  return (
    <UnstyledButton ref={ref} className={className} onClick={onPlay} {...focusProps}>
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

function Hero({ item, now }: { item: ContinueItem; now: number }) {
  const a = useActions();
  const p = item.programme;
  return (
    <div className={classes.hero}>
      <div className={classes.heroMeta}>
        {item.kind === 'live' ? <StatusBadge kind="live" /> : <StatusBadge kind="catchup" />}
        {item.channel && <span>{item.channel.number} · {item.channel.name}</span>}
      </div>
      <h3 className={classes.heroTitle}>{item.title}</h3>
      {p && (
        <div className={classes.heroTimeline}>
          <span>{formatClock(p.startUtc)}</span>
          <span className={classes.nowTrack}>
            <span className={classes.fill} style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
          </span>
          <span>{formatClock(p.endUtc)}</span>
        </div>
      )}
      <div className={classes.heroActions}>
        <DLButton variant="filled" size="lg" onClick={() => a.open(item)}>{item.kind === 'live' ? t('guide.watchLive') : t('vod.play')}</DLButton>
        {item.channel && (item.kind === 'live' || item.kind === 'catchup') && (
          <DLButton variant="default" size="lg" onClick={() => void restartProgramme(item.channel!.id)}>
            {t('guide.fromStart')}
          </DLButton>
        )}
      </div>
    </div>
  );
}

function ContinueCard({ item }: { item: ContinueItem }) {
  const a = useActions();
  const tag = item.kind === 'movie' ? t('home.badge.movie') : item.kind === 'episode' ? t('home.badge.episode') : item.kind === 'recording' ? t('home.badge.recording') : null;
  return (
    <Focusable className={classes.card} onPlay={() => a.open(item)}>
      <div className={classes.art}>
        <span className={classes.artTag}>{item.kind === 'catchup' ? <StatusBadge kind="catchup" /> : tag && <span className={classes.kbd}>{tag}</span>}</span>
        {item.progress != null && (
          <span className={classes.track}>
            <span className={classes.fill} style={{ width: `${item.progress * 100}%` }} />
          </span>
        )}
      </div>
      <div className={classes.cardTitle}>{item.title}</div>
      {item.subtitle && <div className={classes.cardSub}>{item.subtitle}</div>}
    </Focusable>
  );
}

function OnNowCard({ item, now }: { item: OnNowItem; now: number }) {
  const a = useActions();
  const p = item.now;
  return (
    <Focusable className={classes.nowCard} onPlay={() => a.live(item.channel)}>
      <span className={classes.nowHead}>
        <ChannelLogo channelId={item.channel.id} name={item.channel.name} src={item.channel.logo} density="compact" />
        <span>{item.channel.number} · {item.channel.name}</span>
      </span>
      <span className={classes.nowTitle}>{p?.title ?? t('channels.noGuide')}</span>
      {p && (
        <span className={classes.nowFoot}>
          <span className={classes.nowTrack}>
            <span className={classes.fill} style={{ width: `${progress(p.startUtc, p.endUtc, now) * 100}%` }} />
          </span>
          <span>{t('home.minLeft', { min: minutesLeft(p.endUtc, now) })}</span>
        </span>
      )}
    </Focusable>
  );
}

export function HomePage() {
  const navigate = useNavigate();
  const { loaded, hasSource } = useCaps();
  const home = useHome();
  const now = useNowEpoch(30_000);
  const a = useActions();
  if (loaded && !hasSource) return <EmptySource />;
  const d = home.data;
  const [first, ...rest] = d?.continueWatching ?? [];
  return (
    <div className={classes.page}>
      <div className={classes.top}>
        <h1 className={classes.title}>{t('home.title')}</h1>
        <DLButton variant="default" justify="space-between" className={classes.searchBtn} icon={IconSearch} onClick={openPalette}>
          <span className={classes.searchLabel}>{t('home.search')}</span> <span className={classes.kbd}>{modKeyLabel()}</span>
        </DLButton>
      </div>
      {home.isError && !d ? (
        <ErrorState onRetry={() => void home.refetch()} />
      ) : !d ? (
        <SkeletonList rows={4} label={t('loading.generic')} />
      ) : (
        <>
          <div className={classes.sectionHead}><h2 className={classes.h2}>{t('home.continue')}</h2></div>
          {first ? (
            <div className={classes.continue}>
              <Hero item={first} now={now} />
              {rest.slice(0, 2).map((it) => <ContinueCard key={it.id} item={it} />)}
            </div>
          ) : (
            <p className={classes.muted}>{t('home.empty')}</p>
          )}

          <div className={classes.sectionHead}>
            <h2 className={classes.h2}>{t('home.onNow')}</h2>
            <button type="button" className={classes.link} onClick={() => void navigate('/channels')}>{t('home.editFavourites')}</button>
          </div>
          {d.onNow.length ? (
            <div className={classes.onNow}>{d.onNow.map((it) => <OnNowCard key={it.channel.id} item={it} now={now} />)}</div>
          ) : (
            <p className={classes.muted}>{t('home.noFavourites')}</p>
          )}

          <div className={classes.sectionHead}><h2 className={classes.h2}>{t('home.recents')}</h2></div>
          <div className={classes.recents}>
            {d.recents.map((c) => (
              <Focusable key={c.id} className={classes.chip} onPlay={() => a.live(c)}>
                <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" />
                <span><span className={classes.chipNum}>{c.number}</span> {c.name}</span>
              </Focusable>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
