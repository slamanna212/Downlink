import { useEffect, useState } from 'react';
import { IconCopy, IconMinus, IconSquare, IconX } from '@tabler/icons-react';
import { dl } from '../../theme';
import { t } from '../../lib/i18n';
import { isMac } from '../../lib/platform';
import { closeWindow, minimizeWindow, toggleMaximizeWindow, watchMaximized } from '../../lib/windowControls';
import classes from './TitleBar.module.css';

export interface TitleBarProps {
  /** Over video (player): lighter glyphs and a stronger hover. */
  onVideo?: boolean;
  /** Force the button set (Storybook); defaults to the running OS. */
  platform?: 'mac' | 'other';
  /** Multiview: the strip does not catch clicks (tile controls run to the top edge); buttons still work. */
  passive?: boolean;
  /** Takes its own row above the page (pages without a hero) instead of floating over the content. */
  inFlow?: boolean;
}

/**
 * The top 36 px of the content area: a drag region (double-click maximizes) with the window buttons
 * top-right on Windows and Linux. On macOS the native traffic lights sit over the rail instead, so only
 * the drag region is drawn. Content runs underneath; pages keep interactive controls out of this strip.
 */
export function TitleBar({ onVideo, platform, passive, inFlow }: TitleBarProps) {
  const mac = (platform ?? (isMac() ? 'mac' : 'other')) === 'mac';
  const [maximized, setMaximized] = useState(false);
  useEffect(() => (mac ? undefined : watchMaximized(setMaximized)), [mac]);
  return (
    <div
      className={classes.bar}
      data-tauri-drag-region={passive ? undefined : true}
      data-on-video={onVideo || undefined}
      data-passive={passive || undefined}
      data-in-flow={inFlow || undefined}
      style={{ height: dl.layout.titleBar }}
    >
      {!mac && (
        <div className={classes.buttons}>
          <button type="button" tabIndex={-1} className={classes.btn} aria-label={t('window.minimize')} title={t('window.minimize')} onClick={minimizeWindow}>
            <IconMinus size={16} stroke={dl.iconStroke} aria-hidden />
          </button>
          <button
            type="button"
            tabIndex={-1}
            className={classes.btn}
            aria-label={maximized ? t('window.restore') : t('window.maximize')}
            title={maximized ? t('window.restore') : t('window.maximize')}
            onClick={toggleMaximizeWindow}
          >
            {maximized ? <IconCopy size={14} stroke={dl.iconStroke} aria-hidden /> : <IconSquare size={13} stroke={dl.iconStroke} aria-hidden />}
          </button>
          <button type="button" tabIndex={-1} className={`${classes.btn} ${classes.close}`} aria-label={t('window.close')} title={t('window.close')} onClick={closeWindow}>
            <IconX size={16} stroke={dl.iconStroke} aria-hidden />
          </button>
        </div>
      )}
    </div>
  );
}
