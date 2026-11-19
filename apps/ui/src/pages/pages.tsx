import { Text } from '@mantine/core';
import { t } from '../lib/i18n';
import { PageHeader } from './PageHeader';
import { Section } from './Section';

export const HomePage = () => <Section title="page.home" />;
export const GuidePage = () => <Section title="page.guide" />;
export const MoviesPage = () => <Section title="page.movies" />;
export const SeriesPage = () => <Section title="page.series" />;
export const RecordingsPage = () => <Section title="page.recordings" />;
export const SettingsPage = () => <Section title="page.settings" />;
export const SearchPage = () => <Section title="page.search" />;

export function AddSourcePage() {
  return (
    <>
      <PageHeader title={t('page.addSource')} />
      <Text px="xl" c="dimmed">
        {t('page.placeholder')}
      </Text>
    </>
  );
}

export function NotFoundPage() {
  return <PageHeader title={t('page.notFound')} />;
}
