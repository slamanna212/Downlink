import { t } from '../lib/i18n';
import { PageHeader } from './PageHeader';


export function NotFoundPage() {
  return <PageHeader title={t('page.notFound')} />;
}
