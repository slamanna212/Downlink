import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router';
import { useQueries } from '@tanstack/react-query';
import { IconListNumbers, IconPin, IconPinnedOff, IconPlus, IconReplace, IconVolume, IconX } from '@tabler/icons-react';
import { channelQuery } from '../../api/queries';
import type { SportsCatalog, SportsMatch } from '../../api/types';
import { DLButton } from '../../components/DLButton/DLButton';
import { DLIconButton } from '../../components/DLIconButton/DLIconButton';
import { t } from '../../lib/i18n';
import { hoursToSeconds, nowEpoch } from '../../lib/time';
import { openInfo } from '../../store/info';
import { dl } from '../../theme';
import { MultiviewScreen, type MultiviewApi } from '../multiview/MultiviewPage';
import { matchStart, onAirNow, sortMatches } from '../sports/follow';
import { matchTitle } from '../sports/SportsMatchCard';
import { useFollowedMatches } from '../sports/useFollowedMatches';
import { assign, liveCandidates, type AssignPlan, type ScreenTile } from './assign';
import { RailRow } from './RailRow';
import { RailToggleButton, Section, useRailToggle } from './RailParts';
import classes from './Command.module.css';

const HORIZON_HOURS = 48;
const UP_NEXT_LIMIT = 24;
/** An assignment that never shows up in the multiview state is given up on after this long (s). */
const INFLIGHT_S = 15;

/** What the user did on this visit; auto-fill works around it (COMMAND_CENTER §3). */
interface Intent {
  /** slot -> channel the user put there. */
  pins: Map<number, number>;
  /** Channels the user took off screen: never auto-added again this visit. */
  dismissed: Set<number>;
  /** Tiles the user removed, net of tiles they added: lowers how many tiles auto-fill keeps. */
  drop: number;
}

/**
 * Sports command center: every live game you follow on one multiview wall, best first, with a rail
 * listing what is on screen, what else is live and what is next. Opening it plays the games.
 */
export function SportsCommandPage() {
  const [railOpen, setRailOpen] = useRailToggle();
  const [intent, setIntent] = useState<Intent>(() => ({ pins: new Map(), dismissed: new Set(), drop: 0 }));
  const tilesRef = useRef<Map<number, unknown>>(new Map());

  const onManualSet = useCallback((slot: number, channelId: number) => {
    const added = !tilesRef.current.has(slot);
    setIntent((x) => ({ ...x, pins: new Map(x.pins).set(slot, channelId), drop: added ? Math.max(0, x.drop - 1) : x.drop }));
  }, []);
  const onManualRemove = useCallback((slot: number, channelId: number) => {
    setIntent((x) => {
      const pins = new Map(x.pins);
      pins.delete(slot);
      return { pins, dismissed: new Set(x.dismissed).add(channelId), drop: x.drop + 1 };
    });
  }, []);
  const togglePin = useCallback((slot: number, channelId: number) => {
    setIntent((x) => {
      const pins = new Map(x.pins);
      if (pins.get(slot) === channelId) pins.delete(slot);
      else pins.set(slot, channelId);
      return { ...x, pins };
    });
  }, []);

  return (
    <MultiviewScreen
      sideWidth={railOpen ? dl.layout.commandRail : 0}
      onManualSet={onManualSet}
      onManualRemove={onManualRemove}
      toolbarExtra={<RailToggleButton open={railOpen} onToggle={() => setRailOpen((o) => !o)} />}
    >
      {(api) => <SportsDriver api={api} intent={intent} setIntent={setIntent} togglePin={togglePin} railOpen={railOpen} tilesRef={tilesRef} />}
    </MultiviewScreen>
  );
}

interface DriverProps {
  api: MultiviewApi;
  intent: Intent;
  setIntent: React.Dispatch<React.SetStateAction<Intent>>;
  togglePin: (slot: number, channelId: number) => void;
  railOpen: boolean;
  tilesRef: React.RefObject<Map<number, unknown>>;
}

/** Keeps the wall filled (`assign`) and renders the rail. Lives inside the multiview screen for its state. */
function SportsDriver({ api, intent, setIntent, togglePin, railOpen, tilesRef }: DriverProps) {
  const { now, catalog, follow, following, matches } = useFollowedMatches(HORIZON_HOURS);
  const { tiles, maxTiles, loaded, budget, assignTile, remove, focused } = api;
  useEffect(() => {
    tilesRef.current = tiles;
  }, [tiles, tilesRef]);

  const leagueOrder = useMemo(() => new Map((catalog.data?.leagues ?? []).map((l, i) => [l.key, i])), [catalog.data]);
  const priority = follow.data?.priority;
  const candidates = useMemo(
    () => liveCandidates(matches.data ?? [], now, priority ?? [], leagueOrder),
    [matches.data, now, priority, leagueOrder],
  );

  // Assignments sent but not yet in the multiview state (slot -> channel, sent at, epoch seconds).
  // Read only in effects and handlers: a slot with one in flight is left alone until it lands, so a
  // slow `multiview://state` never makes a tile start twice or two channels race for one slot.
  const inflight = useRef(new Map<number, { channelId: number; at: number }>());
  const [failed, setFailed] = useState<Set<number>>(() => new Set());
  const screen: ScreenTile[] = useMemo(
    () => [...tiles].map(([slot, x]) => ({ slot, channelId: x.channelId, pinned: intent.pins.get(slot) === x.channelId, failed: x.state === 'Failed' })),
    [tiles, intent.pins],
  );

  const dismissed = useMemo(() => new Set([...intent.dismissed, ...failed]), [intent.dismissed, failed]);
  const plan: AssignPlan = useMemo(
    () => assign({ candidates, tiles: screen, maxTiles, capacity: Math.max(1, budget - intent.drop), dismissed }),
    [candidates, screen, maxTiles, budget, intent.drop, dismissed],
  );

  const send = useCallback(
    (slot: number, channelId: number) => {
      inflight.current.set(slot, { channelId, at: nowEpoch() });
      void assignTile(slot, channelId).then((ok) => {
        if (ok) return;
        inflight.current.delete(slot);
        setFailed((s) => new Set(s).add(channelId));
      });
    },
    [assignTile],
  );

  // Clear in-flight entries once the state shows them (or they time out).
  useEffect(() => {
    const at = nowEpoch();
    for (const [slot, f] of inflight.current) {
      if (tiles.get(slot)?.channelId === f.channelId || at - f.at > INFLIGHT_S) inflight.current.delete(slot);
    }
  }, [tiles, now]);

  // Apply the plan. Waits for matches and the real tile cap so an early empty list never stops tiles.
  const ready = following && matches.data != null && loaded;
  const planKey = ready ? JSON.stringify([plan.set, plan.remove]) : '';
  useEffect(() => {
    if (!planKey) return;
    for (const { slot, channelId } of plan.set) if (!inflight.current.has(slot)) send(slot, channelId);
    for (const slot of plan.remove) if (!inflight.current.has(slot)) remove(slot);
    // The key carries the plan; `send` and `remove` are stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [planKey]);

  // Swap / add from the rail: a manual choice, so the tile is pinned.
  const swap = useCallback(
    (m: SportsMatch) => {
      const slot = tiles.has(focused) ? focused : 0;
      setIntent((x) => ({ ...x, pins: new Map(x.pins).set(slot, m.channel.id) }));
      send(slot, m.channel.id);
    },
    [tiles, focused, send, setIntent],
  );
  const freeSlot = Array.from({ length: maxTiles }, (_, i) => i).find((s) => !tiles.has(s));
  const canAdd = freeSlot != null && tiles.size < budget;
  const add = useCallback(
    (m: SportsMatch) => {
      if (freeSlot == null) return;
      setIntent((x) => ({ ...x, pins: new Map(x.pins).set(freeSlot, m.channel.id), drop: Math.max(0, x.drop - 1), dismissed: without(x.dismissed, m.channel.id) }));
      send(freeSlot, m.channel.id);
    },
    [freeSlot, send, setIntent],
  );
  const removeTile = useCallback(
    (slot: number, channelId: number) => {
      setIntent((x) => {
        const pins = new Map(x.pins);
        pins.delete(slot);
        return { pins, dismissed: new Set(x.dismissed).add(channelId), drop: x.drop + 1 };
      });
      remove(slot);
    },
    [remove, setIntent],
  );

  const upNext = useMemo(() => {
    const horizon = now + hoursToSeconds(HORIZON_HOURS);
    const seen = new Set<string>();
    return sortMatches(matches.data ?? [], now).filter((m) => {
      if (onAirNow(m, now)) return false;
      const start = matchStart(m);
      if (start == null || start >= horizon) return false;
      const key = `${m.channel.id}:${m.programme?.id ?? ''}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    }).slice(0, UP_NEXT_LIMIT);
  }, [matches.data, now]);

  if (!railOpen) return null;
  return (
    <CommandRail
      api={api}
      catalog={catalog.data}
      now={now}
      following={following}
      plan={plan}
      upNext={upNext}
      pins={intent.pins}
      canAdd={canAdd}
      onTogglePin={togglePin}
      onRemove={removeTile}
      onSwap={swap}
      onAdd={add}
    />
  );
}

const without = (s: Set<number>, id: number) => {
  const next = new Set(s);
  next.delete(id);
  return next;
};

interface RailProps {
  api: MultiviewApi;
  catalog: SportsCatalog | undefined;
  now: number;
  following: boolean;
  plan: AssignPlan;
  upNext: SportsMatch[];
  pins: Map<number, number>;
  canAdd: boolean;
  onTogglePin: (slot: number, channelId: number) => void;
  onRemove: (slot: number, channelId: number) => void;
  onSwap: (m: SportsMatch) => void;
  onAdd: (m: SportsMatch) => void;
}

function CommandRail({ api, catalog, now, following, plan, upNext, pins, canAdd, onTogglePin, onRemove, onSwap, onAdd }: RailProps) {
  const navigate = useNavigate();
  const slots = [...api.tiles.keys()].sort((a, b) => a - b);
  const ids = slots.map((s) => api.tiles.get(s)?.channelId ?? -1);
  const channels = useQueries({ queries: [...new Set(ids)].filter((id) => id >= 0).map(channelQuery) });
  const byId = new Map(channels.flatMap(({ data }) => (data ? [[data.id, data] as const] : [])));
  const rank = () => void navigate('/settings/sports');

  return (
    <aside className={classes.rail} aria-labelledby="command-title">
      <div className={classes.railHead}>
        <h2 className={classes.railTitle} id="command-title">{t('command.sports.title')}</h2>
        {following && <DLButton size="xs" variant="subtle" icon={IconListNumbers} aria-label={t('command.rank.aria')} onClick={rank}>{t('command.rank')}</DLButton>}
      </div>
      <div className={classes.railBody}>
        {!following ? (
          <div className={classes.empty}>
            <span>{t('command.nothingFollowed')}</span>
            <DLButton size="sm" variant="filled" onClick={rank}>{t('command.chooseSports')}</DLButton>
          </div>
        ) : (
          <>
            <Section title={t('command.section.onScreen')} count={slots.length} empty={t('command.onScreen.empty')}>
              {slots.map((slot) => {
                const tile = api.tiles.get(slot);
                if (!tile) return null;
                const m = plan.onScreen.get(slot) ?? null;
                const channel = byId.get(tile.channelId);
                const pinned = pins.get(slot) === tile.channelId;
                const title = m ? matchTitle(m) : channel?.name ?? '';
                return (
                  <RailRow
                    key={slot}
                    match={m}
                    channel={channel}
                    catalog={catalog}
                    now={now}
                    active={api.focused === slot}
                    pinned={pinned}
                    label={t('command.row.focus', { title })}
                    onSelect={() => api.focus(slot)}
                    actions={
                      <>
                        {api.audible !== slot && <DLIconButton label={t('multiview.listen')} icon={IconVolume} size="sm" onClick={() => api.listen(slot)} />}
                        <DLIconButton
                          label={pinned ? t('command.unpin') : t('command.pin')}
                          icon={pinned ? IconPinnedOff : IconPin}
                          size="sm"
                          pressed={pinned}
                          onClick={() => onTogglePin(slot, tile.channelId)}
                        />
                        {slot !== 0 && <DLIconButton label={t('multiview.remove')} icon={IconX} variant="danger" size="sm" onClick={() => onRemove(slot, tile.channelId)} />}
                      </>
                    }
                  />
                );
              })}
            </Section>
            <Section title={t('command.section.alsoLive')} count={plan.overflow.length} empty={t('command.alsoLive.empty')}>
              {plan.overflow.map((m) => (
                <RailRow
                  key={m.channel.id}
                  match={m}
                  channel={m.channel}
                  catalog={catalog}
                  now={now}
                  label={t('command.row.swap', { title: matchTitle(m) })}
                  onSelect={() => onSwap(m)}
                  actions={
                    <>
                      <DLIconButton label={t('command.swap')} icon={IconReplace} size="sm" onClick={() => onSwap(m)} />
                      {canAdd && <DLIconButton label={t('command.add')} icon={IconPlus} size="sm" onClick={() => onAdd(m)} />}
                    </>
                  }
                />
              ))}
            </Section>
            <Section title={t('command.section.upNext')} count={upNext.length} empty={t('command.upNext.empty')}>
              {upNext.map((m) => (
                <RailRow
                  key={`${m.channel.id}:${m.programme?.id ?? m.league}`}
                  match={m}
                  channel={m.channel}
                  catalog={catalog}
                  now={now}
                  label={t('sports.card.details', { title: matchTitle(m), channel: m.channel.name })}
                  onSelect={() => m.programme && openInfo({ kind: 'programme', programme: m.programme, channel: m.channel })}
                />
              ))}
            </Section>
          </>
        )}
      </div>
    </aside>
  );
}
