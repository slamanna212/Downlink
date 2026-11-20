import type { Meta, StoryObj } from '@storybook/react-vite';
import { Notification, Stack } from '@mantine/core';
import { Notifications, notifications } from '@mantine/notifications';
import { IconAlertCircle } from '@tabler/icons-react';
import { DLButton } from '../DLButton/DLButton';
import { StateRow, pseudoStates, type DemoState } from '../StateRow';
import { notifyError } from './notify';
import classes from './Notify.module.css';

const demo = (s: DemoState) => ({ 'data-demo-state': s, ...(s === 'focus' ? { 'data-focused': 'true' } : {}) });

const meta: Meta = { title: 'Primitives/Toast', parameters: pseudoStates };
export default meta;
type Story = StoryObj;

/** Static look in the six states, plus a live trigger (max 3 visible, 5 s, repeats update in place). */
export const States: Story = {
  render: () => (
    <Stack gap="xl" style={{ width: 520 }}>
      <StateRow title="error toast">
        {(s) => (
          <div {...demo(s)} style={{ width: 360 }}>
            <Notification
              withCloseButton
              icon={<IconAlertCircle size={20} aria-hidden />}
              classNames={{ root: classes.root, description: classes.description, icon: classes.icon, closeButton: classes.closeButton }}
              loading={s === 'loading'}
              closeButtonProps={{ disabled: s === 'disabled' }}
            >
              The server did not answer. Try again in a moment.
            </Notification>
          </div>
        )}
      </StateRow>
      <Notifications position="bottom-right" limit={3} autoClose={5000} />
      <DLButton variant="default" onClick={() => notifyError({ code: 'stream', message: 'The server did not answer. Try again in a moment.', retryAfter: null })}>
        Raise a toast
      </DLButton>
      <DLButton variant="subtle" onClick={() => notifications.clean()}>
        Clear
      </DLButton>
    </Stack>
  ),
};
