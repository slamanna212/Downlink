import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Popover, SegmentedControl, Switch, TextInput, VisuallyHidden } from '@mantine/core';
import {
  IconArrowsMaximize,
  IconGauge,
  IconLayoutGrid,
  IconLayoutSidebarRight,
  IconPlus,
  IconReplace,
  IconSquareRoundedArrowUp,
  IconVolume,
  IconVolumeOff,
  IconX,
} from '@tabler/icons-react';
import { inTauri, invoke, listen, toAppError } from '../../api/ipc';
import { useChannels, useHome, useNowNextWindow } from '../../api/queries';
import type { Channel, MultiviewState, MultiviewTile } from '../../api/types';
import { ChannelLogo } from '../../components/ChannelLogo/ChannelLogo';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { Icon } from '../../components/Icon/Icon';
import { notifyError } from '../../components/Notify/notify';
import { t, type MessageKey } from '../../lib/i18n';
import { progress } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { useVideoBounds } from '../../lib/useVideoBounds';
import { usePlayerStore } from '../../store/player';
import { playChannel } from '../player/actions';
import { holdSession, releaseSession } from '../player/session';
import { layoutFor, nextStyle, stylesFor, type GridArea, type LayoutStyle } from './layout';
import classes from './Multiview.module.css';

const STYLE_ICON = { grid: IconLayoutGrid, featured: IconLayoutSidebarRight } as const;

const EMPTY: MultiviewState = { tiles: [], audible: 0, maxTiles: 4, streamLimit: null };

/** Multiview state from Rust: `multiview://state` pushes it, `multiview://tile` asks for a refetch. */
function useMultiviewState() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['multiview'], queryFn: () => invoke('get_multiview', {}) });
  useEffect(() => {
    const offs: (() => void)[] = [];
    let cancelled = false;
    const keep = (u: () => void) => (cancelled ? u() : offs.push(u));
    void listen('multiview://state', (s) => qc.setQueryData(['multiview'], s)).then(keep);
    void listen('multiview://tile', () => void qc.invalidateQueries({ queryKey: ['multiview'] })).then(keep);
    return () => {
      cancelled = true;
      offs.forEach((u) => u());
    };
  }, [qc]);
  return q.data ?? EMPTY;
}

type PickerTab = 'favourites' | 'recent' | 'all';

/** Channel picker for a slot: search plus Favourites / Recent / All; on-screen channels are disabled. */
function ChannelPicker({ channels, recents, onScreen, budgetText, onPick }: {
  channels: Channel[];
  recents: Channel[];
  onScreen: Map<number, number>;
  budgetText: string;
  onPick: (c: Channel) => void;
}) {
  const [query, setQuery] = useState('');
  // Open on the first tab that has something in it.
  const [tab, setTab] = useState<PickerTab>(() =>
    channels.some((c) => c.isFavourite) ? 'favourites' : recents.length ? 'recent' : 'all',
  );
  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    const base = q ? channels : tab === 'favourites' ? channels.filter((c) => c.isFavourite) : tab === 'recent' ? recents : channels;
    const hits = q ? base.filter((c) => c.name.toLowerCase().includes(q) || String(c.number ?? '').startsWith(q)) : base;
    return hits.filter((c) => !c.isRadio).slice(0, 60);
  }, [channels, recents, query, tab]);
  return (
    <div className={classes.picker}>
      <TextInput
        data-autofocus
        placeholder={t('multiview.searchChannels')}
        aria-label={t('multiview.searchChannels')}
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
      />
      {!query && (
        <SegmentedControl
          fullWidth
          size="xs"
          value={tab}
          onChange={(v) => setTab(v as PickerTab)}
          data={[
            { value: 'favourites', label: t('multiview.tab.favourites') },
            { value: 'recent', label: t('multiview.tab.recent') },
            { value: 'all', label: t('multiview.tab.all') },
          ]}
        />
      )}
      <div className={classes.pickerList} role="listbox" aria-label={t('multiview.pickChannel')}>
        {list.length === 0 && <div className={classes.pickerEmpty}>{t('multiview.noChannels')}</div>}
        {list.map((c) => {
          const slot = onScreen.get(c.id);
          return (
            <button
              key={c.id}
              type="button"
              role="option"
              aria-selected={false}
              className={classes.pickerRow}
              disabled={slot != null}
              onClick={() => onPick(c)}
            >
              <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" />
              <span className={classes.pickerNumber}>{c.number ?? ''}</span>
              <span className={classes.pickerName}>{c.name}</span>
              {slot != null && <span className={classes.pickerHint}>{t('multiview.alreadyIn', { n: slot + 1 })}</span>}
            </button>
          );
        })}
      </div>
      <div className={classes.pickerFooter}>{budgetText}</div>
    </div>
  );
}

interface TileProps {
  slot: number;
  /** 0-based place in the reflowed grid; labels and the 1–9 keys use it. */
  position: number;
  area: GridArea;
  tile: MultiviewTile | undefined;
  channel: Channel | undefined;
  focused: boolean;
  audible: boolean;
  pickerOpen: boolean;
  picker: React.ReactNode;
  onPickerChange: (open: boolean) => void;
  onFocus: () => void;
  onListen: () => void;
  onMakeMain: () => void;
  onFullScreen: () => void;
  onRemove: () => void;
  now: number;
  programme: { title: string | null; startUtc: number; endUtc: number } | null;
}

function Tile(p: TileProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  // Slot 0 is the main player's surface; tiles position their own.
  useVideoBounds(ref, !!p.tile && p.tile.failure == null, p.slot, `${p.area.gridColumn}|${p.area.gridRow}`);
  const failed = p.tile?.state === 'Failed';
  const loading = p.tile && ['Resolving', 'Connecting', 'Buffering'].includes(p.tile.state);
  return (
    <div
      ref={ref}
      className={classes.tile}
      style={p.area}
      data-focused={p.focused || undefined}
      data-audible={(p.audible && !!p.tile) || undefined}
      data-empty={!p.tile || undefined}
      data-failed={failed || undefined}
      onMouseDown={p.onFocus}
      aria-label={p.channel ? t('multiview.tileLabel', { n: p.position + 1, name: p.channel.name }) : t('multiview.emptySlot', { n: p.position + 1 })}
      role="group"
    >
      {!inTauri() && p.tile && !failed && <div className={classes.devVideo} aria-hidden>tile {p.position + 1}</div>}
      {!p.tile ? (
        <Popover opened={p.pickerOpen} onChange={p.onPickerChange} width={360} position="bottom" trapFocus withinPortal shadow="lg">
          <div className={classes.empty}>
            <Popover.Target>
              <DLButton variant="default" icon={IconPlus} onClick={() => p.onPickerChange(!p.pickerOpen)}>
                {t('multiview.addChannel')}
              </DLButton>
            </Popover.Target>
          </div>
          <Popover.Dropdown>{p.picker}</Popover.Dropdown>
        </Popover>
      ) : failed && p.tile.failure === 'limit' ? (
        <div className={classes.limit} role="alert">
          <Icon icon={IconGauge} size={24} />
          <div className={classes.limitTitle}>{t('player.error.limit')}</div>
          <div className={classes.limitBody}>{t('multiview.limitBody')}</div>
          <div className={classes.limitActions}>
            <Popover opened={p.pickerOpen} onChange={p.onPickerChange} width={360} position="bottom" trapFocus withinPortal shadow="lg">
              <Popover.Target>
                <DLButton variant="default" size="sm" onClick={() => p.onPickerChange(!p.pickerOpen)}>
                  {t('multiview.replace')}
                </DLButton>
              </Popover.Target>
              <Popover.Dropdown>{p.picker}</Popover.Dropdown>
            </Popover>
            <DLButton variant="subtle" size="sm" onClick={p.onRemove}>
              {t('multiview.remove')}
            </DLButton>
          </div>
        </div>
      ) : (
        <>
          {p.channel && (
            <div className={`${classes.label} ${classes.glass}`}>
              <ChannelLogo channelId={p.channel.id} name={p.channel.name} src={p.channel.logo} density="compact" />
              <span className={classes.labelNumber}>{p.channel.number ?? ''}</span>
              <span className={classes.labelName}>{p.channel.name}</span>
            </div>
          )}
          <div className={classes.audio}>
            {p.audible ? <span className={classes.audioBadge}>{t('multiview.audio')}</span> : <Icon icon={IconVolumeOff} size={16} />}
          </div>
          {(loading || failed) && (
            <div className={classes.status} role="status">
              {failed ? t(`player.error.${p.tile.failure ?? 'stream'}` as MessageKey) : t(`player.state.${p.tile.state}` as MessageKey)}
            </div>
          )}
          <div className={classes.actions}>
            {!p.audible && <DLIconButton label={t('multiview.listen')} icon={IconVolume} variant="overlay" size="md" onClick={p.onListen} />}
            {p.slot !== 0 && <DLIconButton label={t('multiview.makeMain')} icon={IconSquareRoundedArrowUp} variant="overlay" size="md" onClick={p.onMakeMain} />}
            <DLIconButton label={t('multiview.fullScreen')} icon={IconArrowsMaximize} variant="overlay" size="md" onClick={p.onFullScreen} />
            <Popover opened={p.pickerOpen} onChange={p.onPickerChange} width={360} position="bottom" trapFocus withinPortal shadow="lg">
              <Popover.Target>
                <DLIconButton label={t('multiview.replace')} icon={IconReplace} variant="overlay" size="md" onClick={() => p.onPickerChange(!p.pickerOpen)} />
              </Popover.Target>
              <Popover.Dropdown>{p.picker}</Popover.Dropdown>
            </Popover>
            {p.slot !== 0 && <DLIconButton label={t('multiview.remove')} icon={IconX} variant="danger" size="md" onClick={p.onRemove} />}
          </div>
          {p.programme && (
            <div className={classes.programme}>
              <div className={classes.programmeTitle}>{p.programme.title ?? ''}</div>
              <div className={classes.programmeBar}>
                <div style={{ width: `${progress(p.programme.startUtc, p.programme.endUtc, p.now) * 100}%` }} />
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}

export function MultiviewPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const player = usePlayerStore((s) => s.player);
  const mv = useMultiviewState();
  const channels = useChannels({});
  const home = useHome();
  const now = useNowEpoch(15_000);
  const byId = useMemo(() => new Map((channels.data ?? []).map((c) => [c.id, c])), [channels.data]);

  // Slot 0 mirrors the main player: its channel when playing live.
  const mainChannel = player.mode === 'live' && player.channelId != null ? player.channelId : null;
  const tiles = useMemo(() => {
    const m = new Map<number, MultiviewTile>();
    mv.tiles.filter((x) => x.slot !== 0).forEach((x) => m.set(x.slot, x));
    if (mainChannel != null) {
      m.set(0, { slot: 0, channelId: mainChannel, state: player.state, failure: player.state === 'Failed' ? (player.error?.reason ?? 'stream') : null });
    }
    return m;
  }, [mv.tiles, mainChannel, player.state, player.error]);
  // The grid follows the tiles on screen, in slot order (the main player first); holes left by a
  // removed tile close up. With nothing playing, one empty slot 0 offers the picker.
  const order = useMemo(() => [...tiles.keys()].sort((a, b) => a - b), [tiles]);
  const shown = order.length ? order : [0];
  const styles = stylesFor(shown.length);
  const [style, setStyle] = useState<LayoutStyle>('grid');
  const effectiveStyle = styles.includes(style) ? style : 'grid';
  const spec = layoutFor(shown.length, effectiveStyle);

  const [focused, setFocused] = useState(0);
  const [audioFollows, setAudioFollows] = useState(true);
  const [pickerSlot, setPickerSlot] = useState<number | null>(null);
  const [addOpen, setAddOpen] = useState(false);

  // Keep the session (main player) alive while here; every tile stops when leaving (rule 6).
  useEffect(() => {
    holdSession();
    return () => {
      releaseSession();
      void invoke('multiview_exit', {}).catch(() => undefined);
    };
  }, []);

  // "Add to multiview" from a channel: the channel lands in the first free slot.
  const pendingAdd = (location.state as { add?: number } | null)?.add;
  const added = useRef(false);

  const ids = useMemo(() => [...tiles.values()].map((x) => x.channelId), [tiles]);
  const nn = useNowNextWindow(ids, `multiview|${ids.join(',')}`);
  const nowOf = (id: number) => nn.data?.find((x) => x.channelId === id)?.now ?? null;

  const inUse = tiles.size;
  const budget = Math.min(mv.maxTiles, mv.streamLimit ?? mv.maxTiles);
  const budgetText = t('multiview.budget', { used: inUse, total: budget });
  const onScreen = useMemo(() => new Map(order.map((slot, i) => [tiles.get(slot)?.channelId ?? -1, i])), [order, tiles]);
  const freeSlot = Array.from({ length: mv.maxTiles }, (_, i) => i).find((s) => !tiles.has(s)) ?? null;

  const assignTile = useCallback(
    async (slot: number, channelId: number) => {
      if (slot === 0) {
        await playChannel(channelId);
        return;
      }
      try {
        await invoke('multiview_set_tile', { slot, channelId });
      } catch (e) {
        notifyError(toAppError(e));
      }
    },
    [],
  );
  const setTile = useCallback(
    (slot: number, channelId: number) => {
      setPickerSlot(null);
      setAddOpen(false);
      return assignTile(slot, channelId);
    },
    [assignTile],
  );

  useEffect(() => {
    if (added.current || pendingAdd == null || !mv.maxTiles) return;
    added.current = true;
    // The grid reflows on its own once the tile lands.
    if (freeSlot == null) notifyError(t('multiview.full'));
    else void assignTile(freeSlot, pendingAdd);
  }, [pendingAdd, mv.maxTiles, freeSlot, assignTile]);

  const listen_ = useCallback((slot: number) => void invoke('multiview_set_audio', { slot }), []);
  const focus = useCallback(
    (slot: number) => {
      setFocused(slot);
      if (audioFollows && tiles.has(slot) && mv.audible !== slot) listen_(slot);
    },
    [audioFollows, tiles, mv.audible, listen_],
  );
  const makeMain = useCallback((slot: number) => void invoke('multiview_make_main', { slot }).catch(notifyError), []);
  const remove = useCallback((slot: number) => void invoke('multiview_remove_tile', { slot }), []);
  const exit = useCallback(() => void navigate(mainChannel != null ? '/player' : '/channels', { replace: true }), [navigate, mainChannel]);
  const fullScreen = useCallback(
    async (slot: number) => {
      if (slot !== 0) await invoke('multiview_make_main', { slot }).catch(notifyError);
      exit();
    },
    [exit],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || pickerSlot != null || addOpen) return;
      const target = e.target as HTMLElement | null;
      if (target && ['INPUT', 'TEXTAREA'].includes(target.tagName)) return;
      if (/^[1-9]$/.test(e.key)) {
        const slot = order[Number(e.key) - 1];
        if (slot != null) focus(slot);
      } else if (e.key === 'a' || e.key === 'A') listen_(focused);
      else if (e.key === 'Enter' && focused !== 0) makeMain(focused);
      else if (e.key === 'f' || e.key === 'F') void fullScreen(focused);
      else if ((e.key === 'Delete' || e.key === 'Backspace') && focused !== 0) remove(focused);
      else if ((e.key === 'n' || e.key === 'N') && freeSlot != null) setAddOpen(true);
      else if (e.key === 'l' || e.key === 'L') setStyle(nextStyle(effectiveStyle, styles));
      else if (e.key === 'Escape') exit();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [order, focused, pickerSlot, addOpen, freeSlot, focus, listen_, makeMain, fullScreen, remove, exit, effectiveStyle, styles]);

  const picker = (slot: number) => (
    <ChannelPicker
      channels={channels.data ?? []}
      recents={home.data?.recents ?? []}
      onScreen={onScreen}
      budgetText={t('multiview.pickerBudget', { used: inUse, total: budget })}
      onPick={(c) => void setTile(slot, c.id)}
    />
  );

  return (
    <div className={classes.root}>
      <div className={classes.grid} style={{ gridTemplateColumns: spec.gridTemplateColumns, gridTemplateRows: spec.gridTemplateRows }}>
        {shown.map((slot, position) => {
          const tile = tiles.get(slot);
          const prog = tile ? nowOf(tile.channelId) : null;
          return (
            <Tile
              key={slot}
              slot={slot}
              position={position}
              area={spec.areas[position] ?? { gridColumn: 'auto', gridRow: 'auto' }}
              tile={tile}
              channel={tile ? byId.get(tile.channelId) : undefined}
              focused={focused === slot}
              audible={mv.audible === slot}
              pickerOpen={pickerSlot === slot}
              picker={picker(slot)}
              onPickerChange={(open) => setPickerSlot(open ? slot : null)}
              onFocus={() => focus(slot)}
              onListen={() => listen_(slot)}
              onMakeMain={() => makeMain(slot)}
              onFullScreen={() => void fullScreen(slot)}
              onRemove={() => remove(slot)}
              now={now}
              programme={prog}
            />
          );
        })}
      </div>
      <div className={`${classes.toolbar} ${classes.glass}`} role="toolbar" aria-label={t('player.multiview')}>
        {order.length > 0 && (
          <Popover opened={addOpen} onChange={setAddOpen} width={360} position="top" trapFocus withinPortal shadow="lg">
            <Popover.Target>
              <DLButton variant="default" size="sm" icon={IconPlus} keyHint="N" disabled={freeSlot == null} onClick={() => setAddOpen((o) => !o)}>
                {t('multiview.addChannel')}
              </DLButton>
            </Popover.Target>
            <Popover.Dropdown>{freeSlot != null && picker(freeSlot)}</Popover.Dropdown>
          </Popover>
        )}
        {styles.length > 1 && (
          <SegmentedControl
            size="xs"
            value={effectiveStyle}
            onChange={(v) => setStyle(v as LayoutStyle)}
            aria-label={t('multiview.layout')}
            data={styles.map((l) => ({
              value: l,
              label: (
                <>
                  <Icon icon={STYLE_ICON[l]} size={16} />
                  <VisuallyHidden>{t(`multiview.layout.${l}` as MessageKey)}</VisuallyHidden>
                </>
              ),
            }))}
          />
        )}
        <Switch size="sm" checked={audioFollows} onChange={(e) => setAudioFollows(e.currentTarget.checked)} label={t('multiview.audioFollows')} />
        <span className={classes.budget} aria-label={budgetText}>
          <span className={classes.pips} aria-hidden>
            {Array.from({ length: budget }, (_, i) => (
              <span key={i} className={classes.pip} data-on={i < inUse || undefined} />
            ))}
          </span>
          {budgetText}
        </span>
        <DLButton variant="default" size="sm" keyHint="Esc" onClick={exit}>
          {t('multiview.exit')}
        </DLButton>
      </div>
    </div>
  );
}
