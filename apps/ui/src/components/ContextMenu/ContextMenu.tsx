import { useEffect, type ReactNode } from 'react';
import { Menu } from '@mantine/core';

export interface ContextMenuProps {
  opened: boolean;
  /** Viewport coordinates of the right click. */
  x: number;
  y: number;
  onClose: () => void;
  /** `Menu.Item` children. */
  children: ReactNode;
}

/**
 * App context menu: a Mantine Menu anchored at the cursor. Closes on Escape, click-away and scroll
 * (Mantine handles the first two; scroll is a capture listener so list scrolling dismisses it).
 */
export function ContextMenu({ opened, x, y, onClose, children }: ContextMenuProps) {
  useEffect(() => {
    if (!opened) return;
    const close = () => onClose();
    window.addEventListener('scroll', close, true);
    window.addEventListener('wheel', close, { passive: true });
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('wheel', close);
      window.removeEventListener('blur', close);
    };
  }, [opened, onClose]);

  return (
    <Menu opened={opened} onChange={(o) => !o && onClose()} position="bottom-start" offset={2} withinPortal trapFocus returnFocus keepMounted={false}>
      <Menu.Target>
        <span aria-hidden style={{ position: 'fixed', left: x, top: y, width: 0, height: 0, pointerEvents: 'none' }} />
      </Menu.Target>
      <Menu.Dropdown>{children}</Menu.Dropdown>
    </Menu>
  );
}
