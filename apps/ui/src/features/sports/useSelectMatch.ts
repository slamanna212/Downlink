import { useCallback } from 'react';
import { useNavigate } from 'react-router';
import type { Channel, SportsMatch } from '../../api/types';
import { nowEpoch } from '../../lib/time';
import { openInfo } from '../../store/info';
import { playChannel } from '../player/actions';
import { onAirNow } from './follow';

/**
 * What selecting a match does: on air (or an event channel with no times) plays the channel, the same
 * path as the home on-now cards; upcoming opens the programme details, as the guide and search do.
 */
export function useSelectMatch(): (m: SportsMatch) => void {
  const navigate = useNavigate();
  return useCallback(
    (m: SportsMatch) => {
      if (m.programme && !onAirNow(m, nowEpoch())) {
        openInfo({ kind: 'programme', programme: m.programme, channel: m.channel });
        return;
      }
      void playChannel(m.channel.id).then((err) => (err ? undefined : navigate('/player')));
    },
    [navigate],
  );
}

/** Plays one of the other channels showing a match (the card's "+N" menu). */
export function useSelectChannel(): (c: Channel) => void {
  const navigate = useNavigate();
  return useCallback(
    (c: Channel) => {
      void playChannel(c.id).then((err) => (err ? undefined : navigate('/player')));
    },
    [navigate],
  );
}
