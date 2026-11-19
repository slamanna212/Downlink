import { forwardRef, type MouseEvent } from 'react';
import { ActionIcon, Loader, Tooltip, type ActionIconProps } from '@mantine/core';
import { dl } from '../../theme';
import { useUiStore } from '../../store/ui';
import { Icon, type IconSize, type TablerIcon } from '../Icon/Icon';

export type DLIconButtonVariant = 'subtle' | 'default' | 'filled' | 'live' | 'danger' | 'overlay';
/** 32 / 36 / 44 (44 = on video). tv (56) is automatic. */
export type DLIconButtonSize = 'sm' | 'md' | 'lg';

const SIZE_PX: Record<DLIconButtonSize, number> = { sm: 32, md: 36, lg: 44 };
const GLYPH: Record<DLIconButtonSize, IconSize> = { sm: 16, md: 16, lg: 20 };

export interface DLIconButtonProps
  extends Omit<ActionIconProps, 'variant' | 'size' | 'loading' | 'color' | 'children'> {
  /** Mandatory: becomes aria-label and the Tooltip text. */
  label: string;
  icon: TablerIcon;
  variant?: DLIconButtonVariant;
  size?: DLIconButtonSize;
  /** Toggle state: sets aria-pressed and the accent tint. Omit for a plain (non-toggle) button. */
  pressed?: boolean;
  filledGlyph?: boolean;
  loading?: boolean;
  onClick?: (e: MouseEvent<HTMLButtonElement>) => void;
  'data-focused'?: string;
  tabIndex?: number;
}

export const DLIconButton = forwardRef<HTMLButtonElement, DLIconButtonProps>(function DLIconButton(
  { label, icon, variant = 'subtle', size = 'md', pressed, filledGlyph, loading = false, onClick, style, ...rest },
  ref,
) {
  const tv = useUiStore((s) => s.density === 'tv');
  const px = tv && size === 'md' ? dl.buttonSizes.tv.h : SIZE_PX[size];
  const glyph = tv && size === 'md' ? 24 : GLYPH[size];
  return (
    <Tooltip label={label} withinPortal>
      <ActionIcon
        {...rest}
        ref={ref}
        variant={variant}
        size={px}
        radius={size === 'lg' ? 'lg' : 'md'}
        aria-label={label}
        aria-pressed={pressed}
        aria-busy={loading || undefined}
        data-busy={loading || undefined}
        onClick={loading ? (e) => e.preventDefault() : onClick}
        style={style}
      >
        {loading ? <Loader size={glyph} color="currentColor" type="oval" /> : <Icon icon={icon} size={glyph} filled={filledGlyph} />}
      </ActionIcon>
    </Tooltip>
  );
});
