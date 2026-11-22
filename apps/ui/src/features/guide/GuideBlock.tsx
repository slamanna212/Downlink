import { memo } from 'react';
import { IconHistory } from '@tabler/icons-react';
import type { Programme } from '../../api/types';
import { dl } from '../../theme';
import { t } from '../../lib/i18n';
import { formatRange } from '../../lib/time';
import { episodeCode } from '../../lib/episode';
import type { BlockState } from './guideModel';
import classes from './Guide.module.css';

export interface GuideBlockProps {
  p: Programme;
  row: number;
  state: BlockState;
  x: number;
  y: number;
  w: number;
  h: number;
  /** 0..1, only for `now` blocks. */
  pct: number;
  focused: boolean;
  tabbable: boolean;
  channelName?: string;
}

/** One programme block. Memoised: scrolling never re-renders blocks that stay on screen. */
export const GuideBlock = memo(function GuideBlock({ p, row, state, x, y, w, h, pct, focused, tabbable, channelName }: GuideBlockProps) {
  const code = episodeCode(p);
  return (
    <div
      className={classes.block}
      role="gridcell"
      aria-description={[channelName, p.subtitle].filter(Boolean).join(', ') || undefined}
      aria-disabled={state === 'past-none' || undefined}
      aria-selected={focused}
      tabIndex={tabbable ? 0 : -1}
      data-pid={p.id}
      data-row={row}
      data-state={state}
      data-focused={focused || undefined}
      style={{ left: x, top: y, width: w, height: h }}
    >
      <div className={classes.face}>
        <div className={classes.inner} style={{ maxWidth: Math.max(w - 8, 0) }}>
          <div className={classes.blockTitle}>
            {state === 'past-archive' && (
              <span className={classes.histIcon}>
                <IconHistory size={14} stroke={dl.iconStroke} aria-hidden />
              </span>
            )}
            <span className={classes.blockTitleText}>{p.title}</span>
            {p.isScheduled && <span className={classes.recDot} role="img" aria-label="Scheduled to record" />}
            {p.isPremiere && w > 260 && <span className={classes.newTag}>{t('badge.premiere')}</span>}
            {p.isFinale && w > 260 && <span className={classes.newTag}>{t('badge.finale')}</span>}
            {p.isNew && w > 220 && <span className={classes.newTag}>{t('badge.new')}</span>}
          </div>
          <div className={classes.blockTime}>
            {code && w > 150 && <span className={classes.blockCode}>{code}</span>}
            {formatRange(p.startUtc, p.endUtc)}
          </div>
        </div>
        {state === 'now' && <div className={classes.bar} style={{ width: `${pct * 100}%` }} />}
      </div>
    </div>
  );
});
