import { useCallback, useRef } from 'react';
import { UnstyledButton } from '@mantine/core';
import { defaultRangeExtractor, useVirtualizer, type Range } from '@tanstack/react-virtual';
import { useLocalStorage } from '@mantine/hooks';
import { DLButton } from '../../components/DLButton/DLButton';
import { orderGroups, type GroupOrder } from './groupOrder';
import type { Group } from '../../api/types';
import { setFocus } from '@noriginmedia/norigin-spatial-navigation';
import { useDLFocusable } from '../../lib/spatial';
import { t } from '../../lib/i18n';
import classes from './Channels.module.css';

export type GroupSelection = { kind: 'all' } | { kind: 'favourites' } | { kind: 'group'; id: number };

export const selectionKey = (s: GroupSelection) => (s.kind === 'group' ? `g${s.id}` : s.kind);

/** Fixed row height of a group entry (matches `.group` height). */
const ROW_H = 38;

interface GroupItemProps {
  label: string;
  count: number;
  active: boolean;
  onSelect: () => void;
  focusKey: string;
  onFocused?: () => void;
  style?: React.CSSProperties;
}

function GroupItem({ label, count, active, onSelect, focusKey, onFocused, style }: GroupItemProps) {
  const { ref, focusProps } = useDLFocusable<object, HTMLButtonElement>({
    focusKey,
    onEnterPress: onSelect,
    onFocus: onFocused,
    onArrowPress: (dir) => {
      if (dir === 'right') {
        setFocus('CHANNEL_LIST');
        return false;
      }
      return true;
    },
  });
  return (
    <UnstyledButton ref={ref} className={classes.group} style={style} data-active={active || undefined} aria-current={active ? 'true' : undefined} onClick={onSelect} {...focusProps}>
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
  const [order, setOrder] = useLocalStorage<GroupOrder>({ key: 'dl.groupOrder', defaultValue: 'alphabetical' });
  const providerOrder = order === 'provider';
  const isActive = (s: GroupSelection) => selectionKey(s) === selectionKey(selection);
  const entries: { key: string; label: string; count: number; sel: GroupSelection }[] = [
    { key: 'favourites', label: t('channels.favourites'), count: favourites, sel: { kind: 'favourites' } },
    { key: 'all', label: t('channels.all'), count: total, sel: { kind: 'all' } },
    ...orderGroups(groups, order).map((g) => ({ key: `g${g.id}`, label: g.name, count: g.count, sel: { kind: 'group', id: g.id } as GroupSelection })),
  ];
  // Sources with thousands of groups: only the entries in view are mounted. The selected one always
  // is, so focus can move to it from the channel list (ArrowLeft) wherever the list is scrolled.
  const activeIndex = entries.findIndex((e) => isActive(e.sel));
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const rangeExtractor = useCallback(
    (range: Range) => {
      const r = defaultRangeExtractor(range);
      return activeIndex >= 0 && !r.includes(activeIndex) ? [...r, activeIndex].sort((a, b) => a - b) : r;
    },
    [activeIndex],
  );
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_H,
    overscan: 10,
    rangeExtractor,
  });
  return (
    <nav className={classes.sidebar} aria-label={t('channels.groups')}>
      <div className={classes.sidebarLabel}>{t('channels.groups')}</div>
      <DLButton variant="subtle" size="xs" aria-label={t('channels.groupOrder')} aria-pressed={providerOrder}
        onClick={() => setOrder(providerOrder ? 'alphabetical' : 'provider')}>
        {t(providerOrder ? 'channels.groupOrder.provider' : 'channels.groupOrder.alpha')}
      </DLButton>
      <div ref={scrollRef} className={classes.groupList}>
        <div className={classes.groupInner} style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => {
            const e = entries[v.index];
            if (!e) return null;
            return (
              <GroupItem
                key={e.key}
                focusKey={`GROUP_${e.key}`}
                label={e.label}
                count={e.count}
                active={isActive(e.sel)}
                onSelect={() => onSelect(e.sel)}
                // Keep the neighbours of a keyboard-focused entry mounted.
                onFocused={() => virtualizer.scrollToIndex(v.index)}
                style={{ position: 'absolute', top: 0, left: 0, transform: `translateY(${v.start}px)` }}
              />
            );
          })}
        </div>
      </div>
    </nav>
  );
}
