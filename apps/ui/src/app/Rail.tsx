import { Link, useLocation, useNavigate } from 'react-router';
import { Box, Group, NavLink, Stack, Text, Tooltip } from '@mantine/core';
import { IconArrowBarLeft, IconDownload } from '@tabler/icons-react';
import { FocusContext } from '@noriginmedia/norigin-spatial-navigation';
import { useCaps, useSources } from '../api/queries';
import type { Source, SourceStatus } from '../api/types';
import { openPalette } from '../features/palette/CommandPalette';
import { Icon } from '../components/Icon/Icon';
import { DLIconButton } from '../components/DLIconButton/DLIconButton';
import { useDLFocusable } from '../lib/spatial';
import { t } from '../lib/i18n';
import { modKeyLabel } from '../lib/platform';
import { SETTINGS_NAV, visibleNav, type NavEntry } from './nav';
import classes from './Rail.module.css';

export type RailMode = 'expanded' | 'collapsed' | 'tv';

const DOT_CLASS: Record<SourceStatus | 'none', string | undefined> = {
  connected: classes.dotOk,
  syncing: classes.dotProgress,
  offline: classes.dotWarn,
  stale: classes.dotWarn,
  auth_error: classes.dotLive,
  none: classes.dotNone,
};

function RailItem({ entry, mode }: { entry: NavEntry; mode: RailMode }) {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const active = entry.to === '/' ? pathname === '/' : pathname === entry.to || pathname.startsWith(`${entry.to}/`);
  const { ref, focusProps } = useDLFocusable({ onEnterPress: () => (entry.id === 'search' ? openPalette() : void navigate(entry.to)) });
  const label = t(entry.label);
  const hint = entry.hint === '#mod' ? modKeyLabel() : entry.hint;

  const link = (
    <NavLink
      ref={ref as React.Ref<HTMLAnchorElement>}
      component={Link}
      to={entry.to}
      active={active}
      onClick={entry.id === 'search' ? (e: React.MouseEvent) => (e.preventDefault(), openPalette()) : undefined}
      aria-current={active ? 'page' : undefined}
      aria-label={mode === 'collapsed' ? label : undefined}
      className={classes.item}
      classNames={{ body: classes.itemInner, label: classes.itemLabel, section: classes.section }}
      leftSection={
        <span className={classes.icon}>
          <Icon icon={entry.icon} size={mode === 'tv' ? 24 : 20} />
        </span>
      }
      label={mode === 'collapsed' ? undefined : label}
      rightSection={mode === 'expanded' && hint ? <span className={classes.hint}>{hint}</span> : undefined}
      {...focusProps}
    />
  );
  return mode === 'collapsed' ? (
    <Tooltip label={label} position="right" withinPortal>
      {link}
    </Tooltip>
  ) : (
    link
  );
}

function SourceStatus({ source, mode }: { source: Source | undefined; mode: RailMode }) {
  const status = source?.status ?? 'none';
  const label = t(`source.status.${status}`);
  const dot = <span className={`${classes.statusDot} ${DOT_CLASS[status] ?? ''}`} role="img" aria-label={label} />;
  if (mode !== 'expanded') {
    return (
      <Tooltip label={source ? `${source.name} · ${label}` : label} position="right" withinPortal>
        <Group justify="center" py="xs">
          {dot}
        </Group>
      </Tooltip>
    );
  }
  return (
    <Group gap="sm" wrap="nowrap" className={classes.footer}>
      {dot}
      <Box style={{ minWidth: 0 }}>
        <Text className={classes.sourceName} truncate>
          {source?.name ?? t('source.status.none')}
        </Text>
        <Text className={classes.sourceMeta} truncate>
          {source ? label : t('empty.title')}
        </Text>
      </Box>
    </Group>
  );
}

export function Rail({ mode, onToggle }: { mode: RailMode; onToggle: () => void }) {
  const { caps } = useCaps();
  const { data: sources } = useSources();
  const { ref, focusKey } = useDLFocusable<object, HTMLDivElement>({ focusKey: 'RAIL', trackChildren: true, saveLastFocusedChild: true });

  return (
    <FocusContext.Provider value={focusKey}>
      <Stack
        component="nav"
        aria-label={t('nav.primary')}
        ref={ref}
        className={`${classes.rail} ${mode === 'collapsed' ? classes.collapsed : ''} ${mode === 'tv' ? classes.tv : ''}`}
        h="100%"
      >
        <Group className={classes.brand} gap="sm" wrap="nowrap" justify={mode === 'expanded' ? 'space-between' : 'center'}>
          <Group gap="sm" wrap="nowrap">
            <span
              className={classes.brandMark}
              role={mode === 'collapsed' ? 'button' : undefined}
              tabIndex={mode === 'collapsed' ? 0 : undefined}
              aria-label={mode === 'collapsed' ? t('nav.expand') : undefined}
              onClick={mode === 'collapsed' ? onToggle : undefined}
              onKeyDown={mode === 'collapsed' ? (e) => (e.key === 'Enter' || e.key === ' ') && onToggle() : undefined}
            >
              <Icon icon={IconDownload} size={16} />
            </span>
            {mode === 'expanded' && <span className={classes.brandName}>{t('app.name')}</span>}
          </Group>
          {mode === 'expanded' && (
            <DLIconButton label={t('nav.collapse')} icon={IconArrowBarLeft} size="sm" onClick={onToggle} />
          )}
        </Group>
        {visibleNav(caps).map((e) => (
          <RailItem key={e.id} entry={e} mode={mode} />
        ))}
        <div className={classes.spacer} />
        <RailItem entry={SETTINGS_NAV} mode={mode} />
        <SourceStatus source={sources?.[0]} mode={mode} />
      </Stack>
    </FocusContext.Provider>
  );
}
