import { useState } from 'react';
import { Modal, TextInput } from '@mantine/core';
import { DLButton } from '../../components/DLButton/DLButton';
import { t } from '../../lib/i18n';
import classes from './Command.module.css';

/** Server-side limit on names (trimmed). */
export const NAME_MAX = 80;

export interface CenterNameModalProps {
  opened: boolean;
  title: string;
  /** Name the field starts with. */
  initial: string;
  loading?: boolean;
  onClose: () => void;
  onSubmit: (name: string) => void;
}

/** Asks for a command center name: saving a wall, saving as new, renaming. */
export function CenterNameModal({ opened, title, initial, loading, onClose, onSubmit }: CenterNameModalProps) {
  return (
    <Modal opened={opened} onClose={onClose} title={title} withinPortal>
      {/* Keyed so each opening starts from `initial`. */}
      {opened && <NameForm key={initial} initial={initial} loading={loading} onClose={onClose} onSubmit={onSubmit} />}
    </Modal>
  );
}

function NameForm({ initial, loading, onClose, onSubmit }: Pick<CenterNameModalProps, 'initial' | 'loading' | 'onClose' | 'onSubmit'>) {
  const [name, setName] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        const trimmed = name.trim();
        if (!trimmed) {
          setError(t('command.name.required'));
          return;
        }
        onSubmit(trimmed);
      }}
    >
      <TextInput
        data-autofocus
        label={t('command.name.label')}
        placeholder={t('command.name.placeholder')}
        value={name}
        maxLength={NAME_MAX}
        error={error}
        onChange={(e) => {
          setName(e.currentTarget.value);
          setError(null);
        }}
      />
      <div className={classes.modalActions}>
        <DLButton variant="subtle" onClick={onClose}>{t('common.cancel')}</DLButton>
        <DLButton variant="filled" type="submit" loading={loading}>{t('command.save.action')}</DLButton>
      </div>
    </form>
  );
}
