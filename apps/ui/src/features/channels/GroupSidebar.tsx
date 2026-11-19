import { UnstyledButton } from '@mantine/core';
import type { Group } from '../../api/types';
import { setFocus } from '@noriginmedia/norigin-spatial-navigation';
import { useDLFocusable } from '../../lib/spatial';
import { t } from '../../lib/i18n';
import classes from './Channels.module.css';

export type GroupSelection = { kind: 'all' } | { kind: 'favourites' } | { kind: 'group'; id: number };

export const selectionKey = (s: GroupSelection) => (s.kind === 'group' ? `g${s.id}` : s.kind);

function GroupItem({ label, count, active, onSelect, focusKey }: { label: string; count: number; active: boolean; onSelect: () => void; focusKey: string }) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({
    focusKey,
    onEnterPress: onSelect,
    onArrowPress: (dir) => {
      if (dir === 'right') {
        setFocus('CHANNEL_LIST');
        return false;
      }
      return true;
    },
  });
  return (
    <UnstyledButton ref={ref} className={classes.group} data-active={active || undefined} aria-current={active ? 'true' : undefined} onClick={onSelect} {...focusProps}>
      <span>{label}</span>
      <span className={classes.groupCount}>{count}</span>
    </UnstyledButton>
  );
}

export interface GroupSidebarProps {
  groups: Group[];
  total: number;
  favourites: number;
  selection: GroupSelection;
  onSelect: (s: GroupSelection) => void;
}

export function GroupSidebar({ groups, total, favourites, selection, onSelect }: GroupSidebarProps) {
  const isActive = (s: GroupSelection) => selectionKey(s) === selectionKey(selection);
  const entries: { key: string; label: string; count: number; sel: GroupSelection }[] = [
    { key: 'favourites', label: t('channels.favourites'), count: favourites, sel: { kind: 'favourites' } },
    { key: 'all', label: t('channels.all'), count: total, sel: { kind: 'all' } },
    ...groups.map((g) => ({ key: `g${g.id}`, label: g.name, count: g.channelCount, sel: { kind: 'group', id: g.id } as GroupSelection })),
  ];
  return (
    <nav className={classes.sidebar} aria-label={t('channels.groups')}>
      <div className={classes.sidebarLabel}>{t('channels.groups')}</div>
      {entries.map((e) => (
        <GroupItem key={e.key} focusKey={`GROUP_${e.key}`} label={e.label} count={e.count} active={isActive(e.sel)} onSelect={() => onSelect(e.sel)} />
      ))}
    </nav>
  );
}
