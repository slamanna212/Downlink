import { useReleaseNotes } from '../../api/queries';
import { Markdown } from '../../components/Markdown/Markdown';
import { t } from '../../lib/i18n';
import classes from './Settings.module.css';

/** "What's new": the GitHub release body (Markdown), or the latest.json notes if it can't be fetched. */
export function ReleaseNotes({ version, fallback }: { version: string; fallback?: string | null }) {
  const notes = useReleaseNotes(version);
  const body = notes.data?.body ?? (notes.isPending ? null : fallback) ?? null;
  if (!body?.trim()) return null;
  return (
    <div className={classes.notes} data-testid="release-notes">
      <h3 className={classes.notesTitle}>{t('settings.updates.whatsNew', { available: version })}</h3>
      <Markdown>{body}</Markdown>
    </div>
  );
}
