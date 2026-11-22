import type { SessionStats } from '../../api/types';
import { t } from '../../lib/i18n';

/** The "RTX Video" info row, or null when neither feature is in use. */
export function rtxSummary(stats: SessionStats | null | undefined): string | null {
  if (!stats) return null;
  const parts: string[] = [];
  if (stats.rtxUpscale != null) parts.push(t('player.info.rtxUpscale', { n: Number(stats.rtxUpscale.toFixed(2)) }));
  if (stats.rtxHdrRequested) {
    parts.push(
      stats.hdr === true ? t('player.info.rtxHdrSkipped')
      : stats.rtxHdrActive === true ? t('player.info.rtxHdrOn')
      : stats.rtxHdrActive === false ? t('player.info.rtxHdrOff')
      : t('player.info.rtxHdrPending'),
    );
  }
  return parts.length ? parts.join(' · ') : null;
}
