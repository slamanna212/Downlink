import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn(() => Promise.resolve(null));
vi.mock('../../api/ipc', () => ({ invoke: (...a: unknown[]) => (invoke as (...x: unknown[]) => unknown)(...a) }));

import { Markdown } from './Markdown';

describe('Markdown', () => {
  beforeEach(() => invoke.mockClear());

  it('renders headings, lists and images', () => {
    render(<Markdown>{'## Hello\n- one\n- two\n\n![pic](https://example.invalid/a.png)'}</Markdown>);
    expect(screen.getByRole('heading', { name: 'Hello' })).toBeTruthy();
    expect(screen.getAllByRole('listitem')).toHaveLength(2);
    expect(screen.getByAltText('pic').getAttribute('src')).toBe('https://example.invalid/a.png');
  });

  it('does not render raw HTML', () => {
    const { container } = render(<Markdown>{'hi <script>x()</script><img src=x onerror=y()>'}</Markdown>);
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
  });

  it('drops javascript: links and opens real links externally', () => {
    render(<Markdown>{'[bad](javascript:alert(1)) [ok](https://example.invalid/x)'}</Markdown>);
    expect(screen.getByText('bad').getAttribute('href') ?? '').not.toContain('javascript');
    fireEvent.click(screen.getByText('ok'));
    expect(invoke).toHaveBeenCalledWith('open_external_link', { url: 'https://example.invalid/x' });
  });
});
