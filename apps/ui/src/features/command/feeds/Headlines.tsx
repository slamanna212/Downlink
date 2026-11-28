import { UnstyledButton } from '@mantine/core';
import { IconRss, IconSettings } from '@tabler/icons-react';
import { invoke } from '../../../api/ipc';
import { useFeedItems, useFeeds } from '../../../api/queries';
import type { Feed, FeedItem } from '../../../api/types';
import { DLButton } from '../../../components/DLButton/DLButton';
import { DLIconButton } from '../../../components/DLIconButton/DLIconButton';
import { notifyError } from '../../../components/Notify/notify';
import { t, type MessageKey } from '../../../lib/i18n';
import { useDLFocusable } from '../../../lib/spatial';
import { ageOf } from '../../../lib/time';
import classes from './Feeds.module.css';

/** Headlines shown in the rail; the ticker shows fewer. */
export const RAIL_HEADLINES = 30;
export const TICKER_HEADLINES = 12;

/** "4 min ago" through `time.ts` (the only date math in the UI). */
export const ageLabel = (epoch: number, now: number): string => {
  const a = ageOf(epoch, now);
  return t(`feeds.age.${a.unit}` as MessageKey, { n: a.n });
};

export const feedName = (f: Feed | undefined): string => f?.title || (f ? safeHost(f.url) : '') || t('feeds.untitled');

function safeHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

/** Opens a headline in the system browser (the backend allows http/https only). */
export const openHeadline = (item: FeedItem) => {
  if (item.link) void invoke('open_external_link', { url: item.link }).catch(notifyError);
};

export interface HeadlineRowProps {
  item: FeedItem;
  source: string;
  now: number;
  onOpen: (item: FeedItem) => void;
  disabled?: boolean;
  /** Storybook: force a pseudo state. */
  'data-demo-state'?: string;
  'data-focused'?: string;
}

/** One headline: title (two lines at most), then source · age. Opens the story in the browser. */
export function HeadlineRow({ item, source, now, onOpen, disabled, ...demo }: HeadlineRowProps) {
  const off = disabled || !item.link;
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({ onEnterPress: () => !off && onOpen(item), focusable: !off });
  return (
    <UnstyledButton
      ref={ref}
      className={classes.headline}
      disabled={off}
      onClick={() => onOpen(item)}
      aria-label={t('feeds.open', { title: item.title })}
      title={item.summary ?? undefined}
      {...focusProps}
      {...demo}
    >
      <span className={classes.headlineTitle}>{item.title}</span>
      <span className={classes.headlineMeta}>
        {source} · {ageLabel(item.publishedUtc, now)}
      </span>
    </UnstyledButton>
  );
}

/** Loading placeholder in the row's shape. */
export function HeadlineSkeleton() {
  return <span className={`${classes.headline} ${classes.skeleton}`} aria-hidden />;
}

export interface HeadlinesSectionProps {
  feedIds: number[];
  now: number;
  onManage: () => void;
}

/** The rail's "Headlines" section: newest first across the center's feeds. */
export function HeadlinesSection({ feedIds, now, onManage }: HeadlinesSectionProps) {
  const feeds = useFeeds();
  const items = useFeedItems(feedIds, RAIL_HEADLINES);
  const byId = new Map((feeds.data ?? []).map((f) => [f.id, f]));
  const list = items.data ?? [];
  return (
    <section className={classes.section} aria-label={t('feeds.headlines')}>
      <div className={classes.head}>
        <span>{t('feeds.headlines')}</span>
        {feedIds.length > 0 && <span className={classes.count}>{list.length}</span>}
        <span className={classes.spacer} />
        <DLIconButton label={t('feeds.manage')} icon={IconSettings} size="sm" onClick={onManage} />
      </div>
      {feedIds.length === 0 ? (
        <div className={classes.empty}>
          <span>{t('feeds.none')}</span>
          <DLButton size="xs" variant="default" icon={IconRss} onClick={onManage}>{t('feeds.manage')}</DLButton>
        </div>
      ) : items.isError && !items.data ? (
        <div className={classes.empty}>{t('feeds.error')}</div>
      ) : !items.data ? (
        <div className={classes.list}>
          <HeadlineSkeleton />
          <HeadlineSkeleton />
        </div>
      ) : list.length === 0 ? (
        <div className={classes.empty}>{t('feeds.empty')}</div>
      ) : (
        <div className={classes.list}>
          {list.map((item) => (
            <HeadlineRow key={`${item.feedId}:${item.guid}`} item={item} source={feedName(byId.get(item.feedId))} now={now} onOpen={openHeadline} />
          ))}
        </div>
      )}
    </section>
  );
}

/**
 * The headline ticker: one line along the bottom of the wall, like a news crawl. The list is drawn
 * twice so the scroll loops seamlessly; with reduced motion it stands still and shows the newest.
 */
export function HeadlineTicker({ feedIds, height }: { feedIds: number[]; height: number }) {
  const feeds = useFeeds();
  const items = useFeedItems(feedIds, TICKER_HEADLINES);
  const byId = new Map((feeds.data ?? []).map((f) => [f.id, f]));
  const list = items.data ?? [];
  const run = (copy: number) => (
    <span className={classes.tickerRun} aria-hidden={copy > 0 || undefined}>
      {list.map((item) => (
        <button
          key={`${copy}:${item.feedId}:${item.guid}`}
          type="button"
          className={classes.tickerItem}
          tabIndex={copy > 0 ? -1 : undefined}
          disabled={!item.link}
          onClick={() => openHeadline(item)}
        >
          <span className={classes.tickerSource}>{feedName(byId.get(item.feedId))}</span>
          {item.title}
        </button>
      ))}
    </span>
  );
  return (
    <div className={classes.ticker} style={{ height }} role="region" aria-label={t('feeds.ticker')}>
      <span className={classes.tickerLabel}>{t('feeds.headlines')}</span>
      <div className={classes.tickerViewport}>
        {list.length > 0 && (
          // Duration scales with the amount of text so the speed stays even.
          <div className={classes.tickerTrack} style={{ animationDuration: `${Math.max(30, list.length * 9)}s` }}>
            {run(0)}
            {run(1)}
          </div>
        )}
      </div>
    </div>
  );
}
