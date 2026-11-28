import { useState } from 'react';
import { useNavigate } from 'react-router';
import { useQueryClient } from '@tanstack/react-query';
import { Menu, Modal, Text } from '@mantine/core';
import { IconDots, IconPencil, IconTrash } from '@tabler/icons-react';
import { commandCenterQuery, useDeleteCommandCenter, useSaveCommandCenter } from '../../api/queries';
import type { CommandCenter } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { notifyError } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import { CenterNameModal } from './CenterNameModal';
import classes from './Command.module.css';

/** Rename / delete for the center (delete asks first, in a modal). */
export function CenterMenu({ center, onDeleted }: { center: Pick<CommandCenter, 'id' | 'name'> & Partial<CommandCenter>; onDeleted?: () => void }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const save = useSaveCommandCenter();
  const del = useDeleteCommandCenter();

  const rename = async (name: string) => {
    // Renames only: the saved tiles, layout and rail stay as stored (unsaved wall edits are not saved).
    const full = center.tiles ? (center as CommandCenter) : await qc.fetchQuery(commandCenterQuery(center.id));
    save.mutate(
      {
        id: full.id,
        name,
        layoutStyle: full.layoutStyle,
        audible: full.audible,
        rail: full.rail,
        tiles: full.tiles.map((x) => ({ slot: x.slot, channelId: x.channel.id })),
      },
      { onSuccess: () => setRenaming(false) },
    );
  };

  return (
    <>
      <Menu position="bottom-end" withinPortal trapFocus returnFocus>
        <Menu.Target>
          <DLIconButton label={t('command.card.menu', { name: center.name })} icon={IconDots} size="sm" />
        </Menu.Target>
        <Menu.Dropdown aria-label={t('command.menu')}>
          <Menu.Item leftSection={<Icon icon={IconPencil} size={16} />} onClick={() => setRenaming(true)}>{t('command.rename')}</Menu.Item>
          <Menu.Item color="live" leftSection={<Icon icon={IconTrash} size={16} />} onClick={() => setDeleting(true)}>{t('command.delete')}</Menu.Item>
        </Menu.Dropdown>
      </Menu>
      <CenterNameModal
        opened={renaming}
        title={t('command.rename.title')}
        initial={center.name}
        loading={save.isPending}
        onClose={() => setRenaming(false)}
        onSubmit={(name) => void rename(name).catch(notifyError)}
      />
      <Modal opened={deleting} onClose={() => setDeleting(false)} title={t('command.delete.title')} withinPortal>
        <Text size="md" mb="lg">{t('command.delete.confirm', { name: center.name })}</Text>
        <div className={classes.modalActions}>
          <DLButton variant="subtle" onClick={() => setDeleting(false)}>{t('common.cancel')}</DLButton>
          <DLButton
            variant="danger"
            loading={del.isPending}
            onClick={() =>
              del.mutate(center.id, {
                onSuccess: () => {
                  setDeleting(false);
                  if (onDeleted) onDeleted();
                  else void navigate('/command', { replace: true });
                },
              })
            }
          >
            {t('command.delete')}
          </DLButton>
        </div>
      </Modal>
    </>
  );
}
