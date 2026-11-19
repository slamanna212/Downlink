import { t } from '../lib/i18n';
import { PageHeader } from './PageHeader';
import { Section } from './Section';

export const MoviesPage = () => <Section title="page.movies" />;
export const SeriesPage = () => <Section title="page.series" />;
export const RecordingsPage = () => <Section title="page.recordings" />;

export function NotFoundPage() {
  return <PageHeader title={t('page.notFound')} />;
}
