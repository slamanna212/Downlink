import { useNavigate } from 'react-router';
import { Stack, Text } from '@mantine/core';
import { IconPlugConnected } from '@tabler/icons-react';
import { Icon } from '../components/Icon/Icon';
import { DLButton } from '../components/DLButton/DLButton';
import { t } from '../lib/i18n';
import classes from './EmptySource.module.css';

/** The "Add a source" empty state (PLAN §8.7): one sentence, one primary action. */
export function EmptySource() {
  const navigate = useNavigate();
  return (
    <Stack align="center" justify="center" gap="md" className={classes.root}>
      <div className={classes.mark}>
        <Icon icon={IconPlugConnected} size={24} />
      </div>
      <Text className={classes.title} component="h2" m={0}>
        {t('empty.title')}
      </Text>
      <Text className={classes.body} size="md">
        {t('empty.body')}
      </Text>
      <DLButton variant="filled" size="lg" onClick={() => void navigate('/sources/add')}>
        {t('empty.action')}
      </DLButton>
      <Text size="sm" c="dimmed">
        {t('empty.hint')}
      </Text>
    </Stack>
  );
}
