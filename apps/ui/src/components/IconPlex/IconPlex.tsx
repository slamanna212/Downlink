import { forwardRef } from 'react';
import type { IconProps } from '@tabler/icons-react';

/**
 * Plex chevron mark. Tabler ships no Plex glyph. Third-party brand colour (`--dl-plex`), the one
 * exception to the accent rules (DESIGN §2). Always a solid fill; stroke/fill props are ignored.
 * Shaped like a Tabler icon so it fits `<Icon icon={IconPlex} />`.
 */
export const IconPlex = forwardRef<SVGSVGElement, IconProps>(function IconPlex(
  { size = 24, className, 'aria-hidden': ariaHidden = true },
  ref,
) {
  return (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      className={className}
      aria-hidden={ariaHidden}
      focusable="false"
    >
      <path fill="var(--dl-plex)" d="M11.643 0H4.68l7.679 12L4.68 24h6.963l7.677-12-7.677-12" />
    </svg>
  );
});
