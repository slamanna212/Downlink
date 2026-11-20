import { notifications } from '@mantine/notifications';
import { IconAlertCircle, IconCircleCheck } from '@tabler/icons-react';
import classes from './Notify.module.css';
import { toAppError } from '../../api/ipc';

/** One-sentence failure toast. The id is the message, so a repeat updates in place instead of stacking. */
export function notifyError(e: unknown): void {
  const message = toAppError(e).message;
  notifications.show({ id: `err:${message}`, message, icon: <IconAlertCircle size={20} aria-hidden />, withCloseButton: true,
    classNames: { root: classes.root, description: classes.description, icon: classes.icon, closeButton: classes.closeButton } });
}

/** One-sentence confirmation (e.g. favourite toggled). Same limits as errors. */
export function notifyInfo(message: string): void {
  notifications.show({
    id: `info:${message}`,
    message,
    icon: <IconCircleCheck size={20} aria-hidden />,
    withCloseButton: true,
    classNames: { root: classes.root, description: classes.description, icon: classes.icon, closeButton: classes.closeButton },
  });
}
