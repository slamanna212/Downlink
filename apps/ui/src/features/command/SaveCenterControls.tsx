import { useState } from 'react';
import { IconCopyPlus, IconDeviceFloppy } from '@tabler/icons-react';
import { useSaveCommandCenter } from '../../api/queries';
import type { CommandCenterRail } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { notifyInfo } from '../../components/Notify/notify';
import { t } from '../../lib/i18n';
import type { MultiviewApi } from '../multiview/MultiviewPage';
import { CenterNameModal } from './CenterNameModal';
import { wallTiles } from './saved';

export interface SavedRef {
  id: number;
  name: string;
}

export interface SaveCenterControlsProps {
  api: Pick<MultiviewApi, 'tiles' | 'style' | 'audible'>;
  /** The saved center this wall is, or null for a wall that was never saved. */
  center: SavedRef | null;
  /** Tiles or layout changed since the last save: offers "Save changes". */
  edited: boolean;
  /** Channels of the center that are not on screen (they stay in the center). */
  keep?: number[];
  /** The center's rail document, kept as is. */
  rail?: CommandCenterRail;
  onSaved: (center: SavedRef, asNew: boolean) => void;
}

/**
 * Multiview toolbar controls that save the wall: "Save as command center" for a new wall; "Save
 * changes" (when edited) and "Save as new" for a saved one. Slot 0 is the main player's channel.
 */
export function SaveCenterControls({ api, center, edited, keep, rail, onSaved }: SaveCenterControlsProps) {
  const save = useSaveCommandCenter();
  const [naming, setNaming] = useState(false);
  const empty = api.tiles.size === 0 && !keep?.length;

  const run = (id: number | null, name: string) => {
    save.mutate(
      { id, name, layoutStyle: api.style, audible: api.audible, rail: rail ?? {}, tiles: wallTiles(api.tiles, keep) },
      {
        onSuccess: (newId) => {
          setNaming(false);
          notifyInfo(t('command.saved', { name }));
          onSaved({ id: newId, name }, id == null);
        },
      },
    );
  };

  return (
    <>
      {center == null ? (
        <DLButton variant="default" size="sm" icon={IconDeviceFloppy} disabled={empty} onClick={() => setNaming(true)}>
          {t('command.save.asCenter')}
        </DLButton>
      ) : (
        <>
          {edited && (
            <DLButton variant="default" size="sm" icon={IconDeviceFloppy} disabled={empty} loading={save.isPending && !naming} onClick={() => run(center.id, center.name)}>
              {t('command.save.changes')}
            </DLButton>
          )}
          <DLIconButton label={t('command.save.asNew')} icon={IconCopyPlus} size="sm" disabled={empty} onClick={() => setNaming(true)} />
        </>
      )}
      <CenterNameModal
        opened={naming}
        title={center == null ? t('command.save.title') : t('command.save.titleNew')}
        initial={center?.name ?? t('command.name.default')}
        loading={save.isPending}
        onClose={() => setNaming(false)}
        onSubmit={(name) => run(null, name)}
      />
    </>
  );
}
