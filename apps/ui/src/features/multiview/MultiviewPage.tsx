import { useCallback, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router';
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import { useIdle } from '@mantine/hooks';
import { Checkbox, Popover, SegmentedControl, Switch, TextInput, VisuallyHidden } from '@mantine/core';
import {
  IconArrowsMaximize,
  IconGauge,
  IconLayoutGrid,
  IconLayoutSidebarRight,
  IconMaximize,
  IconMinimize,
  IconPlus,
  IconReplace,
  IconSquareRoundedArrowUp,
  IconVolume,
  IconVolumeOff,
  IconX,
} from '@tabler/icons-react';
import { inTauri, invoke, listen, toAppError } from '../../api/ipc';
import { channelQuery, useChannelPages, useHome, useNowNextWindow } from '../../api/queries';
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
import { useUiStore } from '../../store/ui';
import { dl } from '../../theme';
import { playChannel } from '../player/actions';
import { holdFullscreen, isFullscreenKey, releaseFullscreen, setFullscreen, toggleFullscreen } from '../player/fullscreen';
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

/** States in which a tile's mpv surface has a picture worth showing; otherwise the canvas covers it. */
const SHOWS_VIDEO = new Set(['Buffering', 'Playing', 'Paused']);

/**
 * The window is transparent so the native video surfaces under the webview show through. Anything
 * opaque in the webview covers them, so the canvas is one SVG path: the whole view minus a hole per
 * tile that has video (`data-hole`). Gaps, centred short rows and loading tiles get the canvas;
 * the desktop never shows through.
 */
function useCanvasPath(rootRef: React.RefObject<HTMLDivElement | null>, layoutKey: string) {
  const [d, setD] = useState('');
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const measure = () => {
      const r = root.getBoundingClientRect();
      let path = `M0 0H${r.width}V${r.height}H0Z`;
      root.querySelectorAll<HTMLElement>('[data-hole]').forEach((el) => {
        const b = el.getBoundingClientRect();
        path += `M${b.left - r.left} ${b.top - r.top}h${b.width}v${b.height}h${-b.width}Z`;
      });
      setD(path);
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(root);
    root.querySelectorAll('[data-hole]').forEach((el) => ro.observe(el));
    return () => ro.disconnect();
  }, [rootRef, layoutKey]);
  return d;
}

type PickerTab = 'favourites' | 'recent' | 'all';

/** Add-channel mode: a checkbox per row; the picker stays open while channels go in and out. */
interface PickerMulti {
  /** On screen or on the way. */
  checked: Set<number>;
  /** Checked but not removable here (the main player). */
  locked: Set<number>;
  /** No stream left for another channel. */
  full: boolean;
  onToggle: (c: Channel, on: boolean) => void;
}

/**
 * Channel picker: search plus Favourites / Recent / All. Single mode (replace a tile) picks one
 * channel and on-screen channels are disabled; multi mode (add channels) toggles checkboxes.
 */
function ChannelPicker({ onScreen, budgetText, onPick, multi }: {
  onScreen: Map<number, number>;
  budgetText: string;
  onPick?: (c: Channel) => void;
  multi?: PickerMulti;
}) {
  const [query, setQuery] = useState('');
  const deferred = useDeferredValue(query.trim());
  const [selected, setTab] = useState<PickerTab | null>(null);
  const { data: home } = useHome();
  const favourites = useChannelPages({ favouritesOnly: true }, 60);
  const hasFavourites = favourites.data?.pages.some((page) => page.some((c) => !c.isRadio));
  const tab = selected ?? (hasFavourites ? 'favourites' : home?.recents.length ? 'recent' : 'all');
  const all = useChannelPages({ query: deferred || null }, 60, !!deferred || tab === 'all');
  const page = deferred || tab === 'all' ? all : tab === 'favourites' ? favourites : null;
  const list = (page ? page.data?.pages.flat() ?? [] : home?.recents ?? []).filter((c) => !c.isRadio);
  const listIds = list.map((c) => c.id);
  const nn = useNowNextWindow(listIds, `multiview-picker|${listIds.join(',')}`);
  const onNow = new Map((nn.data ?? []).map((x) => [x.channelId, x.now?.title ?? null]));
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
      <div className={classes.pickerList} role="listbox" aria-label={t('multiview.pickChannel')} aria-multiselectable={multi ? true : undefined}>
        {list.length === 0 && <div className={classes.pickerEmpty}>{t(page?.isFetching ? 'button.loading' : 'multiview.noChannels')}</div>}
        {list.map((c) => {
          const slot = onScreen.get(c.id);
          const checked = multi?.checked.has(c.id) ?? false;
          const disabled = multi ? multi.locked.has(c.id) || (!checked && multi.full) : slot != null;
          return (
            <button
              key={c.id}
              type="button"
              role="option"
              aria-selected={checked}
              className={classes.pickerRow}
              disabled={disabled}
              onClick={() => (multi ? multi.onToggle(c, !checked) : onPick?.(c))}
            >
              <ChannelLogo channelId={c.id} name={c.name} src={c.logo} density="compact" bare />
              <span className={classes.pickerNumber}>{c.number ?? ''}</span>
              <span className={classes.pickerText}>
                <span className={classes.pickerName}>{c.name}</span>
                {onNow.get(c.id) && <span className={classes.pickerNow}>{onNow.get(c.id)}</span>}
              </span>
              {multi ? (
                <Checkbox.Indicator checked={checked} disabled={disabled} size="sm" aria-hidden />
              ) : (
                slot != null && <span className={classes.pickerHint}>{t('multiview.alreadyIn', { n: slot + 1 })}</span>
              )}
            </button>
          );
        })}
      </div>
      {page?.hasNextPage && (
        <DLButton size="xs" loading={page.isFetchingNextPage} onClick={() => void page.fetchNextPage()}>
          {t('button.loadMore')}
        </DLButton>
      )}
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
  /** The add-channel picker, offered by an empty slot. */
  addOpen: boolean;
  addPicker: React.ReactNode;
  onAddChange: (open: boolean) => void;
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
  const hole = !!p.tile && p.tile.failure == null && SHOWS_VIDEO.has(p.tile.state);
  return (
    <div
      ref={ref}
      className={classes.tile}
      style={p.area}
      data-focused={p.focused || undefined}
      data-audible={(p.audible && !!p.tile) || undefined}
      data-empty={!p.tile || undefined}
      data-failed={failed || undefined}
      data-hole={hole || undefined}
      data-picker={p.pickerOpen || undefined}
      onMouseDown={p.onFocus}
      aria-label={p.channel ? t('multiview.tileLabel', { n: p.position + 1, name: p.channel.name }) : t('multiview.emptySlot', { n: p.position + 1 })}
      role="group"
    >
      {!inTauri() && p.tile && !failed && <div className={classes.devVideo} aria-hidden>tile {p.position + 1}</div>}
      {!p.tile ? (
        <Popover opened={p.addOpen} onChange={p.onAddChange} width={360} position="bottom" trapFocus withinPortal shadow="lg">
          <div className={classes.empty}>
            <Popover.Target>
              <DLButton variant="default" icon={IconPlus} onClick={() => p.onAddChange(!p.addOpen)}>
                {t('multiview.addChannel')}
              </DLButton>
            </Popover.Target>
          </div>
          <Popover.Dropdown>{p.addPicker}</Popover.Dropdown>
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
              <ChannelLogo channelId={p.channel.id} name={p.channel.name} src={p.channel.logo} density="compact" bare />
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
  const fullscreen = useUiStore((s) => s.fullscreen);
  const now = useNowEpoch(15_000);

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
  const [toolbarHover, setToolbarHover] = useState(false);
  // Chrome (focus ring, audio badge, tile actions, toolbar) fades after a few seconds without input,
  // like the player's control bar; it stays while the pointer rests on the toolbar or a picker is open.
  const idle = useIdle(dl.controlsIdleMs, { events: ['mousemove', 'mousedown', 'keydown', 'touchstart', 'wheel'], initialState: false });
  const chromeHidden = idle && !toolbarHover && !addOpen && pickerSlot == null;
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Keep the session (main player) alive while here; every tile stops when leaving (rule 6). Window
  // fullscreen carries over to and from the player (see ../player/fullscreen).
  useEffect(() => {
    holdSession();
    holdFullscreen();
    return () => {
      releaseSession();
      releaseFullscreen();
      void invoke('multiview_exit', {}).catch(() => undefined);
    };
  }, []);

  // "Add to multiview" from a channel: the channel lands in the first free slot.
  const pendingAdd = (location.state as { add?: number } | null)?.add;
  const added = useRef(false);

  const ids = useMemo(() => [...tiles.values()].map((x) => x.channelId), [tiles]);
  const tileChannels = useQueries({ queries: [...new Set(ids)].map(channelQuery) });
  const byId = new Map(tileChannels.flatMap(({ data }) => data ? [[data.id, data] as const] : []));
  const nn = useNowNextWindow(ids, `multiview|${ids.join(',')}`);
  const nowOf = (id: number) => nn.data?.find((x) => x.channelId === id)?.now ?? null;

  const inUse = tiles.size;
  const budget = Math.min(mv.maxTiles, mv.streamLimit ?? mv.maxTiles);
  const onScreen = useMemo(() => new Map(order.map((slot, i) => [tiles.get(slot)?.channelId ?? -1, i])), [order, tiles]);
  // Channels checked in the add picker whose tile has not landed yet (channel id -> slot), so quick
  // clicks each get their own slot.
  const [pending, setPending] = useState<Map<number, number>>(() => new Map());
  const [pendingFor, setPendingFor] = useState(tiles);
  if (pendingFor !== tiles) {
    // A tile landed (or the state moved on): drop the channels that are now on screen.
    setPendingFor(tiles);
    const left = [...pending].filter(([id, slot]) => tiles.get(slot)?.channelId !== id);
    if (left.length !== pending.size) setPending(new Map(left));
  }
  const reserved = new Set([...tiles.keys(), ...pending.values()]);
  const freeSlot = Array.from({ length: mv.maxTiles }, (_, i) => i).find((s) => !reserved.has(s)) ?? null;

  const assignTile = useCallback(
    async (slot: number, channelId: number) => {
      if (slot === 0) return (await playChannel(channelId)) == null;
      try {
        await invoke('multiview_set_tile', { slot, channelId });
        return true;
      } catch (e) {
        notifyError(toAppError(e));
        return false;
      }
    },
    [],
  );
  const setTile = useCallback(
    (slot: number, channelId: number) => {
      setPickerSlot(null);
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
  const multi: PickerMulti = {
    checked: new Set([...onScreen.keys(), ...pending.keys()]),
    locked: new Set(mainChannel != null ? [mainChannel] : []),
    full: freeSlot == null || reserved.size >= budget,
    onToggle: (c, on) => {
      if (!on) {
        const slot = order.find((s) => tiles.get(s)?.channelId === c.id);
        if (slot != null && slot !== 0) remove(slot);
        return;
      }
      if (freeSlot == null) return;
      const slot = freeSlot;
      setPending((m) => new Map(m).set(c.id, slot));
      void (async () => {
        if (!(await assignTile(slot, c.id))) setPending((m) => {
          const next = new Map(m);
          next.delete(c.id);
          return next;
        });
      })();
    },
  };

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
      if (isFullscreenKey(e)) {
        e.preventDefault();
        toggleFullscreen();
        return;
      }
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
      else if (e.key === 'n' || e.key === 'N') setAddOpen(true);
      else if (e.key === 'l' || e.key === 'L') setStyle(nextStyle(effectiveStyle, styles));
      else if (e.key === 'Escape') {
        if (fullscreen) setFullscreen(false);
        else exit();
      } else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [order, focused, pickerSlot, addOpen, freeSlot, focus, listen_, makeMain, fullScreen, remove, exit, effectiveStyle, styles, fullscreen]);

  const pickerBudget = t('multiview.pickerBudget', { used: inUse, total: budget });
  const picker = (slot: number) => (
    <ChannelPicker onScreen={onScreen} budgetText={pickerBudget} onPick={(c) => void setTile(slot, c.id)} />
  );
  const addPicker = <ChannelPicker onScreen={onScreen} budgetText={pickerBudget} multi={multi} />;

  const layoutKey = [effectiveStyle, ...shown.map((slot) => {
    const x = tiles.get(slot);
    return `${slot}:${x && x.failure == null && SHOWS_VIDEO.has(x.state) ? 1 : 0}`;
  })].join('|');
  const canvas = useCanvasPath(rootRef, layoutKey);

  return (
    <div ref={rootRef} className={classes.root} data-idle={chromeHidden || undefined}>
      <svg className={classes.canvas} aria-hidden>
        <path d={canvas} fillRule="evenodd" />
      </svg>
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
              addOpen={addOpen && order.length === 0}
              addPicker={addPicker}
              onAddChange={setAddOpen}
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
      <div className={`${classes.toolbar} ${classes.glass}`} role="toolbar" aria-label={t('player.multiview')}
        onMouseEnter={() => setToolbarHover(true)} onMouseLeave={() => setToolbarHover(false)}>
        {order.length > 0 && (
          <Popover opened={addOpen} onChange={setAddOpen} width={360} position="top" trapFocus withinPortal shadow="lg">
            <Popover.Target>
              <DLButton variant="default" size="sm" icon={IconPlus} keyHint="N" onClick={() => setAddOpen((o) => !o)}>
                {t('multiview.addChannel')}
              </DLButton>
            </Popover.Target>
            <Popover.Dropdown>{addPicker}</Popover.Dropdown>
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
        <DLIconButton
          label={fullscreen ? t('player.exitFullscreen') : t('player.fullscreen')}
          icon={fullscreen ? IconMinimize : IconMaximize}
          variant="subtle"
          size="sm"
          onClick={toggleFullscreen}
        />
        <DLButton variant="default" size="sm" keyHint="Esc" onClick={exit}>
          {t('multiview.exit')}
        </DLButton>
      </div>
    </div>
  );
}
