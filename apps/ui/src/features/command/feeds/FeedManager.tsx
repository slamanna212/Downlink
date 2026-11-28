import { useState } from 'react';
import { Modal, TextInput } from '@mantine/core';
import { IconKey, IconPlus, IconRefresh, IconTrash, IconX } from '@tabler/icons-react';
import { toAppError } from '../../../api/ipc';
import { useAddFeed, useFeeds, useRefreshFeed, useRemoveFeed } from '../../../api/queries';
import type { Feed } from '../../../api/types';
import { DLButton } from '../../../components/DLButton/DLButton';
import { DLIconButton } from '../../../components/DLIconButton/DLIconButton';
import { Icon } from '../../../components/Icon/Icon';
import { t } from '../../../lib/i18n';
import { feedName } from './Headlines';
import classes from './Feeds.module.css';

export interface FeedManagerProps {
  opened: boolean;
  onClose: () => void;
  /** Feeds on this command center, in order. */
  attached: number[];
  /** Saves the center's new feed list. */
  onChange: (ids: number[]) => void;
  saving?: boolean;
}

/**
 * Manage a center's news feeds: add one by address (it joins the feed library and this center),
 * attach a feed already in the library, detach, refresh, or delete a feed everywhere.
 */
export function FeedManager({ opened, onClose, attached, onChange, saving }: FeedManagerProps) {
  return (
    <Modal opened={opened} onClose={onClose} title={t('feeds.manage.title')} size="lg" withinPortal>
      {opened && <ManagerBody attached={attached} onChange={onChange} saving={saving} />}
    </Modal>
  );
}

function ManagerBody({ attached, onChange, saving }: Pick<FeedManagerProps, 'attached' | 'onChange' | 'saving'>) {
  const feeds = useFeeds();
  const add = useAddFeed();
  const refresh = useRefreshFeed();
  const removeFeed = useRemoveFeed();
  const [url, setUrl] = useState('');
  const [error, setError] = useState<string | null>(null);
  const all = feeds.data ?? [];
  const byId = new Map(all.map((f) => [f.id, f]));
  const mine = attached.flatMap((id) => (byId.has(id) ? [byId.get(id) as Feed] : []));
  const others = all.filter((f) => !attached.includes(f.id));

  const submit = () => {
    const value = url.trim();
    if (!value) return;
    setError(null);
    add.mutate(value, {
      onSuccess: (feed) => {
        setUrl('');
        if (!attached.includes(feed.id)) onChange([...attached, feed.id]);
      },
      onError: (e) => setError(toAppError(e).message),
    });
  };

  return (
    <div className={classes.manager}>
      <p className={classes.help}>{t('feeds.manage.help')}</p>
      <form
        className={classes.addRow}
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <TextInput
          data-autofocus
          className={classes.addInput}
          label={t('feeds.add.label')}
          placeholder={t('feeds.add.placeholder')}
          type="url"
          value={url}
          error={error}
          onChange={(e) => {
            setUrl(e.currentTarget.value);
            setError(null);
          }}
        />
        <DLButton type="submit" variant="filled" icon={IconPlus} loading={add.isPending} disabled={!url.trim()}>
          {t('feeds.add')}
        </DLButton>
      </form>

      <h3 className={classes.subhead}>{t('feeds.attached')}</h3>
      {mine.length === 0 ? (
        <p className={classes.muted}>{t('feeds.attached.none')}</p>
      ) : (
        <ul className={classes.feedList}>
          {mine.map((f) => (
            <FeedLine key={f.id} feed={f}>
              <DLIconButton
                label={t('feeds.refresh', { title: feedName(f) })}
                icon={IconRefresh}
                size="sm"
                loading={refresh.isPending && refresh.variables === f.id}
                onClick={() => refresh.mutate(f.id)}
              />
              <DLIconButton
                label={t('feeds.detach', { title: feedName(f) })}
                icon={IconX}
                size="sm"
                disabled={saving}
                onClick={() => onChange(attached.filter((x) => x !== f.id))}
              />
            </FeedLine>
          ))}
        </ul>
      )}

      {others.length > 0 && (
        <>
          <h3 className={classes.subhead}>{t('feeds.library')}</h3>
          <ul className={classes.feedList}>
            {others.map((f) => (
              <FeedLine key={f.id} feed={f}>
                <DLIconButton label={t('feeds.attach', { title: feedName(f) })} icon={IconPlus} size="sm" disabled={saving} onClick={() => onChange([...attached, f.id])} />
                <DLIconButton
                  label={t('feeds.delete', { title: feedName(f) })}
                  icon={IconTrash}
                  variant="danger"
                  size="sm"
                  loading={removeFeed.isPending && removeFeed.variables === f.id}
                  onClick={() => removeFeed.mutate(f.id)}
                />
              </FeedLine>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function FeedLine({ feed: f, children }: { feed: Feed; children: React.ReactNode }) {
  return (
    <li className={classes.feedLine}>
      <span className={classes.feedText}>
        <span className={classes.feedName}>
          {feedName(f)}
          {f.hasSecret && (
            <span className={classes.secret} title={t('feeds.secret')} aria-label={t('feeds.secret')}>
              <Icon icon={IconKey} size={14} />
            </span>
          )}
        </span>
        <span className={classes.feedUrl}>{f.url}</span>
        {f.lastError ? (
          <span className={classes.feedError}>{t('feeds.lastError', { error: f.lastError })}</span>
        ) : (
          <span className={classes.feedUrl}>{t('feeds.items', { n: f.itemCount })}</span>
        )}
      </span>
      <span className={classes.feedActions}>{children}</span>
    </li>
  );
}
