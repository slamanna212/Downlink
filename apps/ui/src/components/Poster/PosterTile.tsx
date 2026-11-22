import { IconStarFilled } from '@tabler/icons-react';
import { t } from '../../lib/i18n';
import { Icon } from '../Icon/Icon';
import { Poster } from './Poster';
import { ratingLabel } from '../../features/vod/vod';
import classes from '../../features/vod/Vod.module.css';

export interface Tile {
  id: number;
  title: string;
  poster: string | null;
  sub: string | null;
  /** 0..1 saved progress, or null. */
  progress: number | null;
  watched: boolean;
  /** 0-10 score, shown as a corner badge; omitted or null hides it. */
  rating?: number | null;
}

export interface PosterTileProps {
  id?: string;
  tile: Tile;
  active: boolean;
  onOpen?: () => void;
  /** Storybook only: forces a pseudo-state on the tile. */
  demo?: Record<string, unknown>;
}

/** Grid cell: 2:3 poster (ring when active), title and sub line, progress bar and Watched tag. */
export function PosterTile({ id, tile, active, onOpen, demo }: PosterTileProps) {
  return (
    <div id={id} role="gridcell" aria-selected={active} className={classes.tile} data-active={active || undefined} onClick={onOpen} {...demo}>
      <div className={classes.posterRing}>
        <Poster src={tile.poster} title={tile.title}>
          {ratingLabel(tile.rating) && (
            <span className={classes.rating} aria-label={t('vod.rating', { value: ratingLabel(tile.rating) ?? '' })}>
              <Icon icon={IconStarFilled} size={14} filled />
              {ratingLabel(tile.rating)}
            </span>
          )}
          {tile.watched && <span className={classes.badge}>{t('vod.watched')}</span>}
          {tile.progress != null && (
            <span className={classes.track}>
              <span className={classes.fill} style={{ width: `${tile.progress * 100}%` }} />
            </span>
          )}
        </Poster>
      </div>
      <div className={classes.tileTitle}>{tile.title}</div>
      {tile.sub && <div className={classes.tileSub}>{tile.sub}</div>}
    </div>
  );
}
