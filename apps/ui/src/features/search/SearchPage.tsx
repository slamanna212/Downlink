import { useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { TextInput } from '@mantine/core';
import { IconSearch } from '@tabler/icons-react';
import { invoke } from '../../api/ipc';
import { useCaps } from '../../api/queries';
import type { Channel, ProgrammeHit } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { Icon } from '../../components/Icon/Icon';
import { StatusBadge } from '../../components/StatusBadge/StatusBadge';
import { EmptySource } from '../../pages/EmptySource';
import { nextIndex } from '../../lib/listNav';
import { dayLabel, formatRange, nowEpoch } from '../../lib/time';
import { t } from '../../lib/i18n';
import { useDetailStore } from '../../store/detail';
import { activateProgramme } from '../guide/activate';
import { blockState } from '../guide/guideModel';
import { playChannel } from '../player/actions';
import classes from './Search.module.css';

const LIMIT = 20;

type Item = { kind: 'channel'; channel: Channel } | { kind: 'programme'; hit: ProgrammeHit };

export function SearchPage() {
  const navigate = useNavigate();
  const showDetail = useDetailStore((s) => s.show);
  const { loaded, hasSource } = useCaps();
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query.trim());
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement | null>(null);

  const results = useQuery({
    queryKey: ['catalog', 'search', deferred],
    queryFn: () => invoke('search', { query: deferred, limit: LIMIT }),
    enabled: deferred.length > 0,
    placeholderData: (prev) => prev,
    staleTime: 30_000,
  });
  const items: Item[] = useMemo(
    () => [
      ...(results.data?.channels ?? []).map((channel): Item => ({ kind: 'channel', channel })),
      ...(results.data?.programmes ?? []).map((hit): Item => ({ kind: 'programme', hit })),
    ],
    [results.data],
  );

  // Keep the active row in view (index-based, no layout reads).
  useEffect(() => {
    listRef.current?.querySelector<HTMLElement>('[data-active]')?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const activate = (it: Item | undefined) => {
    if (!it) return;
    if (it.kind === 'channel') void playChannel(it.channel.id).then(() => navigate('/player'));
    else activateProgramme(it.hit.programme, it.hit.channel, navigate, showDetail);
  };

  if (loaded && !hasSource) return <EmptySource />;
  const channelCount = results.data?.channels.length ?? 0;
  const now = nowEpoch();

  return (
    <div className={classes.page}>
      <h1 className={classes.title}>{t('search.title')}</h1>
      <TextInput
        size="lg"
        autoFocus
        value={query}
        aria-label={t('search.title')}
        placeholder={t('search.placeholder')}
        leftSection={<Icon icon={IconSearch} size={20} />}
        role="combobox"
        aria-expanded={items.length > 0}
        aria-controls="search-results"
        aria-activedescendant={items.length ? `sr-${active}` : undefined}
        onChange={(e) => {
          setQuery(e.currentTarget.value);
          setActive(0);
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault();
            e.stopPropagation();
            setActive((a) => nextIndex(a, e.key as 'ArrowUp' | 'ArrowDown', items.length));
          } else if (e.key === 'Enter') {
            e.preventDefault();
            activate(items[active]);
          } else if (e.key === 'Escape') {
            if (query) setQuery('');
            else void navigate(-1);
          }
        }}
      />
      <div id="search-results" ref={listRef} role={items.length ? 'listbox' : undefined} aria-label={t('search.results')}>
        {deferred.length === 0 && <p className={classes.empty}>{t('search.hint')}</p>}
        {deferred.length > 0 && results.data && items.length === 0 && <p className={classes.empty}>{t('search.empty', { query: deferred })}</p>}
        {channelCount > 0 && (
          <div role="group" aria-label={t('search.channels')} className={classes.group}>
            <div className={classes.groupLabel} aria-hidden>{t('search.channels')}</div>
            {items.slice(0, channelCount).map((it, i) =>
              it.kind === 'channel' ? (
                <div key={`c${it.channel.id}`} id={`sr-${i}`} role="option" aria-selected={i === active} data-active={i === active || undefined} className={classes.item} onClick={() => activate(it)} onMouseMove={() => i !== active && setActive(i)}>
                  <ChannelLogo channelId={it.channel.id} name={it.channel.name} src={it.channel.logo} density="compact" />
                  <span className={classes.num}>{it.channel.number}</span>
                  <span className={classes.main}>
                    <span className={classes.name}>{it.channel.name}</span>
                  </span>
                </div>
              ) : null,
            )}
          </div>
        )}
        {items.length > channelCount && (
          <div role="group" aria-label={t('search.programmes')} className={classes.group}>
            <div className={classes.groupLabel} aria-hidden>{t('search.programmes')}</div>
            {items.slice(channelCount).map((it, k) => {
              if (it.kind !== 'programme') return null;
              const i = channelCount + k;
              const p = it.hit.programme;
              const st = blockState(p, it.hit.channel, now);
              return (
                <div key={p.id} id={`sr-${i}`} role="option" aria-selected={i === active} data-active={i === active || undefined} className={classes.item} onClick={() => activate(it)} onMouseMove={() => i !== active && setActive(i)}>
                  <ChannelLogo channelId={it.hit.channel.id} name={it.hit.channel.name} src={it.hit.channel.logo} density="compact" />
                  <span className={classes.main}>
                    <span className={classes.name}>{p.title}</span>
                    <span className={classes.sub}>
                      {it.hit.channel.number} · {it.hit.channel.name} · {dayLabel(p.startUtc, now)} {formatRange(p.startUtc, p.endUtc)}
                    </span>
                  </span>
                  {st === 'now' && <StatusBadge kind="live" />}
                  {st === 'past-archive' && <StatusBadge kind="catchup" />}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
