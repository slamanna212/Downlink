import { Text } from '@mantine/core';
import { useCaps, useChannels } from '../api/queries';
import { SkeletonList } from '../components/Skeleton/SkeletonRow';
import { t, type MessageKey } from '../lib/i18n';
import { EmptySource } from './EmptySource';
import { PageHeader } from './PageHeader';

/** Placeholder for a section that is built in a later task: header + empty state when no source. */
export function Section({ title, showCount = false }: { title: MessageKey; showCount?: boolean }) {
  const { loaded, hasSource } = useCaps();
  const channels = useChannels();
  return (
    <>
      <PageHeader title={t(title)} />
      {!loaded ? (
        <SkeletonList rows={6} label={t('loading.generic')} />
      ) : !hasSource ? (
        <EmptySource />
      ) : (
        <Text px="xl" c="dimmed">
          {showCount && channels.data ? `${t('page.channelCount', { count: channels.data.length })}. ` : ''}
          {t('page.placeholder')}
        </Text>
      )}
    </>
  );
}
