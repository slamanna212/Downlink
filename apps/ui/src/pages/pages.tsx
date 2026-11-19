import { useNavigate } from 'react-router';
import { Stack, Text } from '@mantine/core';
import { IconArrowLeft } from '@tabler/icons-react';
import { usePlayerStore } from '../store/player';
import { DLButton } from '../components/DLButton/DLButton';
import { t } from '../lib/i18n';
import { PageHeader } from './PageHeader';
import { Section } from './Section';
import classes from './Player.module.css';

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

/** The video rect is transparent: libmpv renders natively beneath the webview. UI sits on top. */
export function PlayerPage() {
  const navigate = useNavigate();
  const state = usePlayerStore((s) => s.player.state);
  return (
    <div className={classes.root} data-video-rect>
      <Stack className={classes.overlay} gap="sm" align="flex-start" p="xl">
        <DLButton variant="overlay" size="lg" icon={IconArrowLeft} onClick={() => void navigate(-1)}>
          {t('player.back')}
        </DLButton>
        {state === 'Idle' && (
          <Text c="var(--dl-on-video)" fw={500}>
            {t('player.idle')}
          </Text>
        )}
      </Stack>
    </div>
  );
}
