import type { ComponentType } from 'react';
import type { IconProps as TablerIconProps } from '@tabler/icons-react';
import { dl } from '../../theme';
import { useUiStore } from '../../store/ui';

export type IconSize = (typeof dl.iconSizes)[number];
export type TablerIcon = ComponentType<TablerIconProps>;

export interface IconProps {
  icon: TablerIcon;
  /** 14 | 16 | 20 | 24; becomes 26 in 10-foot density. */
  size?: IconSize;
  /** Filled glyph: only for play/pause/star-on (DESIGN §6). */
  filled?: boolean;
  className?: string;
  'aria-hidden'?: boolean;
}

/** The only way to render a Tabler icon: stroke 1.75, size from the enum. */
export function Icon({ icon: Glyph, size = 16, filled = false, className, ...rest }: IconProps) {
  const tv = useUiStore((s) => s.density === 'tv');
  const px = tv && size === 24 ? 26 : size;
  return (
    <Glyph
      size={px}
      stroke={dl.iconStroke}
      className={className}
      fill={filled ? 'currentColor' : 'none'}
      aria-hidden={rest['aria-hidden'] ?? true}
    />
  );
}
