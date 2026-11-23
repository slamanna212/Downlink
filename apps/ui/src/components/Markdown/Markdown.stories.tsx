import type { Meta, StoryObj } from '@storybook/react-vite';
import { Markdown } from './Markdown';

const sample = [
  "## What's new",
  '- Faster **guide** scrolling',
  '- See [the docs](https://example.invalid/docs)',
  '',
  '![A wide picture](data:image/svg+xml;utf8,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%222000%22 height=%22300%22%3E%3Crect width=%222000%22 height=%22300%22 fill=%22%23554%22/%3E%3C/svg%3E)',
  '',
  '| a | b |',
  '|---|---|',
  '| 1 | 2 |',
  '',
  '<script>alert(1)</script>',
].join('\n');

const meta: Meta<typeof Markdown> = {
  title: 'Primitives/Markdown',
  component: Markdown,
  args: { children: sample },
};
export default meta;
type Story = StoryObj<typeof Markdown>;

/** Links are focusable and open externally; raw HTML is not rendered; images never exceed the width. */
export const Playground: Story = {};
export const Narrow: Story = { decorators: [(S) => <div style={{ width: 280 }}><S /></div>] };
