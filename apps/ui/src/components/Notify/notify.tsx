import { notifications } from '@mantine/notifications';
import { IconAlertCircle } from '@tabler/icons-react';
import classes from './Notify.module.css';
import { toAppError } from '../../api/ipc';

/** One-sentence failure toast. The id is the message, so a repeat updates in place instead of stacking. */
export function notifyError(e: unknown): void {
  const message = toAppError(e).message;
  notifications.show({ id: `err:${message}`, message, icon: <IconAlertCircle size={20} aria-hidden />, withCloseButton: true,
    classNames: { root: classes.root, description: classes.description, icon: classes.icon, closeButton: classes.closeButton } });
}
