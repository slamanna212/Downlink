import { useCallback } from 'react';
import { Menu } from '@mantine/core';
import { IconStar } from '@tabler/icons-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { invoke } from '../../api/ipc';
import { invalidateFavourites } from '../../api/queries';
import { t } from '../../lib/i18n';
import { useChannelMenuStore } from '../../store/contextMenu';
import { Icon } from '../Icon/Icon';
import { ContextMenu } from './ContextMenu';

/** Single app-wide host for the channel context menu (mounted once in App). */
export function ChannelContextMenu() {
  const open = useChannelMenuStore((s) => s.open);
  const close = useChannelMenuStore((s) => s.close);
  const qc = useQueryClient();
  const favourite = useMutation({
    mutationFn: (v: { channelId: number; favourite: boolean }) => invoke('set_favourite', v),
    onSuccess: () => invalidateFavourites(qc),
  });
  const onClose = useCallback(() => close(), [close]);
  const channel = open?.channel;
  return (
    <ContextMenu opened={open != null} x={open?.x ?? 0} y={open?.y ?? 0} onClose={onClose}>
      <Menu.Item
        leftSection={<Icon icon={IconStar} size={16} />}
        onClick={() => channel && favourite.mutate({ channelId: channel.id, favourite: !channel.isFavourite })}
      >
        {t(channel?.isFavourite ? 'channels.favourite.remove' : 'channels.favourite.add')}
      </Menu.Item>
      {open?.extra && (
        <>
          <Menu.Divider />
          {open.extra}
        </>
      )}
    </ContextMenu>
  );
}
