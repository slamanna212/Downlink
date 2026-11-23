import type { MouseEvent } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { invoke } from '../../api/ipc';
import classes from './Markdown.module.css';

/**
 * Renders untrusted Markdown (GitHub release notes). Raw HTML is not rendered (react-markdown drops it; no
 * rehype-raw), URLs go through react-markdown's protocol allow-list, images are capped to the container width,
 * and links open in the system browser through Rust, never in the webview.
 */
export function Markdown({ children }: { children: string }) {
  return (
    <div className={classes.md}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children: kids }) => (
            <a
              href={href}
              rel="noreferrer noopener"
              onClick={(e: MouseEvent) => {
                e.preventDefault();
                if (href) void invoke('open_external_link', { url: href }).catch(() => undefined);
              }}
            >
              {kids}
            </a>
          ),
          img: ({ src, alt }) => <img src={src} alt={alt ?? ''} loading="lazy" referrerPolicy="no-referrer" />,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
