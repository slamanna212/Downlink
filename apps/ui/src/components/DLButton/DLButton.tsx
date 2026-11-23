import { forwardRef, useLayoutEffect, useRef, type MouseEvent, type ReactNode } from 'react';
import { Button, Loader, type ButtonProps } from '@mantine/core';
import { dl } from '../../theme';
import { useUiStore } from '../../store/ui';
import { Icon, type TablerIcon } from '../Icon/Icon';
import classes from '../Primitives.module.css';

export type DLButtonVariant = 'filled' | 'default' | 'subtle' | 'live' | 'danger' | 'overlay';
export type DLButtonSize = 'xs' | 'sm' | 'md' | 'lg';

export interface DLButtonProps
  extends Omit<ButtonProps, 'variant' | 'size' | 'leftSection' | 'rightSection' | 'loading' | 'color'> {
  variant?: DLButtonVariant;
  /** tv size is automatic from <html data-density="tv"> and applies to md. */
  size?: DLButtonSize;
  /** Icon on the left only. */
  icon?: TablerIcon;
  /** Filled glyph (play/pause only, DESIGN §6). */
  filledGlyph?: boolean;
  /** Loader replaces the icon, label stays, width stays locked. Clicks are ignored while loading. */
  loading?: boolean;
  /** Optional key hint (Geist Mono 11px), at most one per view. */
  keyHint?: string;
  children?: ReactNode;
  onClick?: (e: MouseEvent<HTMLButtonElement>) => void;
  type?: 'button' | 'submit' | 'reset';
  'aria-label'?: string;
  'data-focused'?: string;
}

export const DLButton = forwardRef<HTMLButtonElement, DLButtonProps>(function DLButton(
  { variant = 'default', size = 'md', icon, filledGlyph = false, loading = false, keyHint, children, onClick, style, disabled, ...rest },
  ref,
) {
  const tvDensity = useUiStore((s) => s.density === 'tv');
  const spec = dl.buttonSizes[tvDensity && size === 'md' ? 'tv' : size];
  const inner = useRef<HTMLButtonElement | null>(null);

  // Lock the width at the moment loading starts so the swap icon -> loader never shifts layout.
  useLayoutEffect(() => {
    const el = inner.current;
    if (!el) return;
    if (loading) el.style.minWidth = `${el.offsetWidth}px`;
    else el.style.minWidth = '';
  }, [loading]);

  const leftSection = loading ? (
    <Loader size={spec.icon} color="currentColor" type="oval" />
  ) : variant === 'live' ? (
    <span className={classes.liveDot} aria-hidden />
  ) : icon ? (
    <Icon icon={icon} size={spec.icon as 14 | 16 | 20 | 24} filled={filledGlyph} />
  ) : undefined;

  return (
    <Button
      {...rest}
      ref={(node) => {
        inner.current = node;
        if (typeof ref === 'function') ref(node);
        else if (ref) ref.current = node;
      }}
      variant={variant}
      size={size}
      radius={spec.r}
      disabled={disabled}
      leftSection={leftSection}
      aria-busy={loading || undefined}
      data-busy={loading || undefined}
      onClick={loading ? (e) => e.preventDefault() : onClick}
      style={{
        '--button-height': `${spec.h}px`,
        '--button-fz': `${spec.fz}px`,
        '--button-padding-x': `${spec.px}px`,
        '--button-radius': `${spec.r}px`,
        ...style,
      }}
    >
      {children}
      {keyHint ? <span className={classes.keyHint}>{keyHint}</span> : null}
    </Button>
  );
});
