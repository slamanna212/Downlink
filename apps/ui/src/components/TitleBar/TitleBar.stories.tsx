import type { Meta, StoryObj } from '@storybook/react-vite';
import { TitleBar } from './TitleBar';

const meta: Meta<typeof TitleBar> = { title: 'Chrome/TitleBar', component: TitleBar };
export default meta;
type Story = StoryObj<typeof TitleBar>;

const frame = (children: React.ReactNode, video = false) => (
  <div style={{ position: 'relative', width: 720, height: 120, background: video ? 'var(--dl-letterbox)' : 'var(--dl-surface-0)', border: '1px solid var(--dl-hairline)' }}>
    {children}
  </div>
);

/** Windows / Linux: drag strip with minimize, maximize and close (red on hover). Hover and press each. */
export const WindowsLinux: Story = { render: () => frame(<TitleBar platform="other" />) };
/** Over video (player route): brighter glyphs. */
export const OnVideo: Story = { render: () => frame(<TitleBar platform="other" onVideo />, true) };
/** Pages without a hero: the strip takes its own row. */
export const InFlow: Story = { render: () => frame(<TitleBar platform="other" inFlow />) };
/** macOS: drag region only; the native traffic lights sit over the rail. */
export const Mac: Story = { render: () => frame(<TitleBar platform="mac" />) };
