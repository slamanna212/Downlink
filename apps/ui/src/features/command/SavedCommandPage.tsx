import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from 'react';
import { useNavigate, useParams } from 'react-router';
import { useQueries } from '@tanstack/react-query';
import { IconNews, IconNewsOff, IconPlus, IconReplace, IconVolume, IconX } from '@tabler/icons-react';
import { channelQuery, useCommandCenter, useFeedsWatch } from '../../api/queries';
import type { Channel, CommandCenter } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { t } from '../../lib/i18n';
import { nowEpoch } from '../../lib/time';
import { useNowEpoch } from '../../lib/useNow';
import { dl } from '../../theme';
import { MultiviewScreen, type MultiviewApi } from '../multiview/MultiviewPage';
import { CenterMenu } from './CenterMenu';
import { HeadlineTicker } from './feeds/Headlines';
import { useCenterRail } from './feeds/useCenterRail';
import { RailExtras, type RailExtrasProps } from './RailExtras';
import { RailToggleButton, Section, useRailToggle } from './RailParts';
import { RailRow } from './RailRow';
import { SaveCenterControls } from './SaveCenterControls';
import { planOpen } from './saved';
import classes from './Command.module.css';

/** Height of the headline ticker strip under the wall (px; 2.25rem). */
const TICKER_HEIGHT = 36;

/** An assignment that never shows up in the multiview state is given up on after this long (s). */
const INFLIGHT_S = 15;

export interface SavedCommandPageProps {
  /** Extra rail sections below the built-in ones (Phase 3: Headlines). Defaults to `RailExtras`. */
  extraSections?: ComponentType<RailExtrasProps>;
}

/**
 * A saved command center (`/multiview/c/:id`): opening it plays its channels, clipped to the stream
 * budget in slot order, with its layout and audible tile. What does not fit is listed in the rail as
 * "Not on screen". Changing tiles by hand offers "Save changes".
 */
export function SavedCommandPage({ extraSections = RailExtras }: SavedCommandPageProps) {
  const navigate = useNavigate();
  const { id: param } = useParams();
  const parsed = Number(param);
  const id = Number.isInteger(parsed) && parsed > 0 ? parsed : null;
  const center = useCommandCenter(id);
  const [railOpen, setRailOpen] = useRailToggle();
  const [edited, setEdited] = useState(false);
  const touch = useCallback(() => setEdited(true), []);
  // Channels of this center that are not on screen (did not fit, failed, or swapped out).
  const [overflow, setOverflow] = useState<number[]>([]);
  // A center just saved from this wall with "Save as new": the URL moves to it, the wall stays.
  const [adopted, setAdopted] = useState<number | null>(null);
  // News feeds (COMMAND_CENTER §7): polled while this center is open, rail shown or not.
  const feeds = useCenterRail(center.data);
  useFeedsWatch(feeds.feedIds);
  const showTicker = feeds.ticker;

  return (
    <MultiviewScreen
      sideWidth={railOpen ? dl.layout.commandRail : 0}
      bottomInset={showTicker ? TICKER_HEIGHT : 0}
      onManualSet={touch}
      onManualRemove={touch}
      onStyleChange={touch}
      toolbarExtra={(api) => (
        <>
          {center.data && (
            <SaveCenterControls
              api={api}
              center={{ id: center.data.id, name: center.data.name }}
              edited={edited}
              keep={overflow}
              rail={center.data.rail}
              onSaved={(c, asNew) => {
                setEdited(false);
                if (asNew) {
                  setAdopted(c.id);
                  void navigate(`/multiview/c/${c.id}`, { replace: true });
                }
              }}
            />
          )}
          {feeds.feedIds.length > 0 && (
            <DLIconButton
              label={showTicker ? t('feeds.ticker.hide') : t('feeds.ticker.show')}
              icon={showTicker ? IconNewsOff : IconNews}
              variant="subtle"
              size="sm"
              pressed={showTicker}
              disabled={feeds.saving}
              onClick={() => feeds.setTicker(!showTicker)}
            />
          )}
          <RailToggleButton open={railOpen} onToggle={() => setRailOpen((o) => !o)} />
        </>
      )}
    >
      {(api) => (
        <>
        {showTicker && <HeadlineTicker feedIds={feeds.feedIds} height={TICKER_HEIGHT} />}
        <SavedDriver
          api={api}
          center={center.data}
          missing={id == null || center.isError}
          overflow={overflow}
          setOverflow={setOverflow}
          adopted={adopted}
          onOpened={() => setEdited(false)}
          onEdited={touch}
          railOpen={railOpen}
          extraSections={extraSections}
        />
        </>
      )}
    </MultiviewScreen>
  );
}

interface DriverProps {
  api: MultiviewApi;
  center: CommandCenter | undefined;
  missing: boolean;
  overflow: number[];
  setOverflow: React.Dispatch<React.SetStateAction<number[]>>;
  adopted: number | null;
  onOpened: () => void;
  onEdited: () => void;
  railOpen: boolean;
  extraSections: ComponentType<RailExtrasProps>;
}

/** Puts the center on the wall once per opening and renders the rail. Lives inside the multiview screen for its state. */
function SavedDriver({ api, center, missing, overflow, setOverflow, adopted, onOpened, onEdited, railOpen, extraSections }: DriverProps) {
  const { tiles, loaded, budget, maxTiles, assignTile, remove, focused } = api;
  const now = useNowEpoch(15_000);
  // Assignments sent but not yet in the multiview state (slot -> channel, sent at, epoch seconds).
  const inflight = useRef(new Map<number, { channelId: number; at: number }>());
  const opened = useRef<number | null>(null);
  const pendingAudio = useRef<number | null>(null);

  const send = useCallback(
    (slot: number, channelId: number) => {
      const f = inflight.current.get(slot);
      if (f?.channelId === channelId) return;
      inflight.current.set(slot, { channelId, at: nowEpoch() });
      void assignTile(slot, channelId).then((ok) => {
        if (ok) return;
        inflight.current.delete(slot);
        // Failed to start: keep it in the rail so it can be tried again.
        setOverflow((o) => (o.includes(channelId) ? o : [...o, channelId]));
      });
    },
    [assignTile, setOverflow],
  );

  // Open: once per center, after the real tile cap is known. Navigating here is the play (rule 4).
  useEffect(() => {
    if (!center || !loaded || opened.current === center.id) return;
    opened.current = center.id;
    if (adopted === center.id) return;
    onOpened();
    api.setStyle(center.layoutStyle);
    const plan = planOpen(center.tiles.map((x) => ({ slot: x.slot, channelId: x.channel.id })), center.audible, budget);
    setOverflow(plan.overflow);
    const planned = new Set(plan.start.map((x) => x.slot));
    // Another center was on the wall (palette jump): its extra tiles stop.
    for (const slot of tiles.keys()) if (slot !== 0 && !planned.has(slot)) remove(slot);
    for (const x of plan.start) if (tiles.get(x.slot)?.channelId !== x.channelId) send(x.slot, x.channelId);
    pendingAudio.current = plan.start.length > 0 ? plan.audible : null;
    // Runs when the center or the cap arrives; the rest is read at that moment.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [center, loaded]);

  // Clear in-flight entries once the state shows them (or they time out); give the saved tile its sound.
  useEffect(() => {
    const at = nowEpoch();
    for (const [slot, f] of inflight.current) {
      if (tiles.get(slot)?.channelId === f.channelId || at - f.at > INFLIGHT_S) inflight.current.delete(slot);
    }
    const slot = pendingAudio.current;
    if (slot != null && tiles.has(slot) && !inflight.current.has(slot)) {
      pendingAudio.current = null;
      if (api.audible !== slot) api.listen(slot);
    }
  }, [tiles, now, api]);

  const canAdd = tiles.size < budget && tiles.size < maxTiles;

  const swap = useCallback(
    (channelId: number) => {
      const slot = tiles.has(focused) ? focused : 0;
      const out = tiles.get(slot)?.channelId;
      setOverflow((o) => {
        const next = o.filter((x) => x !== channelId);
        return out != null && out !== channelId ? [out, ...next] : next;
      });
      send(slot, channelId);
    },
    [tiles, focused, send, setOverflow],
  );
  const add = useCallback(
    (channelId: number) => {
      // Slots with a tile on the way count as taken.
      const reserved = new Set([...tiles.keys(), ...inflight.current.keys()]);
      const slot = Array.from({ length: maxTiles }, (_, i) => i).find((s) => !reserved.has(s));
      if (slot == null || reserved.size >= budget) return;
      setOverflow((o) => o.filter((x) => x !== channelId));
      send(slot, channelId);
    },
    [tiles, maxTiles, budget, send, setOverflow],
  );
  const removeTile = useCallback(
    (slot: number) => {
      onEdited();
      remove(slot);
    },
    [onEdited, remove],
  );

  if (!railOpen) return null;
  return (
    <SavedRail
      api={api}
      center={center}
      missing={missing}
      now={now}
      overflow={overflow}
      canAdd={canAdd}
      onSwap={swap}
      onAdd={add}
      onRemove={removeTile}
      extraSections={extraSections}
    />
  );
}

interface RailProps {
  api: MultiviewApi;
  center: CommandCenter | undefined;
  missing: boolean;
  now: number;
  overflow: number[];
  canAdd: boolean;
  onSwap: (channelId: number) => void;
  onAdd: (channelId: number) => void;
  onRemove: (slot: number) => void;
  extraSections: ComponentType<RailExtrasProps>;
}

function SavedRail({ api, center, missing, now, overflow, canAdd, onSwap, onAdd, onRemove, extraSections: Extra }: RailProps) {
  const navigate = useNavigate();
  const slots = [...api.tiles.keys()].sort((a, b) => a - b);
  const known = useMemo(() => new Map((center?.tiles ?? []).map((x) => [x.channel.id, x.channel] as const)), [center]);
  const wanted = [...new Set([...slots.map((s) => api.tiles.get(s)?.channelId ?? -1), ...overflow])].filter((id) => id >= 0 && !known.has(id));
  const fetched = useQueries({ queries: wanted.map(channelQuery) });
  const channelOf = (id: number): Channel | undefined => known.get(id) ?? fetched.find((q) => q.data?.id === id)?.data;

  return (
    <aside className={classes.rail} aria-labelledby="command-title">
      <div className={classes.railHead}>
        <h2 className={classes.railTitle} id="command-title">{center?.name ?? t('command.page.title')}</h2>
        {center && <CenterMenu center={center} />}
      </div>
      <div className={classes.railBody}>
        {missing ? (
          <div className={classes.railMissing}>
            <span>{t('command.saved.missing')}</span>
            <DLButton size="sm" variant="default" onClick={() => void navigate('/command')}>{t('command.saved.back')}</DLButton>
          </div>
        ) : center ? (
          <>
            <Section title={t('command.section.onScreen')} count={slots.length} empty={t('command.saved.onScreenEmpty')}>
              {slots.map((slot) => {
                const tile = api.tiles.get(slot);
                if (!tile) return null;
                const channel = channelOf(tile.channelId);
                return (
                  <RailRow
                    key={slot}
                    match={null}
                    channel={channel}
                    catalog={undefined}
                    now={now}
                    active={api.focused === slot}
                    label={t('command.row.focus', { title: channel?.name ?? '' })}
                    onSelect={() => api.focus(slot)}
                    actions={
                      <>
                        {api.audible !== slot && <DLIconButton label={t('multiview.listen')} icon={IconVolume} size="sm" onClick={() => api.listen(slot)} />}
                        {slot !== 0 && <DLIconButton label={t('multiview.remove')} icon={IconX} variant="danger" size="sm" onClick={() => onRemove(slot)} />}
                      </>
                    }
                  />
                );
              })}
            </Section>
            <Section title={t('command.section.notOnScreen')} count={overflow.length} empty={t('command.notOnScreen.empty')}>
              {overflow.map((id) => {
                const channel = channelOf(id);
                const name = channel?.name ?? '';
                return (
                  <RailRow
                    key={id}
                    match={null}
                    channel={channel}
                    catalog={undefined}
                    now={now}
                    label={t('command.row.swap', { title: name })}
                    onSelect={() => onSwap(id)}
                    actions={
                      <>
                        <DLIconButton label={t('command.swap')} icon={IconReplace} size="sm" onClick={() => onSwap(id)} />
                        {canAdd && <DLIconButton label={t('command.add')} icon={IconPlus} size="sm" onClick={() => onAdd(id)} />}
                      </>
                    }
                  />
                );
              })}
            </Section>
            <Extra center={center} api={api} />
          </>
        ) : null}
      </div>
    </aside>
  );
}
