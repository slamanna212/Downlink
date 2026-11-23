import type { NavigateFunction } from 'react-router';
import type { Channel, Programme } from '../../api/types';
import { nowEpoch } from '../../lib/time';
import { openInfo } from '../../store/info';
import { playCatchup, playChannel } from '../player/actions';
import { blockState, enterAction } from './guideModel';

/** Enter on a programme (PLAN §8.2): now -> live, past with archive -> catch-up, otherwise its details. */
export function activateProgramme(p: Programme, ch: Channel | undefined, navigate: NavigateFunction): void {
  const action = enterAction(blockState(p, ch, nowEpoch()));
  if (action === 'play-live') void playChannel(p.channelId).then((err) => (err ? undefined : navigate('/player')));
  else if (action === 'play-catchup') void playCatchup(p.id).then((err) => (err ? undefined : navigate('/player')));
  else openInfo({ kind: 'programme', programme: p, channel: ch });
}
