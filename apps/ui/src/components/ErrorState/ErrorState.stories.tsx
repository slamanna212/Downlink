import type { Meta, StoryObj } from '@storybook/react-vite';
import { ErrorState } from './ErrorState';

const meta: Meta<typeof ErrorState> = { title: 'Primitives/ErrorState', component: ErrorState, args: { onRetry: () => undefined } };
export default meta;
type Story = StoryObj<typeof ErrorState>;
export const Default: Story = {};
export const Custom: Story = { args: { message: 'The server did not answer. Check the address in Settings, then try again.', actionLabel: 'Open settings' } };
