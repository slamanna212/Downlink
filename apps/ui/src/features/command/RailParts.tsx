import { useEffect, useState, type ReactNode } from 'react';
import { IconLayoutSidebarRightCollapse, IconLayoutSidebarRightExpand } from '@tabler/icons-react';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { t } from '../../lib/i18n';
import classes from './Command.module.css';

/** A rail section: 12 px caps label, a faint count, and one dim sentence when empty. */
export function Section({ title, count, empty, children }: { title: string; count: number; empty: string; children: ReactNode }) {
  return (
    <section className={classes.section} aria-label={title}>
      <div className={classes.sectionHead}>
        <span>{title}</span>
        <span className={classes.sectionCount}>{count}</span>
      </div>
      {count === 0 ? <div className={classes.sectionEmpty}>{empty}</div> : children}
    </section>
  );
}

/** Rail visibility, toggled with R (not while typing). */
export function useRailToggle() {
  const [railOpen, setRailOpen] = useState(true);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)) return;
      if (e.key !== 'r' && e.key !== 'R') return;
      e.preventDefault();
      setRailOpen((o) => !o);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  return [railOpen, setRailOpen] as const;
}

/** Toolbar button that shows or hides the rail. */
export function RailToggleButton({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <DLIconButton
      label={open ? t('command.rail.hide') : t('command.rail.show')}
      icon={open ? IconLayoutSidebarRightCollapse : IconLayoutSidebarRightExpand}
      variant="subtle"
      size="sm"
      pressed={open}
      onClick={onToggle}
    />
  );
}
